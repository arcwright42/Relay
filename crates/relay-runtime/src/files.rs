//! One personal file space with conflict-checked writes. No project partitioning.
use relay_core::files::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

pub struct FileStore {
    root: PathBuf,
    initialized: Mutex<bool>,
    writes: Mutex<()>,
}

impl FileStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            initialized: Mutex::new(false),
            writes: Mutex::new(()),
        }
    }

    fn workspace(&self) -> Result<PathBuf, FileError> {
        let workspace = workspace_directory(&self.root);
        let mut initialized = self
            .initialized
            .lock()
            .expect("file space initialization lock");
        if !*initialized {
            fs::create_dir_all(&workspace).map_err(io_error)?;
            migrate_legacy_files(&self.root, &workspace).map_err(io_error)?;
            *initialized = true;
        }
        let workspace = workspace.canonicalize().map_err(io_error)?;
        if !workspace.is_dir() {
            return Err(FileError::InvalidPath);
        }
        Ok(workspace)
    }

    fn checked_path(
        &self,
        location: &FileLocation,
        must_exist: bool,
    ) -> Result<PathBuf, FileError> {
        let workspace = self.workspace()?;
        if workspace != location.workspace {
            return Err(FileError::WorkspaceChanged);
        }
        relative_path(&location.path)?;
        if location.path.as_os_str().is_empty() {
            return Err(FileError::InvalidPath);
        }
        let path = workspace.join(&location.path);
        let mut current = workspace.clone();
        for component in location.path.components() {
            if let Component::Normal(name) = component {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    // Do not edit or import through links, even links pointing inside the workspace.
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(FileError::OutsideWorkspace);
                    }
                    Ok(_) => {}
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound
                            && !must_exist
                            && current == path => {}
                    Err(error) => return Err(io_error(error)),
                }
            }
        }
        let parent = path
            .parent()
            .ok_or(FileError::InvalidPath)?
            .canonicalize()
            .map_err(io_error)?;
        if !parent.starts_with(&workspace) {
            return Err(FileError::OutsideWorkspace);
        }
        Ok(path)
    }

    fn thumbnail(&self, path: &Path, stamp: &FileStamp) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            use std::{
                process::{Command, Stdio},
                time::{Duration, Instant},
            };
            // A different disk version gets a different path, also invalidating GPUI's image cache.
            let key = hash(format!("{}|{:?}", path.display(), stamp).as_bytes());
            let directory = self.root.join("file-previews").join(key);
            let preview = directory.join(format!("{}.png", path.file_name()?.to_string_lossy()));
            if preview.is_file() {
                return Some(preview);
            }
            fs::create_dir_all(&directory).ok()?;
            let mut child = Command::new("/usr/bin/qlmanage")
                .args(["-t", "-s", "1200", "-o"])
                .arg(&directory)
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let started = Instant::now();
            loop {
                if child.try_wait().ok()?.is_some() {
                    break;
                }
                if started.elapsed() > Duration::from_secs(10) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            preview.is_file().then_some(preview)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, stamp);
            None
        }
    }
}

impl FileService for FileStore {
    fn list(&self, directory: PathBuf) -> Result<FileListing, FileError> {
        relative_path(&directory)?;
        let workspace = self.workspace()?;
        let path = if directory.as_os_str().is_empty() {
            workspace.clone()
        } else {
            self.checked_path(
                &FileLocation {
                    workspace: workspace.clone(),
                    path: directory.clone(),
                },
                true,
            )?
        };
        let mut entries = Vec::new();
        let mut truncated = false;
        for entry in fs::read_dir(path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            // Hide internal staging files and unsupported symlinks. Ordinary hidden files remain accessible.
            if name.to_string_lossy().starts_with(".relay-save-") {
                continue;
            }
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            if metadata.file_type().is_symlink() || !(metadata.is_dir() || metadata.is_file()) {
                continue;
            }
            if entries.len() == 5_000 {
                truncated = true;
                break;
            }
            entries.push(FileEntry {
                path: directory.join(&name),
                name: name.to_string_lossy().into_owned(),
                kind: if metadata.is_dir() {
                    FileKind::Directory
                } else {
                    kind(&entry.path())
                },
                stamp: stamp(&metadata),
            });
        }
        entries.sort_by(|a, b| {
            (
                a.kind != FileKind::Directory,
                a.name.to_lowercase(),
                &a.name,
            )
                .cmp(&(
                    b.kind != FileKind::Directory,
                    b.name.to_lowercase(),
                    &b.name,
                ))
        });
        Ok(FileListing {
            workspace,
            directory,
            entries,
            truncated,
        })
    }

    fn read(&self, location: FileLocation) -> Result<FileDocument, FileError> {
        let path = self.checked_path(&location, true)?;
        let metadata = fs::metadata(&path).map_err(io_error)?;
        if !metadata.is_file() {
            return Err(FileError::InvalidPath);
        }
        let file_stamp = stamp(&metadata);
        let file_kind = kind(&path);
        let (version, content) =
            if file_kind == FileKind::Image && metadata.len() <= MAX_IMAGE_BYTES {
                let bytes = read_bounded(&path, MAX_IMAGE_BYTES)?;
                (
                    hash(&bytes),
                    FileContent::Image {
                        bytes: bytes.into(),
                        extension: extension(&path),
                    },
                )
            } else if file_kind != FileKind::Document && metadata.len() <= MAX_EDIT_BYTES {
                let bytes = read_bounded(&path, MAX_EDIT_BYTES)?;
                let version = hash(&bytes);
                let content = match String::from_utf8(bytes) {
                    Ok(text) if is_text(&text) => FileContent::Text {
                        text,
                        markdown: file_kind == FileKind::Markdown,
                    },
                    _ => FileContent::Binary { preview: None },
                };
                (version, content)
            } else {
                let preview = (file_kind == FileKind::Document)
                    .then(|| self.thumbnail(&path, &file_stamp))
                    .flatten();
                (format!("{:?}", file_stamp), FileContent::Binary { preview })
            };
        Ok(FileDocument {
            location,
            stamp: file_stamp,
            version,
            content,
        })
    }

    fn apply(&self, command: FileCommand) -> Result<FileLocation, FileError> {
        let _lock = self.writes.lock().expect("file writes lock");
        match command {
            FileCommand::Create {
                location,
                directory,
                content,
            } => {
                if content.len() as u64 > MAX_EDIT_BYTES {
                    return Err(FileError::TooLarge);
                }
                let path = self.checked_path(&location, false)?;
                if directory {
                    fs::create_dir(path).map_err(io_error)?;
                } else {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(io_error)?;
                    if let Err(error) = file
                        .write_all(content.as_bytes())
                        .and_then(|_| file.sync_all())
                    {
                        let _ = fs::remove_file(path);
                        return Err(io_error(error));
                    }
                }
                Ok(location)
            }
            FileCommand::Import { location, source } => {
                let target = self.checked_path(&location, false)?;
                let mut source = fs::File::open(source).map_err(io_error)?;
                if !source.metadata().map_err(io_error)?.is_file() {
                    return Err(FileError::InvalidPath);
                }
                // create_new preserves an existing user file even when the picker selects a duplicate name.
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(io_error)?;
                let result = std::io::copy(&mut source, &mut file).and_then(|_| file.sync_all());
                if let Err(error) = result {
                    let _ = fs::remove_file(target);
                    return Err(io_error(error));
                }
                Ok(location)
            }
            FileCommand::Save {
                location,
                expected_version,
                content,
            } => {
                if content.len() as u64 > MAX_EDIT_BYTES {
                    return Err(FileError::TooLarge);
                }
                if !is_text(&content) {
                    return Err(FileError::NotText);
                }
                let path = self.checked_path(&location, true)?;
                let old = read_bounded(&path, MAX_EDIT_BYTES)?;
                if hash(&old) != expected_version {
                    return Err(FileError::Conflict);
                }
                if !String::from_utf8(old).is_ok_and(|text| is_text(&text)) {
                    return Err(FileError::NotText);
                }
                let staged = path
                    .parent()
                    .ok_or(FileError::InvalidPath)?
                    .join(format!(".relay-save-{}", crate::installer::unique_id()));
                let result = (|| {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&staged)
                        .map_err(io_error)?;
                    file.set_permissions(fs::metadata(&path).map_err(io_error)?.permissions())
                        .map_err(io_error)?;
                    file.write_all(content.as_bytes()).map_err(io_error)?;
                    file.sync_all().map_err(io_error)?;
                    // Check again after preparing the new file; never overwrite a detected Agent edit.
                    let checked = self.checked_path(&location, true)?;
                    if hash(&read_bounded(&checked, MAX_EDIT_BYTES)?) != expected_version {
                        return Err(FileError::Conflict);
                    }
                    fs::rename(&staged, checked).map_err(io_error)?;
                    Ok(location)
                })();
                if result.is_err() {
                    let _ = fs::remove_file(staged);
                }
                result
            }
        }
    }

    fn resolve_link(&self, link: &str) -> Result<FileLocation, FileError> {
        let workspace = self.workspace()?;
        let link = link
            .strip_prefix("file://")
            .or_else(|| link.strip_prefix("sandbox:"))
            .unwrap_or(link);
        let link = percent_decode(link.split('#').next().unwrap_or(link))?;
        // Agent code citations may carry :line or :line:column after the file name.
        let mut path = link.as_str();
        for _ in 0..2 {
            if let Some((prefix, suffix)) = path.rsplit_once(':')
                && !suffix.is_empty()
                && suffix.chars().all(|c| c.is_ascii_digit())
            {
                path = prefix;
            }
        }
        let path = PathBuf::from(path);
        let relative = if path.is_absolute() {
            path.strip_prefix(&workspace)
                .map_err(|_| FileError::OutsideWorkspace)?
                .to_path_buf()
        } else {
            path
        };
        let mut normalized = PathBuf::new();
        for part in relative.components() {
            match part {
                Component::Normal(name) => normalized.push(name),
                Component::CurDir => {}
                Component::ParentDir if normalized.pop() => {}
                _ => return Err(FileError::OutsideWorkspace),
            }
        }
        let location = FileLocation {
            workspace,
            path: normalized,
        };
        self.checked_path(&location, true)?;
        Ok(location)
    }
}

pub(crate) fn workspace_directory(root: &Path) -> PathBuf {
    let directory = root.join("files");
    directory.canonicalize().unwrap_or_else(|_| {
        root.canonicalize()
            .unwrap_or_else(|_| root.to_path_buf())
            .join("files")
    })
}

// Bring Relay-owned legacy files into the single personal space. Keep the
// originals, merge folders, and give different same-named files distinct names.
fn migrate_legacy_files(root: &Path, workspace: &Path) -> std::io::Result<()> {
    let marker = root.join(".personal-files-v1");
    if marker.is_file() {
        return Ok(());
    }
    let projects = root.join("projects");
    if projects.is_dir() {
        let mut entries = fs::read_dir(projects)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let source = entry.path().join("workspace");
            if fs::symlink_metadata(&source).is_ok_and(|m| m.file_type().is_dir()) {
                merge_legacy_directory(&source, workspace)?;
            }
        }
    }
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(marker)?
        .sync_all()
}

fn merge_legacy_directory(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        if !(kind.is_dir() || kind.is_file())
            || entry
                .file_name()
                .to_string_lossy()
                .starts_with(".relay-save-")
        {
            continue;
        }
        let mut suffix = 1;
        loop {
            let name = if suffix == 1 {
                entry.file_name()
            } else {
                let original = entry.path();
                let mut name = original.file_stem().unwrap_or_default().to_os_string();
                name.push(format!(" ({suffix})"));
                if let Some(extension) = original.extension() {
                    name.push(".");
                    name.push(extension);
                }
                name
            };
            let destination = target.join(name);
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if kind.is_dir() && metadata.file_type().is_dir() => {
                    merge_legacy_directory(&entry.path(), &destination)?;
                    break;
                }
                Ok(metadata)
                    if kind.is_file()
                        && metadata.file_type().is_file()
                        && hash_file(&entry.path())? == hash_file(&destination)? =>
                {
                    break;
                }
                Ok(_) => suffix += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if kind.is_dir() {
                        fs::create_dir(&destination)?;
                        merge_legacy_directory(&entry.path(), &destination)?;
                    } else {
                        let mut source = fs::File::open(entry.path())?;
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&destination)?;
                        if let Err(error) =
                            std::io::copy(&mut source, &mut file).and_then(|_| file.sync_all())
                        {
                            let _ = fs::remove_file(&destination);
                            return Err(error);
                        }
                    }
                    break;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(digest.finalize().to_vec());
        }
        digest.update(&buffer[..read]);
    }
}

fn relative_path(path: &Path) -> Result<(), FileError> {
    if path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        Ok(())
    } else {
        Err(FileError::InvalidPath)
    }
}
fn io_error(error: std::io::Error) -> FileError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FileError::NotFound,
        std::io::ErrorKind::AlreadyExists => FileError::AlreadyExists,
        _ => FileError::Io(error.to_string()),
    }
}
fn stamp(metadata: &fs::Metadata) -> FileStamp {
    FileStamp {
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
    }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, FileError> {
    let file = fs::File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(FileError::InvalidPath);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(FileError::TooLarge);
    }
    Ok(bytes)
}
fn is_text(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c == '\0' || c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}'))
}
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
fn kind(path: &Path) -> FileKind {
    match extension(path).as_str() {
        "md" | "markdown" | "mdown" => FileKind::Markdown,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico" => FileKind::Image,
        "pdf" | "doc" | "docx" | "ppt" | "pptx" | "xls" | "xlsx" | "odt" | "ods" | "odp" => {
            FileKind::Document
        }
        "txt" | "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "json" | "jsonl" | "yaml" | "yml"
        | "toml" | "csv" | "tsv" | "html" | "htm" | "svg" | "css" | "sh" | "xml" | "log" | "go"
        | "c" | "cpp" | "h" | "java" | "sql" | "tex" => FileKind::Text,
        _ => FileKind::Other,
    }
}
fn percent_decode(link: &str) -> Result<String, FileError> {
    let mut decoded = Vec::new();
    let mut bytes = link.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(FileError::InvalidPath)?;
            let low = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(FileError::InvalidPath)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).map_err(|_| FileError::InvalidPath)
}

#[cfg(test)]
mod tests;
