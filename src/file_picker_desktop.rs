use crate::file_picker::Picker;
use rlobkit_dialogs::picker::{OpenFileOptions, SaveFileOptions};
use rlobkit_dialogs::{PlatformFile, RlobKit, RlobKitMode, RlobKitType};

pub fn pick_open_file(title: &str, extensions: &[&str]) -> Picker<PlatformFile> {
    let title = title.to_string();
    let exts: Vec<String> = extensions.iter().map(|s| s.to_string()).collect();

    Picker::new(move || async move {
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
        Ok(result.and_then(|mut files| files.pop()))
    })
}

pub fn pick_save_file(title: &str, suggested_name: &str, extension: &str) -> Picker<PlatformFile> {
    let title = title.to_string();
    let suggested = suggested_name.to_string();
    let ext = extension.to_string();

    Picker::new(move || async move {
        let result = RlobKit::open_file_saver(SaveFileOptions {
            suggested_name: Some(suggested),
            extension: Some(ext.clone()),
            file_type: Some(RlobKitType::Custom {
                extensions: vec![ext],
                mime_types: vec![],
            }),
            title: Some(title.to_string()),
            initial_directory: None,
            #[cfg(target_arch = "wasm32")]
            data: None,
        })
        .await
        .map_err(|e| e.to_string())?;
        Ok(result)
    })
}

pub fn pick_multiple_audio() -> Picker<Vec<PlatformFile>> {
    use crate::constants::AUDIO_IMPORT_EXTENSIONS;
    let extensions: Vec<String> = AUDIO_IMPORT_EXTENSIONS
        .iter()
        .map(|s| s.to_string())
        .collect();

    Picker::new(move || async move {
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
        Ok(result)
    })
}

pub fn pick_directory(title: &str) -> Picker<PlatformFile> {
    let title = title.to_string();

    Picker::new(move || async move {
        let result =
            RlobKit::open_directory_picker(rlobkit_dialogs::picker::OpenDirectoryOptions {
                title: Some(title.to_string()),
                initial_directory: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(result.map(|dir| {
            PlatformFile::from_path(dir.name().unwrap_or_default(), dir.path().map(|p| p.to_path_buf()).unwrap_or_default())
        }))
    })
}

pub fn write_file_to_uri(_source_path: &std::path::Path, _uri: &str) -> Result<(), String> {
    Err("write_file_to_uri not needed on desktop".into())
}
