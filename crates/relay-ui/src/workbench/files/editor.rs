use super::*;
use gpui_kit::component::text::TextView;
#[cfg(test)]
mod tests;
mod view;

pub(super) struct FileEditor {
    pub(super) document: FileDocument,
    pub(super) language: Language,
    service: Arc<dyn FileService>,
    body: Entity<TextareaState>,
    preview: bool,
    saving: bool,
    external: Option<FileDocument>,
    error: Option<FileError>,
    image: Option<Arc<Image>>,
    _subscription: Subscription,
}

impl FileEditor {
    pub(super) fn new(
        document: FileDocument,
        service: Arc<dyn FileService>,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = match &document.content {
            FileContent::Text { text, .. } => text.clone(),
            _ => String::new(),
        };
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(content)
                .soft_wrap(true)
        });
        let subscription = cx.subscribe(&body, |_, _, _: &InputEvent, cx| cx.notify());
        let image = image_source(&document);
        Self {
            document,
            language,
            service,
            body,
            preview: true,
            saving: false,
            external: None,
            error: None,
            image,
            _subscription: subscription,
        }
    }

    pub(super) fn dirty(&self, cx: &App) -> bool {
        match &self.document.content {
            FileContent::Text { text, .. } => self.body.read(cx).value().as_ref() != text,
            _ => false,
        }
    }

    pub(super) fn sync_disk(
        &mut self,
        document: FileDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        if self.document.version == document.version {
            self.document.stamp = document.stamp;
            self.external = None;
            cx.notify();
            return;
        }
        let matches_draft = matches!(&document.content, FileContent::Text { text, .. } if self.body.read(cx).value().as_ref() == text);
        if self.dirty(cx) && !matches_draft {
            self.external = Some(document);
        } else {
            self.replace_document(document, window, cx);
        }
        cx.notify();
    }

    fn replace_document(
        &mut self,
        document: FileDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let FileContent::Text { text, .. } = &document.content {
            self.body
                .update(cx, |body, cx| body.set_value(text.clone(), window, cx));
        }
        self.image = image_source(&document);
        self.document = document;
        self.external = None;
        self.error = None;
    }

    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || !self.dirty(cx) {
            return;
        }
        let content = self.body.read(cx).value().to_string();
        let command = FileCommand::Save {
            location: self.document.location.clone(),
            expected_version: self.document.version.clone(),
            content: content.clone(),
        };
        self.saving = true;
        self.error = None;
        let service = self.service.clone();
        let task = cx.background_executor().spawn(async move {
            let location = service.apply(command)?;
            service.read(location)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(document) if matches!(&document.content, FileContent::Text { text, .. } if text == &content) => {
                        this.replace_document(document, window, cx);
                    }
                    Ok(document) => {
                        this.external = Some(document);
                        this.error = Some(FileError::Conflict);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let service = self.service.clone();
        let location = self.document.location.clone();
        self.saving = true;
        let task = cx
            .background_executor()
            .spawn(async move { service.read(location) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(document) => this.replace_document(document, window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut parent = self.document.location.clone();
        parent.path = parent
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .to_path_buf();
        let weak = cx.entity().downgrade();
        new_entry::show(
            parent,
            false,
            Some(self.body.read(cx).value().to_string()),
            self.language,
            self.service.clone(),
            Box::new(move |location, _, cx| {
                let _ = weak.update(cx, |_, cx| {
                    cx.emit(OpenFileLink {
                        link: location.absolute_path().to_string_lossy().into_owned(),
                    })
                });
            }),
            window,
            cx,
        );
    }

    fn open_link(&self, link: String, cx: &mut Context<Self>) {
        if link.starts_with("https://")
            || link.starts_with("http://")
            || link.starts_with("mailto:")
        {
            cx.open_url(&link);
            return;
        }
        let link =
            if link.starts_with('/') || link.starts_with("file:") || link.starts_with("sandbox:") {
                link
            } else {
                self.document
                    .location
                    .absolute_path()
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .join(link)
                    .to_string_lossy()
                    .into_owned()
            };
        cx.emit(OpenFileLink { link });
    }
}

fn image_source(document: &FileDocument) -> Option<Arc<Image>> {
    let FileContent::Image { bytes, extension } = &document.content else {
        return None;
    };
    let format = match extension.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "ico" => ImageFormat::Ico,
        _ => return None,
    };
    Some(Arc::new(Image::from_bytes(format, bytes.to_vec())))
}
