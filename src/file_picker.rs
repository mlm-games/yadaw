use crate::spawn_detached;
use flume;

pub struct Picker<T> {
    rx: Option<flume::Receiver<Result<Option<T>, String>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + 'static> Picker<T> {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Option<T>, String>> + Send + 'static,
    {
        let (tx, rx) = flume::unbounded();
        spawn_detached!(async move {
            let result = f().await;
            let _ = tx.send(result);
        });
        Self { rx: Some(rx) }
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: 'static> Picker<T> {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: FnOnce() -> Fut + 'static,
        Fut: std::future::Future<Output = Result<Option<T>, String>> + 'static,
    {
        let (tx, rx) = flume::unbounded();
        spawn_detached!(async move {
            let result = f().await;
            let _ = tx.send(result);
        });
        Self { rx: Some(rx) }
    }
}

impl<T> Picker<T> {
    pub fn poll(&mut self) -> Option<Result<Option<T>, String>> {
        let rx = self.rx.as_mut()?;
        match rx.try_recv() {
            Ok(res) => {
                self.rx = None;
                Some(res)
            }
            Err(flume::TryRecvError::Empty) => None,
            Err(flume::TryRecvError::Disconnected) => {
                self.rx = None;
                Some(Err("Picker task disconnected".into()))
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
pub fn write_file_to_uri(_source_path: &std::path::Path, _uri: &str) -> Result<(), String> {
    Err("write_file_to_uri not needed on desktop".into())
}

pub use rlobkit_dialogs::{PlatformFile, RlobKit};

pub use rlobkit_dialogs::picker::{OpenDirectoryOptions, OpenFileOptions, SaveFileOptions};

pub use rlobkit_dialogs::{RlobKitMode, RlobKitType};
