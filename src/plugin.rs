use anyhow::{anyhow, Result};

use crate::lv2_plugin_host::PluginInfo;
use crate::messages::AudioCommand;
use crate::model::plugin::{PluginDescriptor, PluginParam};
use crate::model::track::Track;
use crate::plugin_host::get_available_plugins;

pub use crate::lv2_plugin_host::PluginInfo as PluginScanResult;

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
        self.plugins = get_available_plugins().unwrap_or_default();
    }

    pub fn get_plugins(&self) -> Vec<PluginScanResult> {
        self.plugins.clone()
    }
}

/// Create a plugin descriptor from URI
pub fn create_plugin_instance(uri: &str, _sample_rate: f32) -> Result<PluginDescriptor> {
    let list = get_available_plugins()?;
    let plugin_info = list
        .into_iter()
        .find(|p| p.uri == uri)
        .ok_or_else(|| anyhow!("Plugin not found: {}", uri))?;

    let mut params = std::collections::HashMap::new();
    for port in &plugin_info.control_ports {
        params.insert(port.symbol.clone(), port.default);
    }

    Ok(PluginDescriptor {
        uri: uri.to_string(),
        name: plugin_info.name.clone(),
        bypass: false,
        params,
        preset_name: None,
        custom_name: None,
    })
}

pub struct PluginParameterUpdate {
    pub track_id: usize,
    pub plugin_idx: usize,
    pub param_name: String,
    pub value: f32,
}

impl PluginParameterUpdate {
    pub fn apply_to_descriptor(
        descriptor: &mut PluginDescriptor,
        param_name: &str,
        value: f32,
    ) -> Result<()> {
        if let Some(v) = descriptor.params.get_mut(param_name) {
            *v = value;
            Ok(())
        } else {
            Err(anyhow!("Parameter {} not found", param_name))
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

impl PluginParameterAccess for Track {
    fn update_plugin_param(
        &mut self,
        plugin_idx: usize,
        param_name: &str,
        value: f32,
    ) -> Result<()> {
        self.plugin_chain
            .get_mut(plugin_idx)
            .ok_or_else(|| anyhow!("Plugin index {} out of bounds", plugin_idx))
            .and_then(|plugin| {
                if let Some(v) = plugin.params.get_mut(param_name) {
                    *v = value;
                    Ok(())
                } else {
                    Err(anyhow!("Parameter {} not found", param_name))
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
    let list = get_available_plugins().ok()?;
    list.into_iter().find(|p| p.uri == uri).map(|info| {
        if info.is_instrument {
            PluginKind::Instrument
        } else if info.has_midi && info.audio_outputs > 0 {
            PluginKind::Instrument
        } else if info.audio_inputs > 0 && info.audio_outputs > 0 {
            PluginKind::Effect
        } else if info.has_midi && info.audio_inputs == 0 && info.audio_outputs == 0 {
            PluginKind::MidiFx
        } else {
            PluginKind::Unknown
        }
    })
}

/// Categorizes plugin (based on name for effect subtypes)
pub fn categorize_plugin(plugin: &PluginInfo) -> Vec<String> {
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
