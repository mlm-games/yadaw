use once_cell::sync::{Lazy, OnceCell};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

static PRELOADED: OnceCell<HashMap<&'static str, String>> = OnceCell::new();

/// In-memory config cache used on wasm so that writes are immediately
/// visible to subsequent reads (OPFS writes are async). Updated by
/// `save_config_string` and checked by `read_config_string`.
static CONFIG_CACHE: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(target_arch = "wasm32")]
mod opfs_io {
    use std::collections::HashMap;

    pub async fn init() -> Result<(), String> {
        // Ensure directories exist for features that may not have
        // written files yet (projects, presets, plugins).
        for path in &[
            crate::paths::opfs::DIR_PROJECTS,
            crate::paths::opfs::DIR_PRESETS,
            crate::paths::opfs::DIR_PLUGINS,
        ] {
            opfs::ensure_dir(path)
                .await
                .map_err(|e| format!("OPFS init: {e}"))?;
        }
        let mut map = HashMap::new();
        for key in &[
            crate::paths::opfs::FILE_CONFIG,
            crate::paths::opfs::FILE_CUSTOM_THEMES,
            crate::paths::opfs::FILE_CURRENT_THEME,
            crate::paths::opfs::FILE_SHORTCUTS,
            "config/layouts.json",
            "config/autosave.json",
            "config/recent_projects.json",
        ] {
            if let Ok(data) = read_string(key).await {
                map.insert(*key, data);
            }
        }
        let _ = super::PRELOADED.set(map);
        Ok(())
    }

    pub(super) fn get_preloaded(key: &str) -> Option<&str> {
        super::PRELOADED.get().and_then(|m| m.get(key).map(|s| s.as_str()))
    }

    async fn read_string(name: &str) -> Result<String, String> {
        let data = opfs::read(name)
            .await
            .map_err(|e| format!("read {name}: {e}"))?;
        String::from_utf8(data).map_err(|e| format!("decode {name}: {e}"))
    }
}

#[cfg(target_arch = "wasm32")]
pub use opfs_io::init;

pub fn read_config_string(wasm_key: &str, fs_path: &Path) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(data) = opfs_io::get_preloaded(wasm_key) {
            return Some(data.to_string());
        }
        if let Some(data) = CONFIG_CACHE.lock().unwrap().get(wasm_key) {
            return Some(data.clone());
        }
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(fs_path).ok()
    }
}

/// Build an OPFS cache filename from a 64-bit content hash.
pub fn audio_cache_key(hash: u64) -> String {
    format!("audio_{:016x}.f32", hash)
}

/// Write decoded audio samples to the OPFS cache, keyed by content hash.
/// No-op on native.
pub async fn cache_audio_by_hash(hash: u64, samples: &[f32]) -> anyhow::Result<()> {
    let key = audio_cache_key(hash);
    let data: &[u8] =
        unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 4) };
    #[cfg(target_arch = "wasm32")]
    {
        let full_key = format!("{}/{}", crate::paths::opfs::DIR_CACHE, key);
        opfs::write(&full_key, data)
            .await
            .map_err(|e| anyhow::anyhow!("cache audio: {e}"))?;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = data;
    }
    Ok(())
}

/// Read previously cached audio samples from OPFS by content hash.
/// Returns `None` if not cached.
pub async fn read_cached_audio_by_hash(hash: u64) -> Option<Vec<f32>> {
    let key = audio_cache_key(hash);
    #[cfg(target_arch = "wasm32")]
    {
        let full_key = format!("{}/{}", crate::paths::opfs::DIR_CACHE, key);
        let data = opfs::read(&full_key).await.ok()?;
        let (_prefix, samples, _suffix) = unsafe { data.align_to::<f32>() };
        Some(samples.to_vec())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

pub fn save_config_string(wasm_key: &str, fs_path: &Path, data: &str) -> anyhow::Result<()> {
    #[cfg(target_arch = "wasm32")]
    {
        CONFIG_CACHE.lock().unwrap().insert(wasm_key.to_string(), data.to_string());
        let key = wasm_key.to_string();
        let data = data.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = opfs::write(&key, data.as_bytes()).await;
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
