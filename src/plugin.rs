use anyhow::{Result, anyhow};

use crate::lv2_plugin_host::{ControlPortInfo, PluginInfo};
use crate::messages::AudioCommand;
use crate::model::plugin::PluginDescriptor;
use crate::model::plugin_api::{BackendKind, UnifiedPluginInfo};
use crate::plugin_host::{get_available_plugins, with_host};

pub trait PluginCategorizationInfo {
    fn name(&self) -> &str;
    fn uri(&self) -> &str;
    fn is_instrument(&self) -> bool;
    fn audio_inputs(&self) -> usize;
    fn audio_outputs(&self) -> usize;
    fn has_midi(&self) -> bool;
}

impl PluginCategorizationInfo for PluginInfo {
    fn name(&self) -> &str {
        &self.name
    }
    fn uri(&self) -> &str {
        &self.uri
    }
    fn is_instrument(&self) -> bool {
        self.is_instrument
    }
    fn audio_inputs(&self) -> usize {
        self.audio_inputs
    }
    fn audio_outputs(&self) -> usize {
        self.audio_outputs
    }
    fn has_midi(&self) -> bool {
        self.has_midi
    }
}

impl PluginCategorizationInfo for UnifiedPluginInfo {
    fn name(&self) -> &str {
        &self.name
    }
    fn uri(&self) -> &str {
        &self.uri
    }
    fn is_instrument(&self) -> bool {
        self.is_instrument
    }
    fn audio_inputs(&self) -> usize {
        self.audio_inputs
    }
    fn audio_outputs(&self) -> usize {
        self.audio_outputs
    }
    fn has_midi(&self) -> bool {
        self.has_midi
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
        id: 0,
        uri: uri.to_string(),
        name: plugin_info.name.clone(),
        backend: BackendKind::Clap,
        bypass: false,
        params,
        preset_name: None,
        custom_name: None,
    })
}

pub struct PluginParameterUpdate {
    pub track_id: u64,
    pub plugin_id: u64,
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
            self.plugin_id,
            self.param_name.clone(),
            self.value,
        )
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
pub fn categorize_plugin(p: &impl PluginCategorizationInfo) -> Vec<String> {
    let mut categories = vec!["All".to_string()];
    if p.is_instrument() || (p.has_midi() && p.audio_outputs() > 0 && p.audio_inputs() == 0) {
        categories.push("Instruments".to_string());
    } else if p.audio_inputs() > 0 && p.audio_outputs() > 0 {
        categories.push("Effects".to_string());
    }
    let name = p.name().to_lowercase();
    let uri = p.uri().to_lowercase();

    if name.contains("compressor") || name.contains("limiter") || name.contains("gate") {
        categories.push("Dynamics".to_string());
    }
    if name.contains("eq") || name.contains("equalizer") || name.contains("filter") {
        categories.push("EQ".to_string());
    }
    if name.contains("reverb") || name.contains("room") || name.contains("hall") {
        categories.push("Reverb".to_string());
    }
    if name.contains("delay") || name.contains("echo") {
        categories.push("Delay".to_string());
    }
    if name.contains("chorus") || name.contains("flanger") || name.contains("phaser") {
        categories.push("Modulation".to_string());
    }
    if name.contains("distortion") || name.contains("overdrive") || name.contains("saturation") {
        categories.push("Distortion".to_string());
    }
    if name.contains("utility") || name.contains("meter") || name.contains("analyzer") {
        categories.push("Utility".to_string());
    }
    categories
}

pub fn get_control_port_info(uri: &str, symbol: &str) -> Option<ControlPortInfo> {
    with_host(|h| {
        h.get_available_plugins()
            .iter()
            .find(|p| p.uri == uri)
            .and_then(|p| p.control_ports.iter().find(|c| c.symbol == symbol))
            .cloned()
    })
    .ok()
    .flatten()
}
