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

/// Categorizes plugin based on URI, name, and port configuration
pub fn categorize_plugin(p: &impl PluginCategorizationInfo) -> Vec<String> {
    let mut categories = vec!["All".to_string()];

    let name = p.name().to_lowercase();
    let uri = p.uri().to_lowercase();

    // Primary classification by I/O
    let is_instrument =
        p.is_instrument() || (p.has_midi() && p.audio_outputs() > 0 && p.audio_inputs() == 0);
    let is_effect = p.audio_inputs() > 0 && p.audio_outputs() > 0;
    let is_midi_fx = p.has_midi() && p.audio_inputs() == 0 && p.audio_outputs() == 0;
    let is_analyzer = p.audio_inputs() > 0 && p.audio_outputs() == 0;
    let is_generator = !p.has_midi() && p.audio_inputs() == 0 && p.audio_outputs() > 0;

    if is_instrument {
        categories.push("Instruments".to_string());

        // Instrument subtypes
        if matches_any(&name, &uri, &["synth", "synthesizer", "oscill"]) {
            categories.push("Synths".to_string());
        }
        if matches_any(&name, &uri, &["sampler", "sample", "drum", "percussion"]) {
            categories.push("Samplers".to_string());
        }
        if matches_any(&name, &uri, &["piano", "keys", "organ", "rhodes", "wurli"]) {
            categories.push("Keys".to_string());
        }
        if matches_any(&name, &uri, &["bass"]) && !name.contains("bassoon") {
            categories.push("Bass".to_string());
        }
        if matches_any(&name, &uri, &["string", "violin", "cello", "viola"]) {
            categories.push("Strings".to_string());
        }
        if matches_any(&name, &uri, &["pad", "ambient", "atmosphere"]) {
            categories.push("Pads".to_string());
        }
    }

    if is_effect {
        categories.push("Effects".to_string());

        // Dynamics
        if matches_any(
            &name,
            &uri,
            &[
                "compressor",
                "comp",
                "limiter",
                "gate",
                "expander",
                "transient",
                "dynamics",
            ],
        ) {
            categories.push("Dynamics".to_string());
        }

        // EQ & Filters
        if matches_any(
            &name,
            &uri,
            &["eq", "equalizer", "equaliser", "parametric", "graphic"],
        ) && !name.contains("frequency")
        // Avoid false positives
        {
            categories.push("EQ".to_string());
        }
        if matches_any(
            &name,
            &uri,
            &[
                "filter", "lowpass", "highpass", "bandpass", "notch", "shelf",
            ],
        ) {
            categories.push("Filter".to_string());
        }

        // Time-based
        if matches_any(
            &name,
            &uri,
            &[
                "reverb",
                "room",
                "hall",
                "plate",
                "spring",
                "convolution",
                "ambien",
            ],
        ) {
            categories.push("Reverb".to_string());
        }
        if matches_any(&name, &uri, &["delay", "echo", "tape"]) && !name.contains("envelope") {
            categories.push("Delay".to_string());
        }

        // Modulation
        if matches_any(
            &name,
            &uri,
            &[
                "chorus", "flanger", "phaser", "tremolo", "vibrato", "rotary", "leslie", "ensemble",
            ],
        ) {
            categories.push("Modulation".to_string());
        }

        // Distortion/Saturation
        if matches_any(
            &name,
            &uri,
            &[
                "distortion",
                "overdrive",
                "saturation",
                "fuzz",
                "amp",
                "cabinet",
                "cab",
                "tube",
                "valve",
                "clipper",
                "waveshap",
            ],
        ) {
            categories.push("Distortion".to_string());
        }

        // Pitch
        if matches_any(
            &name,
            &uri,
            &["pitch", "autotune", "harmonizer", "octaver", "shifter"],
        ) {
            categories.push("Pitch".to_string());
        }

        // Stereo/Imaging
        if matches_any(
            &name,
            &uri,
            &[
                "stereo", "widener", "imager", "pan", "mid-side", "m/s", "spatial",
            ],
        ) {
            categories.push("Imaging".to_string());
        }
    }

    if is_midi_fx {
        categories.push("MIDI Effects".to_string());

        if matches_any(&name, &uri, &["arpeggio", "arp"]) {
            categories.push("Arpeggiator".to_string());
        }
        if matches_any(&name, &uri, &["chord"]) {
            categories.push("Chord".to_string());
        }
    }

    // Utility (can be effect or standalone)
    if matches_any(
        &name,
        &uri,
        &[
            "utility", "gain", "volume", "meter", "analyzer", "analyser", "spectrum", "scope",
            "tuner", "monitor",
        ],
    ) {
        categories.push("Utility".to_string());
    }

    if is_analyzer {
        categories.push("Analyzer".to_string());
    }

    if is_generator {
        categories.push("Generator".to_string());
    }

    // Check LV2 categories in URI
    if uri.contains("/lv2/") {
        if uri.contains("instrument") {
            if !categories.contains(&"Instruments".to_string()) {
                categories.push("Instruments".to_string());
            }
        }
        if uri.contains("delay") && !categories.contains(&"Delay".to_string()) {
            categories.push("Delay".to_string());
        }
        if uri.contains("reverb") && !categories.contains(&"Reverb".to_string()) {
            categories.push("Reverb".to_string());
        }
        if uri.contains("dynamics") && !categories.contains(&"Dynamics".to_string()) {
            categories.push("Dynamics".to_string());
        }
        if uri.contains("filter") && !categories.contains(&"Filter".to_string()) {
            categories.push("Filter".to_string());
        }
        if uri.contains("distortion") && !categories.contains(&"Distortion".to_string()) {
            categories.push("Distortion".to_string());
        }
    }

    categories
}

fn matches_any(name: &str, uri: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| name.contains(p) || uri.contains(p))
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
