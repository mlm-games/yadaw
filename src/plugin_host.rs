//! Minimal global LV2 plugin host facade (SSOT).
//!
//! Usage:
//!   - Initialize once when the audio device/config is known:
//!       crate::plugin_host::init(sample_rate, MAX_BLOCK_SIZE)?;
//!   - Query/scanning on UI thread:
//!       let list = crate::plugin_host::get_available_plugins()?;
//!   - Instantiate a plugin (avoid doing this on the RT audio callback):
//!       let inst = crate::plugin_host::instantiate(uri)?;
//!
//! Notes:
//!   - This wrapper centralizes host ownership and removes duplicate statics.
//!   - Re-initialization replaces the host (e.g., if sample rate changes).
//!   - Avoid calling `instantiate` on the realtime audio thread; do it on a
//!     setup/control thread and pass the instance handle into the audio graph.

use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::lv2_plugin_host::{LV2PluginHost, LV2PluginInstance, PluginInfo};

static HOST: Lazy<Arc<RwLock<Option<LV2PluginHost>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));

/// Initialize (or reinitialize) the global LV2 host.
/// Safe to call multiple times; the host will be replaced atomically.
pub fn init(sample_rate: f64, max_block_size: usize) -> Result<()> {
    let mut guard = HOST.write();
    *guard = Some(LV2PluginHost::new(sample_rate, max_block_size)?);
    Ok(())
}

/// Returns true if the global host is present.
#[inline]
pub fn is_initialized() -> bool {
    HOST.read().is_some()
}

/// Replace the host if not initialized, otherwise no-op.
/// Useful when multiple subsystems race to ensure initialization.
pub fn ensure(sample_rate: f64, max_block_size: usize) -> Result<()> {
    let mut guard = HOST.write();

    let need_recreate = match guard.as_ref() {
        None => true,
        Some(host) => host.sample_rate() != sample_rate || host.max_block_size() != max_block_size,
    };

    if need_recreate {
        *guard = Some(LV2PluginHost::new(sample_rate, max_block_size)?);
    }

    Ok(())
}

/// Drop the global host (e.g., during shutdown).
pub fn shutdown() {
    let mut guard = HOST.write();
    *guard = None;
}

/// Borrow the host for read-only access.
pub fn with_host<R>(f: impl FnOnce(&LV2PluginHost) -> R) -> Result<R> {
    let guard = HOST.read();
    let host = guard
        .as_ref()
        .ok_or_else(|| anyhow!("Plugin host not initialized"))?;
    Ok(f(host))
}

/// List all available plugins.
pub fn get_available_plugins() -> Result<Vec<PluginInfo>> {
    with_host(|h| h.get_available_plugins().to_vec())
}

/// Instantiate a plugin by URI.
/// Avoid calling this on the realtime audio thread.
pub fn instantiate(uri: &str) -> Result<LV2PluginInstance> {
    with_host(|h| h.instantiate_plugin(uri))?
        .map_err(|e| anyhow!("Instantiate failed for {}: {e}", uri))
        .or_else(|_| with_host(|h| h.instantiate_plugin(uri))?)
}
