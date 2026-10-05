//! Personal files are ordinary local files, independent of threads and agent sessions.
use std::{path::PathBuf, sync::Arc, time::SystemTime};

pub const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Directory,
    Text,
    Markdown,
    Image,
    Document,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative to the personal file space, never a traversal or an absolute path.
    pub path: PathBuf,
    pub name: String,
    pub kind: FileKind,
    pub stamp: FileStamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileListing {
    pub workspace: PathBuf,
    pub directory: PathBuf,
    pub entries: Vec<FileEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileLocation {
    /// Pin writes to the personal directory the user actually opened.
    pub workspace: PathBuf,
    pub path: PathBuf,
}

impl FileLocation {
    pub fn absolute_path(&self) -> PathBuf {
        self.workspace.join(&self.path)
    }
}

#[derive(Clone, Debug)]
pub enum FileContent {
    Text {
        text: String,
        markdown: bool,
    },
    Image {
        bytes: Arc<[u8]>,
        extension: String,
    },
    /// PDF and Office files may have a Quick Look thumbnail; full viewing uses the system app.
    Binary {
        preview: Option<PathBuf>,
    },
}

#[derive(Clone, Debug)]
pub struct FileDocument {
    pub location: FileLocation,
    pub stamp: FileStamp,
    /// Content hash for editable files. Metadata alone cannot protect against external writes.
    pub version: String,
    pub content: FileContent,
}

#[derive(Clone, Debug)]
pub enum FileCommand {
    Create {
        location: FileLocation,
        directory: bool,
        content: String,
    },
    Import {
        location: FileLocation,
        source: PathBuf,
    },
    Save {
        location: FileLocation,
        expected_version: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileError {
    Unavailable,
    InvalidPath,
    OutsideWorkspace,
    WorkspaceChanged,
    NotFound,
    AlreadyExists,
    Conflict,
    TooLarge,
    NotText,
    Io(String),
}

/// All methods can perform disk I/O. Call on a background executor, never from render.
pub trait FileService: Send + Sync {
    fn list(&self, directory: PathBuf) -> Result<FileListing, FileError>;
    fn read(&self, location: FileLocation) -> Result<FileDocument, FileError>;
    fn apply(&self, command: FileCommand) -> Result<FileLocation, FileError>;
    fn resolve_link(&self, link: &str) -> Result<FileLocation, FileError>;
}

pub struct EmptyFiles;
impl FileService for EmptyFiles {
    fn list(&self, _: PathBuf) -> Result<FileListing, FileError> {
        Err(FileError::Unavailable)
    }
    fn read(&self, _: FileLocation) -> Result<FileDocument, FileError> {
        Err(FileError::Unavailable)
    }
    fn apply(&self, _: FileCommand) -> Result<FileLocation, FileError> {
        Err(FileError::Unavailable)
    }
    fn resolve_link(&self, _: &str) -> Result<FileLocation, FileError> {
        Err(FileError::Unavailable)
    }
}
