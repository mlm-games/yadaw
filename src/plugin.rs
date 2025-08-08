use crate::lv2_plugin_host::{ControlPortInfo, LV2PluginHost, PluginInfo};
use crate::state::{PluginDescriptor, PluginParam};
use anyhow::Result;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;

pub use crate::lv2_plugin_host::PluginInfo as PluginScanResult;

static PLUGIN_HOST: Lazy<Mutex<Option<LV2PluginHost>>> = Lazy::new(|| Mutex::new(None));

pub struct PluginScanner {
    pub(crate) plugins: Vec<PluginInfo>,
}

impl PluginScanner {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn discover_plugins(&mut self) {
        // Get plugins from the host
        let host_lock = PLUGIN_HOST.lock();
        if let Some(host) = host_lock.as_ref() {
            self.plugins = host.get_available_plugins().to_vec();
        }
    }

    pub fn get_plugins(&self) -> Vec<PluginScanResult> {
        self.plugins.clone()
    }
}

/// Create a plugin descriptor from URI
pub fn create_plugin_instance(uri: &str, _sample_rate: f32) -> Result<PluginDescriptor> {
    let host_lock = PLUGIN_HOST.lock();
    let host = host_lock
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Plugin host not initialized"))?;

    let plugin_info = host
        .get_available_plugins()
        .iter()
        .find(|p| p.uri == uri)
        .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", uri))?;

    let mut params = HashMap::new();

    for port in &plugin_info.control_ports {
        params.insert(
            port.symbol.clone(),
            PluginParam {
                index: port.index,
                name: port.name.clone(),
                value: port.default,
                min: port.min,
                max: port.max,
                default: port.default,
            },
        );
    }

    Ok(PluginDescriptor {
        uri: uri.to_string(),
        name: plugin_info.name.clone(),
        bypass: false,
        params,
    })
}

pub fn initialize_plugin_host(sample_rate: f64, max_block_size: usize) -> Result<()> {
    let mut host_lock = PLUGIN_HOST.lock();
    *host_lock = Some(LV2PluginHost::new(sample_rate, max_block_size)?);
    Ok(())
}
