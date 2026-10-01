use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

// Compaction records can contain large context snapshots even though we do not archive them.
const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileStamp {
    pub(super) bytes: u64,
    modified_ns: Option<u64>,
    identity: Option<(u64, u64)>,
}

impl FileStamp {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)?;
        ensure!(metadata.is_file(), "Client session source is not a file");
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.dev(), metadata.ino()))
        };
        #[cfg(not(unix))]
        let identity = None;
        Ok(Self {
            bytes: metadata.len(),
            modified_ns: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|t| t.as_nanos().try_into().ok()),
            identity,
        })
    }
}

pub(super) fn sources(home: &Path, stopped: &AtomicBool) -> Result<Vec<PathBuf>> {
    fn visit(
        path: &Path,
        depth: usize,
        found: &mut Vec<PathBuf>,
        stopped: &AtomicBool,
    ) -> Result<()> {
        if stopped.load(Ordering::Acquire) {
            bail!("Local session sync stopped");
        }
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() && depth < 8 {
                visit(&entry.path(), depth + 1, found, stopped)?;
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl") {
                found.push(entry.path());
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(&home.join("sessions"), 0, &mut found, stopped)?;
    found.sort();
    let current_count = found.len();
    visit(&home.join("archived_sessions"), 0, &mut found, stopped)?;
    found[current_count..].sort();
    Ok(found)
}

fn line(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> Result<usize> {
    buffer.clear();
    let size = reader
        .take(MAX_RECORD_BYTES + 1)
        .read_until(b'\n', buffer)?;
    ensure!(
        size as u64 <= MAX_RECORD_BYTES,
        "Client session record exceeds 64 MB"
    );
    Ok(size)
}

pub(super) fn metadata(source: &Path, stamp: FileStamp) -> Result<SavedSession> {
    let mut reader = BufReader::new(fs::File::open(source)?);
    let mut buffer = Vec::new();
    line(&mut reader, &mut buffer)?;
    ensure!(
        buffer.ends_with(b"\n"),
        "Client session header is incomplete"
    );
    let value: Value = serde_json::from_slice(&buffer)?;
    ensure!(
        value["type"] == "session_meta",
        "Unsupported Codex session header"
    );
    let payload = &value["payload"];
    let id = payload["id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .context("Missing native session ID")?;
    let cwd = payload["cwd"]
        .as_str()
        .context("Missing native working directory")?;
    ensure!(
        Path::new(cwd).is_absolute(),
        "Native working directory is not absolute"
    );
    Ok(SavedSession {
        client: "codex".into(),
        native_id: id.into(),
        title: id.into(),
        working_directory: cwd.into(),
        source: source.into(),
        project: None,
        updated_at: value["timestamp"].as_str().unwrap_or("").into(),
        message_count: 0,
        available: true,
        stamp,
    })
}

fn prefix(file: &mut fs::File, length: u64, stopped: &AtomicBool) -> Result<Sha256> {
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut remaining = length;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        if stopped.load(Ordering::Acquire) {
            bail!("Local session sync stopped");
        }
        let amount = remaining.min(buffer.len() as u64) as usize;
        let count = file.read(&mut buffer[..amount])?;
        ensure!(count > 0, "Native source changed while reading");
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(digest)
}

fn visible_message(value: &Value, id: u64) -> Option<SavedMessage> {
    if value["type"] != "response_item" {
        return None;
    }
    let item = &value["payload"];
    if item["type"] != "message" {
        return None;
    }
    let role = item["role"].as_str()?;
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    if role == "assistant"
        && item["phase"]
            .as_str()
            .is_some_and(|p| !matches!(p, "final" | "final_answer" | "commentary"))
    {
        return None;
    }
    let text = item["content"]
        .as_array()?
        .iter()
        .filter_map(|part| {
            matches!(
                part["type"].as_str(),
                Some("input_text" | "output_text" | "text")
            )
            .then(|| part["text"].as_str())
            .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = text.trim();
    // Codex stores injected environment and repository instructions as user-role items.
    if trimmed.is_empty()
        || (role == "user"
            && [
                "<environment_context>",
                "<permissions instructions>",
                "# AGENTS.md instructions",
            ]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix)))
    {
        return None;
    }
    Some(SavedMessage {
        id,
        role: role.into(),
        text,
    })
}

pub(super) fn read(
    source: &Path,
    metadata: SavedSession,
    previous: Option<Archive>,
    stopped: &AtomicBool,
) -> Result<(Archive, bool)> {
    let mut file = fs::File::open(source)?;
    let mut digest = Sha256::new();
    let mut archive = Archive {
        version: 1,
        session: metadata.clone(),
        offset: 0,
        prefix_sha256: String::new(),
        messages: Vec::new(),
    };
    if let Some(previous) = previous
        && previous.session.source == metadata.source
        && previous.offset <= metadata.stamp.bytes
    {
        digest = prefix(&mut file, previous.offset, stopped)?;
        if crate::installer::digest_hex(digest.clone().finalize().as_ref())
            == previous.prefix_sha256
        {
            archive = previous;
            archive.session.source = metadata.source.clone();
            archive.session.stamp = metadata.stamp.clone();
            archive.session.available = true;
        } else {
            digest = Sha256::new();
        }
    }
    let start = archive.offset;
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::new(file.take(metadata.stamp.bytes - start));
    let mut buffer = Vec::new();
    let mut records = 0;
    while line(&mut reader, &mut buffer)? > 0 {
        if stopped.load(Ordering::Acquire) {
            bail!("Local session sync stopped");
        }
        // Unfinished lines remain pending for the next sync. Never advance their cursor.
        if !buffer.ends_with(b"\n") {
            break;
        }
        let value: Value = serde_json::from_slice(&buffer)
            .context("Invalid complete native session record; previous snapshot preserved")?;
        if let Some(message) = visible_message(&value, archive.offset + 1) {
            if archive.session.title == archive.session.native_id && message.role == "user" {
                archive.session.title = message
                    .text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(90)
                    .collect();
            }
            archive.messages.push(message);
        }
        if let Some(time) = value["timestamp"].as_str() {
            archive.session.updated_at = time.into();
        }
        digest.update(&buffer);
        archive.offset += buffer.len() as u64;
        records += 1;
    }
    let after = FileStamp::read(source)?;
    ensure!(
        after.identity == metadata.stamp.identity
            && after.bytes >= metadata.stamp.bytes
            && (after.bytes > metadata.stamp.bytes
                || after.modified_ns == metadata.stamp.modified_ns),
        "Native source changed while reading; previous snapshot preserved"
    );
    archive.prefix_sha256 = crate::installer::digest_hex(digest.finalize().as_ref());
    archive.session.message_count = archive.messages.len();
    // An unchanged transcript still records the latest source stamp, so future syncs skip I/O.
    Ok((
        archive,
        records > 0 || start == 0 || metadata.stamp.bytes > start,
    ))
}
