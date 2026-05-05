#![cfg(target_os = "android")]

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use rlobkit_dialogs::picker::{OpenFileOptions, SaveFileOptions};
use rlobkit_dialogs::{RlobKit, RlobKitMode, RlobKitType};

#[derive(Debug, Clone)]
pub enum PickedFile {
    Uri(String),
    Path(PathBuf),
}

pub struct AndroidPicker<T> {
    rx: Option<Receiver<Result<Option<T>, String>>>,
}

impl<T: Send + 'static> AndroidPicker<T> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnOnce() -> Result<Option<T>, String> + Send + 'static,
    {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = f();
            let _ = tx.send(result);
        });
        Self { rx: Some(rx) }
    }

    pub fn poll(&mut self) -> Option<Result<Option<T>, String>> {
        let rx = self.rx.as_mut()?;
        match rx.try_recv() {
            Ok(res) => {
                self.rx = None;
                Some(res)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                Some(Err("Picker thread disconnected".into()))
            }
        }
    }

    pub fn is_done(&self) -> bool {
        self.rx.is_none()
    }
}

fn block_on_android<T>(future: impl std::future::Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("Failed to create async runtime");
    runtime.block_on(future)
}

pub fn pick_open_file(title: &str, extensions: &[&str]) -> AndroidPicker<PickedFile> {
    let title = title.to_string();
    let exts: Vec<String> = extensions.iter().map(|s| s.to_string()).collect();
    AndroidPicker::new(move || {
        block_on_android(async {
            let result = RlobKit::open_file_picker(OpenFileOptions {
                file_type: RlobKitType::Custom {
                    extensions: exts,
                    mime_types: vec!["*/*".to_string()],
                },
                mode: RlobKitMode::Single,
                title: Some(title.to_string()),
                initial_directory: None,
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(result.and_then(|mut files| {
                files.pop().and_then(|f| {
                    if let Some(uri) = f.uri() {
                        Some(PickedFile::Uri(uri.to_string()))
                    } else {
                        f.path().map(|p| PickedFile::Path(p.to_path_buf()))
                    }
                })
            }))
        })
    })
}

pub fn pick_save_file(title: &str, suggested_name: &str, extension: &str) -> AndroidPicker<PickedFile> {
    let title = title.to_string();
    let suggested = suggested_name.to_string();
    let ext = extension.to_string();
    AndroidPicker::new(move || {
        block_on_android(async {
            let result = RlobKit::open_file_saver(SaveFileOptions {
                suggested_name: Some(suggested),
                extension: Some(ext),
                title: Some(title.to_string()),
                initial_directory: None,
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(result.and_then(|f| {
                if let Some(uri) = f.uri() {
                    Some(PickedFile::Uri(uri.to_string()))
                } else {
                    f.path().map(|p| PickedFile::Path(p.to_path_buf()))
                }
            }))
        })
    })
}

pub fn pick_multiple_audio() -> AndroidPicker<Vec<PickedFile>> {
    AndroidPicker::new(move || {
        block_on_android(async {
            let result = RlobKit::open_file_picker(OpenFileOptions {
                file_type: RlobKitType::Custom {
                    extensions: vec![
                        "wav".to_string(),
                        "mp3".to_string(),
                        "flac".to_string(),
                        "ogg".to_string(),
                        "m4a".to_string(),
                        "aac".to_string(),
                        "mid".to_string(),
                        "midi".to_string(),
                    ],
                    mime_types: vec!["*/*".to_string()],
                },
                mode: RlobKitMode::Multiple { limit: None },
                title: Some("Import Audio".to_string()),
                initial_directory: None,
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(result.map(|files| {
                files
                    .into_iter()
                    .filter_map(|f| {
                        if let Some(uri) = f.uri() {
                            Some(PickedFile::Uri(uri.to_string()))
                        } else {
                            f.path().map(|p| PickedFile::Path(p.to_path_buf()))
                        }
                    })
                    .collect()
            }))
        })
    })
}

pub fn pick_directory(title: &str) -> AndroidPicker<PickedFile> {
    let title = title.to_string();
    AndroidPicker::new(move || {
        block_on_android(async {
            let result = RlobKit::open_directory_picker(rlobkit_dialogs::picker::OpenDirectoryOptions {
                title: Some(title.to_string()),
                initial_directory: None,
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(result.map(|dir| PickedFile::Path(dir.path().to_path_buf())))
        })
    })
}

pub fn write_file_to_uri(source_path: &PathBuf, uri: &str) -> Result<(), String> {
    let target = rlobkit_dialogs::PlatformFile::from_uri(uri);
    rlobkit_dialogs::RlobKit::write_file_from_path(&target, source_path).map_err(|e| e.to_string())
}