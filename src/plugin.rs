use crate::lv2_plugin_host::{ControlPortInfo, LV2PluginHost, PluginInfo};
use crate::state::{AudioCommand, PluginDescriptor, PluginParam};
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
pub fn create_plugin_instance(
    uri: &str,
    _sample_rate: f32,
) -> anyhow::Result<crate::state::PluginDescriptor> {
    let host_lock = PLUGIN_HOST.lock();
    let host = host_lock
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Plugin host not initialized"))?;

    let plugin_info = host
        .get_available_plugins()
        .iter()
        .find(|p| p.uri == uri)
        .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", uri))?;

    let mut params = std::collections::HashMap::new();
    for port in &plugin_info.control_ports {
        // Store only the value in the descriptor (0..1 or whatever the LV2 default range implies)
        params.insert(port.symbol.clone(), port.default);
    }

    Ok(crate::state::PluginDescriptor {
        uri: uri.to_string(),
        name: plugin_info.name.clone(),
        bypass: false,
        params,
        preset_name: None,
        custom_name: None,
    })
}

pub fn initialize_plugin_host(sample_rate: f64, max_block_size: usize) -> Result<()> {
    let mut host_lock = PLUGIN_HOST.lock();
    *host_lock = Some(LV2PluginHost::new(sample_rate, max_block_size)?);
    Ok(())
}

pub struct PluginParameterUpdate {
    pub track_id: usize,
    pub plugin_idx: usize,
    pub param_name: String,
    pub value: f32,
}

impl PluginParameterUpdate {
    pub fn apply_to_descriptor(
        descriptor: &mut crate::state::PluginDescriptor,
        param_name: &str,
        value: f32,
    ) -> anyhow::Result<()> {
        if let Some(v) = descriptor.params.get_mut(param_name) {
            *v = value;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Parameter {} not found", param_name))
        }
    }

    pub fn create_command(&self) -> AudioCommand {
        AudioCommand::SetPluginParam(
            self.track_id,
            self.plugin_idx,
            self.param_name.clone(),
            self.value,
        )
    }
}

// Add a helper trait for Track
pub trait PluginParameterAccess {
    fn update_plugin_param(
        &mut self,
        plugin_idx: usize,
        param_name: &str,
        value: f32,
    ) -> Result<()>;

    fn get_plugin_param(&self, plugin_idx: usize, param_name: &str) -> Option<f32>;
}

impl PluginParameterAccess for crate::state::Track {
    fn update_plugin_param(
        &mut self,
        plugin_idx: usize,
        param_name: &str,
        value: f32,
    ) -> anyhow::Result<()> {
        self.plugin_chain
            .get_mut(plugin_idx)
            .ok_or_else(|| anyhow::anyhow!("Plugin index {} out of bounds", plugin_idx))
            .and_then(|plugin| {
                if let Some(v) = plugin.params.get_mut(param_name) {
                    *v = value;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Parameter {} not found", param_name))
                }
            })
    }

    fn get_plugin_param(&self, plugin_idx: usize, param_name: &str) -> Option<f32> {
        self.plugin_chain
            .get(plugin_idx)
            .and_then(|plugin| plugin.params.get(param_name))
            .copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    Instrument,
    Effect,
    MidiFx,
    Unknown,
}

pub fn classify_plugin_uri(uri: &str) -> Option<PluginKind> {
    let host_lock = PLUGIN_HOST.lock();
    let host = host_lock.as_ref()?;
    host.get_available_plugins()
        .iter()
        .find(|p| p.uri == uri)
        .map(|info| {
            // Instruments: is_instrument flag OR (has MIDI input and generates audio)
            if info.is_instrument {
                PluginKind::Instrument
            } else if info.has_midi && info.audio_outputs > 0 {
                // Plugin accepts MIDI and produces audio - it's an instrument
                PluginKind::Instrument
            } else if info.audio_inputs > 0 && info.audio_outputs > 0 {
                // Has audio I/O - it's an effect
                PluginKind::Effect
            } else if info.has_midi && info.audio_inputs == 0 && info.audio_outputs == 0 {
                // MIDI only - MIDI effect
                PluginKind::MidiFx
            } else {
                PluginKind::Unknown
            }
        })
}

/// Categorizes plugin (based on name for effect subtypes)
pub fn categorize_plugin(plugin: &crate::lv2_plugin_host::PluginInfo) -> Vec<String> {
    let mut categories = Vec::new();

    categories.push("All".to_string());

    if plugin.is_instrument
        || (plugin.has_midi && plugin.audio_outputs > 0 && plugin.audio_inputs == 0)
    {
        categories.push("Instruments".to_string());
    } else if plugin.audio_inputs > 0 && plugin.audio_outputs > 0 {
        categories.push("Effects".to_string());
    }

    let name_lower = plugin.name.to_lowercase();
    let uri_lower = plugin.uri.to_lowercase();

    if name_lower.contains("compressor")
        || name_lower.contains("limiter")
        || name_lower.contains("gate")
        || name_lower.contains("expander")
        || uri_lower.contains("compressor")
        || uri_lower.contains("limiter")
    {
        categories.push("Dynamics".to_string());
    }

    if name_lower.contains("eq")
        || name_lower.contains("equalizer")
        || name_lower.contains("filter")
        || uri_lower.contains("eq")
        || uri_lower.contains("equalizer")
    {
        categories.push("EQ".to_string());
    }

    if name_lower.contains("reverb")
        || name_lower.contains("room")
        || name_lower.contains("hall")
        || uri_lower.contains("reverb")
    {
        categories.push("Reverb".to_string());
    }

    if name_lower.contains("delay") || name_lower.contains("echo") || uri_lower.contains("delay") {
        categories.push("Delay".to_string());
    }

    if name_lower.contains("chorus")
        || name_lower.contains("flanger")
        || name_lower.contains("phaser")
        || name_lower.contains("tremolo")
        || uri_lower.contains("modulation")
    {
        categories.push("Modulation".to_string());
    }

    if name_lower.contains("distortion")
        || name_lower.contains("overdrive")
        || name_lower.contains("fuzz")
        || name_lower.contains("saturation")
        || uri_lower.contains("distortion")
    {
        categories.push("Distortion".to_string());
    }

    if name_lower.contains("utility")
        || name_lower.contains("meter")
        || name_lower.contains("analyzer")
        || name_lower.contains("scope")
    {
        categories.push("Utility".to_string());
    }

    categories
}
