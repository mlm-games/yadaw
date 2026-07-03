use std::collections::HashMap;
use std::path::Path;
use once_cell::sync::OnceCell;

static PRELOADED: OnceCell<HashMap<&'static str, String>> = OnceCell::new();

#[cfg(target_arch = "wasm32")]
mod opfs_io {
    use super::*;
    use opfs as opfs_crate;
    use opfs_crate::persistent;
    use opfs_crate::{DirectoryHandle as _, FileHandle as _, WritableFileStream as _};

    pub async fn init() -> Result<(), String> {
        let mut root = persistent::app_specific_dir()
            .await
            .map_err(|e| format!("OPFS init: {e}"))?;
        for name in &[crate::paths::opfs::DIR_CONFIG, crate::paths::opfs::DIR_CACHE, crate::paths::opfs::DIR_PROJECTS, crate::paths::opfs::DIR_PRESETS, crate::paths::opfs::DIR_PLUGINS] {
            root.get_directory_handle_with_options(
                name,
                &opfs_crate::GetDirectoryHandleOptions { create: true },
            )
            .await
            .map_err(|e| format!("create dir {name}: {e}"))?;
        }
        let mut map = HashMap::new();
        for key in &[crate::paths::opfs::FILE_CONFIG, crate::paths::opfs::FILE_CUSTOM_THEMES, crate::paths::opfs::FILE_CURRENT_THEME, crate::paths::opfs::FILE_SHORTCUTS] {
            if let Ok(data) = read_string(key).await {
                map.insert(*key, data);
            }
        }
        let _ = PRELOADED.set(map);
        Ok(())
    }

    pub(super) fn get_preloaded(key: &str) -> Option<&str> {
        PRELOADED.get().and_then(|m| m.get(key).map(|s| s.as_str()))
    }

    async fn read_string(name: &str) -> Result<String, String> {
        let data = read_file(name).await?;
        String::from_utf8(data).map_err(|e| format!("decode {name}: {e}"))
    }

    async fn read_file(name: &str) -> Result<Vec<u8>, String> {
        let mut root = persistent::app_specific_dir()
            .await
            .map_err(|e| format!("OPFS root: {e}"))?;
        let mut file = root
            .get_file_handle_with_options(name, &opfs_crate::GetFileHandleOptions { create: false })
            .await
            .map_err(|e| format!("open {name}: {e}"))?;
        file.read().await.map_err(|e| format!("read {name}: {e}"))
    }

    pub(super) async fn write_string(name: &str, data: &str) -> Result<(), String> {
        write_file(name, data.as_bytes()).await
    }

    async fn write_file(name: &str, data: &[u8]) -> Result<(), String> {
        let mut root = persistent::app_specific_dir()
            .await
            .map_err(|e| format!("OPFS root: {e}"))?;
        let mut file = root
            .get_file_handle_with_options(name, &opfs_crate::GetFileHandleOptions { create: true })
            .await
            .map_err(|e| format!("open {name}: {e}"))?;
        let mut writer = file
            .create_writable_with_options(&opfs_crate::CreateWritableOptions {
                keep_existing_data: false,
                mode: opfs_crate::WritableMode::Siloed,
            })
            .await
            .map_err(|e| format!("writer {name}: {e}"))?;
        writer
            .write_at_cursor_pos(data)
            .await
            .map_err(|e| format!("write {name}: {e}"))?;
        writer
            .close()
            .await
            .map_err(|e| format!("close {name}: {e}"))
    }
}

#[cfg(target_arch = "wasm32")]
pub use opfs_io::init;

pub fn read_config_string(wasm_key: &str, fs_path: &Path) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    if let Some(data) = opfs_io::get_preloaded(wasm_key) {
        return Some(data.to_string());
    }
    std::fs::read_to_string(fs_path).ok()
}

pub fn save_config_string(wasm_key: &str, fs_path: &Path, data: &str) -> anyhow::Result<()> {
    #[cfg(target_arch = "wasm32")]
    {
        let key = wasm_key.to_string();
        let data = data.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = opfs_io::write_string(&key, &data).await;
        });
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(parent) = fs_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(fs_path, data)?;
        Ok(())
    }
}
