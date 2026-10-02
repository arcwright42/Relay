mod browser;
mod editor;
mod new_entry;
#[cfg(test)]
mod tests;

use super::*;
use editor::FileEditor;
use relay_core::files::*;
use std::{collections::BTreeMap, path::PathBuf};

actions!(relay, [SaveFile]);

impl Workbench {
    pub fn set_file_service(&mut self, service: Arc<dyn FileService>, cx: &mut Context<Self>) {
        self.files.update(cx, |files, _| files.set_service(service));
    }

    pub(super) fn open_file_link(
        &mut self,
        link: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if link.starts_with("https://")
            || link.starts_with("http://")
            || link.starts_with("mailto:")
        {
            cx.open_url(&link);
            return;
        }
        self.navigate(Page::Files, window, cx);
        self.files
            .update(cx, |files, cx| files.open_link(link, window, cx));
    }
}

#[derive(Clone)]
pub(super) struct OpenFileLink {
    pub link: String,
}
impl EventEmitter<OpenFileLink> for FileEditor {}
pub(super) struct CloseFile;
impl EventEmitter<CloseFile> for FileEditor {}

pub(super) struct FilesView {
    service: Arc<dyn FileService>,
    language: Language,
    directory: PathBuf,
    listing: Option<FileListing>,
    active: Option<FileLocation>,
    editors: BTreeMap<FileLocation, Entity<FileEditor>>,
    search: Entity<InputState>,
    loading: bool,
    opening: bool,
    open_request: u64,
    mutating: bool,
    error: Option<FileError>,
    generation: u64,
    visible: bool,
    _updates: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl FilesView {
    pub(super) fn new(language: Language, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(language.text(Text::FilesSearch)));
        let subscription = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let executor = cx.background_executor().clone();
        let updates = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_secs(2)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        if this.visible {
                            this.refresh(window, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            service: Arc::new(EmptyFiles),
            language,
            directory: PathBuf::new(),
            listing: None,
            active: None,
            editors: BTreeMap::new(),
            search,
            loading: false,
            opening: false,
            open_request: 0,
            mutating: false,
            error: None,
            generation: 0,
            visible: false,
            _updates: updates,
            _subscriptions: vec![subscription],
        }
    }

    pub(super) fn set_service(&mut self, service: Arc<dyn FileService>) {
        self.service = service;
    }

    pub(super) fn set_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.language = language;
        self.search.update(cx, |search, cx| {
            search.set_placeholder(language.text(Text::FilesSearch), window, cx)
        });
        for editor in self.editors.values() {
            editor.update(cx, |editor, cx| {
                editor.language = language;
                cx.notify();
            });
        }
        cx.notify();
    }

    pub(super) fn set_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible = visible;
        if visible {
            self.refresh(window, cx);
        }
    }

    fn reset_directory(&mut self, directory: PathBuf) {
        self.generation += 1;
        self.directory = directory;
        self.active = None;
        self.listing = None;
        self.loading = false;
        self.opening = false;
        self.error = None;
    }

    fn change_directory(
        &mut self,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_directory(directory);
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.refresh(window, cx);
        cx.notify();
    }

    fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = None;
        self.opening = false;
        self.open_request += 1;
        self.error = None;
        self.refresh(window, cx);
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.mutating {
            return;
        }
        self.loading = true;
        let generation = self.generation;
        let service = self.service.clone();
        let directory = self.directory.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.list(directory) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(listing) => {
                        let workspace_changed = this
                            .listing
                            .as_ref()
                            .is_some_and(|old| old.workspace != listing.workspace);
                        if workspace_changed {
                            this.active = None;
                        }
                        let reload =
                            this.active
                                .as_ref()
                                .filter(|_| !this.opening)
                                .and_then(|location| {
                                    let Some(editor) = this.editors.get(location) else {
                                        return Some(location.clone());
                                    };
                                    let editor = editor.read(cx);
                                    let entry = listing
                                        .entries
                                        .iter()
                                        .find(|entry| entry.path == location.path);
                                    (entry.is_none_or(|entry| entry.stamp != editor.document.stamp))
                                        .then(|| location.clone())
                                });
                        if this.listing.as_ref() != Some(&listing) {
                            this.listing = Some(listing);
                            cx.notify();
                        }
                        if matches!(
                            this.error,
                            Some(FileError::Unavailable | FileError::NotFound | FileError::Io(_))
                        ) {
                            this.error = None;
                            cx.notify();
                        }
                        if let Some(location) = reload {
                            this.open_file(location, window, cx);
                        }
                    }
                    Err(error) => {
                        if this.error.as_ref() != Some(&error) {
                            this.error = Some(error);
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
        // The first load has a visible placeholder; background refreshes don't flicker the list.
        if self.listing.is_none() {
            cx.notify();
        }
    }

    fn select_entry(&mut self, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.kind == FileKind::Directory {
            self.change_directory(entry.path.clone(), window, cx);
            return;
        }
        let Some(listing) = &self.listing else {
            return;
        };
        let location = FileLocation {
            workspace: listing.workspace.clone(),
            path: entry.path.clone(),
        };
        self.active = Some(location.clone());
        self.error = None;
        self.open_file(location, window, cx);
        cx.notify();
    }

    fn open_file(&mut self, location: FileLocation, window: &mut Window, cx: &mut Context<Self>) {
        self.open_request += 1;
        let request = self.open_request;
        self.opening = true;
        let generation = self.generation;
        let service = self.service.clone();
        let requested = location.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.read(requested) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.open_request != request {
                    return;
                }
                this.opening = false;
                if this.active.as_ref() != Some(&location) {
                    return;
                }
                match result {
                    Ok(document) => {
                        if let Some(editor) = this.editors.get(&location) {
                            editor.update(cx, |editor, cx| editor.sync_disk(document, window, cx));
                        } else {
                            let service = this.service.clone();
                            let language = this.language;
                            let editor = cx
                                .new(|cx| FileEditor::new(document, service, language, window, cx));
                            this._subscriptions.push(cx.subscribe_in(
                                &editor,
                                window,
                                |this, _, event: &OpenFileLink, window, cx| {
                                    this.open_link(event.link.clone(), window, cx);
                                },
                            ));
                            this._subscriptions.push(cx.subscribe_in(
                                &editor,
                                window,
                                |this, _, _: &CloseFile, window, cx| {
                                    this.close_active(window, cx);
                                },
                            ));
                            this.editors.insert(location, editor);
                        }
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_link(&mut self, link: String, window: &mut Window, cx: &mut Context<Self>) {
        if link.starts_with("https://")
            || link.starts_with("http://")
            || link.starts_with("mailto:")
        {
            cx.open_url(&link);
            return;
        }
        self.open_request += 1;
        let request = self.open_request;
        let generation = self.generation;
        self.opening = true;
        let service = self.service.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.resolve_link(&link) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.open_request != request {
                    return;
                }
                this.opening = false;
                match result {
                    Ok(location) => {
                        this.change_directory(
                            location
                                .path
                                .parent()
                                .unwrap_or_else(|| std::path::Path::new(""))
                                .to_path_buf(),
                            window,
                            cx,
                        );
                        this.active = Some(location.clone());
                        this.open_file(location, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(listing) = &self.listing else {
            return;
        };
        let workspace = listing.workspace.clone();
        let directory = self.directory.clone();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(self.language.text(Text::FilesImport).into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    let commands = paths
                        .into_iter()
                        .filter_map(|source| {
                            let name = source.file_name()?.to_os_string();
                            Some(FileCommand::Import {
                                location: FileLocation {
                                    workspace: workspace.clone(),
                                    path: directory.join(name),
                                },
                                source,
                            })
                        })
                        .collect();
                    this.apply_commands(commands, window, cx);
                });
            }
        })
        .detach();
    }

    fn apply_commands(
        &mut self,
        commands: Vec<FileCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mutating || commands.is_empty() {
            return;
        }
        self.mutating = true;
        self.error = None;
        let service = self.service.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            for command in commands {
                service.apply(command)?;
            }
            Ok::<(), FileError>(())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.mutating = false;
                if this.generation == generation {
                    this.error = result.err();
                    this.refresh(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_active(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self
            .active
            .as_ref()
            .and_then(|location| self.editors.get(location))
        {
            editor.update(cx, |editor, cx| editor.save(window, cx));
        }
    }
}

fn file_icon(kind: FileKind) -> IconName {
    match kind {
        FileKind::Directory => IconName::FolderClosed,
        FileKind::Image => IconName::Image,
        _ => IconName::FileText,
    }
}
fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024. * 1024.))
    }
}
fn error_text(language: Language, error: &FileError) -> String {
    let text = language.text(match error {
        FileError::Unavailable => Text::FilesUnavailable,
        FileError::InvalidPath => Text::FilesInvalidPath,
        FileError::OutsideWorkspace => Text::FilesOutside,
        FileError::WorkspaceChanged => Text::FilesWorkspaceChanged,
        FileError::NotFound => Text::FilesMissing,
        FileError::AlreadyExists => Text::FilesExists,
        FileError::Conflict => Text::FilesConflict,
        FileError::TooLarge => Text::FilesTooLarge,
        FileError::NotText => Text::FilesBinaryContent,
        FileError::Io(_) => Text::FilesIoError,
    });
    if let FileError::Io(detail) = error {
        format!("{text} {detail}")
    } else {
        text.to_owned()
    }
}
