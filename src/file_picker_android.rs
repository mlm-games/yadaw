#![cfg(target_os = "android")]

use crate::constants::AUDIO_IMPORT_EXTENSIONS;
use crate::file_picker::PickedFile;
use rlobkit_dialogs::picker::{OpenFileOptions, SaveFileOptions};
use rlobkit_dialogs::{RlobKit, RlobKitMode, RlobKitType};
use std::os::fd::FromRawFd;

fn block_on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("Failed to create async runtime");
    runtime.block_on(future)
}

pub fn pick_open_file(title: &str, extensions: &[&str]) -> crate::file_picker::Picker<PickedFile> {
    let title = title.to_string();
    let exts: Vec<String> = extensions.iter().map(|s| s.to_string()).collect();

    crate::file_picker::Picker::new(move || {
        block_on_runtime(async {
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
                        let name = f.name();
                        Some(PickedFile::Uri(uri.to_string(), name))
                    } else {
                        f.path().map(|p| PickedFile::Path(p.to_path_buf()))
                    }
                })
            }))
        })
    })
}

pub fn pick_save_file(
    title: &str,
    suggested_name: &str,
    extension: &str,
) -> crate::file_picker::Picker<PickedFile> {
    let title = title.to_string();
    let suggested = suggested_name.to_string();
    let ext = extension.to_string();

    crate::file_picker::Picker::new(move || {
        block_on_runtime(async {
            let result = RlobKit::open_file_saver(SaveFileOptions {
                suggested_name: Some(suggested),
                extension: Some(ext),
                title: Some(title.to_string()),
                initial_directory: None,
                file_type: None,
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(result.and_then(|f| {
                if let Some(uri) = f.uri() {
                    let name = f.name();
                    Some(PickedFile::Uri(uri.to_string(), name))
                } else {
                    f.path().map(|p| PickedFile::Path(p.to_path_buf()))
                }
            }))
        })
    })
}

pub fn pick_multiple_audio() -> crate::file_picker::Picker<Vec<PickedFile>> {
    let extensions: Vec<String> = AUDIO_IMPORT_EXTENSIONS
        .iter()
        .map(|s| s.to_string())
        .collect();

    crate::file_picker::Picker::new(move || {
        block_on_runtime(async {
            let result = RlobKit::open_file_picker(OpenFileOptions {
                file_type: RlobKitType::Custom {
                    extensions: extensions.clone(),
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
                            let name = f.name();
                            Some(PickedFile::Uri(uri.to_string(), name))
                        } else {
                            f.path().map(|p| PickedFile::Path(p.to_path_buf()))
                        }
                    })
                    .collect()
            }))
        })
    })
}

pub fn pick_directory(title: &str) -> crate::file_picker::Picker<PickedFile> {
    let title = title.to_string();

    crate::file_picker::Picker::new(move || {
        block_on_runtime(async {
            let result =
                RlobKit::open_directory_picker(rlobkit_dialogs::picker::OpenDirectoryOptions {
                    title: Some(title.to_string()),
                    initial_directory: None,
                })
                .await
                .map_err(|e| e.to_string())?;
            Ok(result.map(|dir| PickedFile::Path(dir.path().to_path_buf())))
        })
    })
}

pub fn write_file_to_uri(source_path: &std::path::PathBuf, uri: &str) -> Result<(), String> {
    let fd = rlobkit_dialogs::take_writable_fd_for_uri(uri)
        .ok_or_else(|| "Failed to get writable file descriptor".to_string())?;

    use std::os::fd::IntoRawFd;
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    std::io::copy(
        &mut std::io::BufReader::new(std::fs::File::open(source_path).map_err(|e| e.to_string())?),
        &mut file,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
