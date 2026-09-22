use anyhow::{Context, Result, bail};
use relay_core::settings::{Language, SettingsService, SettingsSnapshot};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
};

#[derive(Serialize, Deserialize)]
struct SavedSettings {
    version: u32,
    language: String,
}

#[derive(Default)]
struct State {
    view: SettingsSnapshot,
    generation: u64,
}

/// One ordered writer prevents a slow earlier toggle from replacing a later choice.
pub struct SettingsStore {
    state: Arc<Mutex<State>>,
    writer: Mutex<Option<mpsc::Sender<(u64, Language)>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl SettingsStore {
    pub fn new(root: PathBuf) -> Self {
        let mut initial = State::default();
        match load(&root) {
            Ok(language) => initial.view.language = language,
            Err(error) => {
                initial.view.error = Some(format!("Could not read settings: {error:#}"));
            }
        }
        let mut preserve_unreadable_file = initial.view.error.is_some();
        let state = Arc::new(Mutex::new(initial));
        let shared = state.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok((mut generation, mut language)) = receiver.recv() {
                while let Ok(latest) = receiver.try_recv() {
                    (generation, language) = latest;
                }
                let result = (|| -> Result<()> {
                    if preserve_unreadable_file {
                        // Retry after the original file becomes readable; never replace corrupt data.
                        load(&root)?;
                        preserve_unreadable_file = false;
                    }
                    save(&root, language)
                })();
                let mut state = shared.lock().expect("settings lock");
                if state.generation == generation {
                    state.view.saving = false;
                    state.view.error = result
                        .err()
                        .map(|e| format!("Could not save settings: {e:#}"));
                }
            }
        });
        Self {
            state,
            writer: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
        }
    }

    /// Drain queued preferences before process exit. Safe to call more than once.
    pub fn shutdown(&self) {
        self.writer.lock().expect("settings writer lock").take();
        if let Some(worker) = self.worker.lock().expect("settings worker lock").take() {
            let _ = worker.join();
        }
    }
}

impl SettingsService for SettingsStore {
    fn snapshot(&self) -> SettingsSnapshot {
        self.state.lock().expect("settings lock").view.clone()
    }

    fn set_language(&self, language: Language) {
        let mut state = self.state.lock().expect("settings lock");
        state.view.language = language;
        state.generation += 1;
        state.view.saving = true;
        state.view.error = None;
        let writer = self.writer.lock().expect("settings writer lock");
        if writer
            .as_ref()
            .is_none_or(|sender| sender.send((state.generation, language)).is_err())
        {
            state.view.saving = false;
            state.view.error = Some("Settings writer has stopped.".into());
        }
    }
}

impl Drop for SettingsStore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn load(root: &Path) -> Result<Language> {
    let bytes = match fs::read(root.join("settings.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Language::default()),
        Err(error) => return Err(error.into()),
    };
    let saved: SavedSettings =
        serde_json::from_slice(&bytes).context("Reading application preferences")?;
    if saved.version != 1 {
        bail!(
            "Unsupported settings version {}. The file has been preserved.",
            saved.version
        );
    }
    Language::from_code(&saved.language)
        .context("Unsupported interface language. The file has been preserved.")
}

fn save(root: &Path, language: Language) -> Result<()> {
    fs::create_dir_all(root)?;
    let temporary = root.join(format!(
        "settings-{}.pending",
        crate::installer::unique_id()
    ));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&SavedSettings {
            version: 1,
            language: language.code().into(),
        })?)?;
        file.sync_all()?;
        fs::rename(&temporary, root.join("settings.json"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!("relay-settings-{}", crate::installer::unique_id()))
    }

    #[test]
    fn new_install_defaults_to_chinese_and_last_toggle_survives_restart() {
        let root = directory();
        let store = SettingsStore::new(root.clone());
        assert_eq!(store.snapshot().language, Language::SimplifiedChinese);
        for _ in 0..20 {
            store.set_language(Language::English);
            store.set_language(Language::SimplifiedChinese);
        }
        store.set_language(Language::English);
        store.shutdown();
        assert!(!store.snapshot().saving);
        assert!(store.snapshot().error.is_none());
        drop(store);
        let reopened = SettingsStore::new(root.clone());
        assert_eq!(reopened.snapshot().language, Language::English);
        reopened.set_language(Language::SimplifiedChinese);
        drop(reopened); // Drop also flushes the last choice.
        assert_eq!(load(&root).unwrap(), Language::SimplifiedChinese);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_and_unknown_settings_are_never_overwritten() {
        for original in [
            "{broken",
            r#"{"version":2,"language":"en"}"#,
            r#"{"version":1,"language":"future"}"#,
        ] {
            let root = directory();
            fs::create_dir_all(&root).unwrap();
            let path = root.join("settings.json");
            fs::write(&path, original).unwrap();
            let store = SettingsStore::new(root.clone());
            store.set_language(Language::English);
            store.shutdown();
            assert_eq!(store.snapshot().language, Language::English);
            assert!(store.snapshot().error.is_some());
            drop(store);
            assert_eq!(fs::read_to_string(path).unwrap(), original);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn save_failure_is_visible_without_reverting_the_current_language() {
        let root = directory();
        let store = SettingsStore::new(root.clone());
        fs::write(&root, "not a directory").unwrap();
        store.set_language(Language::English);
        store.shutdown();
        let snapshot = store.snapshot();
        assert_eq!(snapshot.language, Language::English);
        assert!(!snapshot.saving);
        assert!(snapshot.error.is_some());
        fs::remove_file(root).unwrap();
    }
}
