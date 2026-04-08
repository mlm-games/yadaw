use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::lv2_plugin_host::{LV2PluginHost, LV2PluginInstance, PluginInfo};

static HOST: Lazy<Arc<RwLock<Option<LV2PluginHost>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));

pub fn init(sample_rate: f64, max_block_size: usize) -> Result<()> {
    let mut guard = HOST.write();
    *guard = Some(LV2PluginHost::new(sample_rate, max_block_size)?);
    Ok(())
}

#[inline]
pub fn is_initialized() -> bool {
    HOST.read().is_some()
}

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

pub fn shutdown() {
    let mut guard = HOST.write();
    *guard = None;
}

pub fn with_host<R>(f: impl FnOnce(&LV2PluginHost) -> R) -> Result<R> {
    let guard = HOST.read();
    let host = guard
        .as_ref()
        .ok_or_else(|| anyhow!("Plugin host not initialized"))?;
    Ok(f(host))
}

pub fn get_available_plugins() -> Result<Vec<PluginInfo>> {
    with_host(|h| h.get_available_plugins().to_vec())
}

pub fn instantiate(uri: &str) -> Result<LV2PluginInstance> {
    with_host(|h| h.instantiate_plugin(uri))?
        .map_err(|e| anyhow!("Instantiate failed for {}: {e}", uri))
        .or_else(|_| with_host(|h| h.instantiate_plugin(uri))?)
}
