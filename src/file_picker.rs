use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

pub enum PickedFile {
    Uri(String),
    Path(PathBuf),
}

pub struct Picker<T> {
    rx: Option<Receiver<Result<Option<T>, String>>>,
}

impl<T: Send + 'static> Picker<T> {
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

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_arch = "wasm32"))]
pub use crate::file_picker_desktop::{pick_open_file, pick_save_file, pick_multiple_audio, pick_directory};

#[cfg(target_os = "android")]
pub use crate::file_picker_android::{pick_open_file, pick_save_file, pick_multiple_audio, pick_directory, write_file_to_uri};

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_arch = "wasm32"))]
pub fn write_file_to_uri(_source_path: &PathBuf, _uri: &str) -> Result<(), String> {
    Err("write_file_to_uri not needed on desktop".into())
}

#[cfg(target_os = "android")]
pub use rlobkit_dialogs::{PlatformFile, RlobKit};

#[cfg(target_os = "android")]
pub use rlobkit_dialogs::picker::{OpenDirectoryOptions, OpenFileOptions, SaveFileOptions};

#[cfg(target_os = "android")]
pub use rlobkit_dialogs::{RlobKitMode, RlobKitType};