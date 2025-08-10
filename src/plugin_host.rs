use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PluginInstance {
    pub id: usize,
    pub name: String,
    pub plugin_type: PluginType,
    pub parameters: Vec<PluginParameter>,
    pub presets: Vec<PluginPreset>,
    pub current_preset: Option<usize>,
    pub bypass: bool,
    pub wet_dry_mix: f32,
}

#[derive(Debug, Clone)]
pub enum PluginType {
    Instrument,
    Effect,
    MidiEffect,
    Analyzer,
}

#[derive(Debug, Clone)]
pub struct PluginParameter {
    pub id: usize,
    pub name: String,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: String,
    pub is_automatable: bool,
    pub display_format: ParameterDisplayFormat,
}

#[derive(Debug, Clone)]
pub enum ParameterDisplayFormat {
    Linear,
    Logarithmic,
    Percentage,
    Decibel,
    Frequency,
    Time,
    OnOff,
}

#[derive(Debug, Clone)]
pub struct PluginPreset {
    pub name: String,
    pub parameters: HashMap<usize, f32>,
    pub metadata: PresetMetadata,
}

#[derive(Debug, Clone)]
pub struct PresetMetadata {
    pub author: String,
    pub tags: Vec<String>,
    pub description: String,
}

pub struct PluginHost {
    plugins: HashMap<usize, Arc<RwLock<PluginInstance>>>,
    next_plugin_id: usize,
    available_plugins: Vec<PluginDescriptor>,
}

#[derive(Debug, Clone)]
pub struct PluginDescriptor {
    pub name: String,
    pub manufacturer: String,
    pub plugin_type: PluginType,
    pub unique_id: String,
    pub version: String,
    pub path: std::path::PathBuf,
}

impl PluginHost {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            next_plugin_id: 0,
            available_plugins: Self::scan_plugins(),
        }
    }

    fn scan_plugins() -> Vec<PluginDescriptor> {
        // In a real implementation, this would scan for VST/AU/LV2 plugins
        // For now, we'll create some built-in effect descriptors
        vec![
            PluginDescriptor {
                name: "Reverb".to_string(),
                manufacturer: "YADAW".to_string(),
                plugin_type: PluginType::Effect,
                unique_id: "yadaw.reverb".to_string(),
                version: "1.0.0".to_string(),
                path: std::path::PathBuf::new(),
            },
            PluginDescriptor {
                name: "Delay".to_string(),
                manufacturer: "YADAW".to_string(),
                plugin_type: PluginType::Effect,
                unique_id: "yadaw.delay".to_string(),
                version: "1.0.0".to_string(),
                path: std::path::PathBuf::new(),
            },
            PluginDescriptor {
                name: "Compressor".to_string(),
                manufacturer: "YADAW".to_string(),
                plugin_type: PluginType::Effect,
                unique_id: "yadaw.compressor".to_string(),
                version: "1.0.0".to_string(),
                path: std::path::PathBuf::new(),
            },
            PluginDescriptor {
                name: "EQ".to_string(),
                manufacturer: "YADAW".to_string(),
                plugin_type: PluginType::Effect,
                unique_id: "yadaw.eq".to_string(),
                version: "1.0.0".to_string(),
                path: std::path::PathBuf::new(),
            },
        ]
    }

    pub fn load_plugin(&mut self, descriptor: &PluginDescriptor) -> Result<usize, String> {
        let plugin_id = self.next_plugin_id;
        self.next_plugin_id += 1;

        let plugin = match descriptor.unique_id.as_str() {
            "yadaw.reverb" => self.create_reverb_plugin(plugin_id),
            "yadaw.delay" => self.create_delay_plugin(plugin_id),
            "yadaw.compressor" => self.create_compressor_plugin(plugin_id),
            "yadaw.eq" => self.create_eq_plugin(plugin_id),
            _ => return Err("Unknown plugin".to_string()),
        };

        self.plugins
            .insert(plugin_id, Arc::new(RwLock::new(plugin)));
        Ok(plugin_id)
    }

    fn create_reverb_plugin(&self, id: usize) -> PluginInstance {
        PluginInstance {
            id,
            name: "Reverb".to_string(),
            plugin_type: PluginType::Effect,
            parameters: vec![
                PluginParameter {
                    id: 0,
                    name: "Room Size".to_string(),
                    value: 0.5,
                    min: 0.0,
                    max: 1.0,
                    default: 0.5,
                    unit: "".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Percentage,
                },
                PluginParameter {
                    id: 1,
                    name: "Damping".to_string(),
                    value: 0.5,
                    min: 0.0,
                    max: 1.0,
                    default: 0.5,
                    unit: "".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Percentage,
                },
                PluginParameter {
                    id: 2,
                    name: "Wet Level".to_string(),
                    value: 0.3,
                    min: 0.0,
                    max: 1.0,
                    default: 0.3,
                    unit: "".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Percentage,
                },
                PluginParameter {
                    id: 3,
                    name: "Dry Level".to_string(),
                    value: 0.7,
                    min: 0.0,
                    max: 1.0,
                    default: 0.7,
                    unit: "".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Percentage,
                },
            ],
            presets: vec![
                PluginPreset {
                    name: "Small Room".to_string(),
                    parameters: HashMap::from([(0, 0.2), (1, 0.7), (2, 0.2), (3, 0.8)]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["room".to_string(), "small".to_string()],
                        description: "Small room reverb".to_string(),
                    },
                },
                PluginPreset {
                    name: "Large Hall".to_string(),
                    parameters: HashMap::from([(0, 0.9), (1, 0.3), (2, 0.4), (3, 0.6)]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["hall".to_string(), "large".to_string()],
                        description: "Large concert hall reverb".to_string(),
                    },
                },
            ],
            current_preset: None,
            bypass: false,
            wet_dry_mix: 1.0,
        }
    }

    fn create_delay_plugin(&self, id: usize) -> PluginInstance {
        PluginInstance {
            id,
            name: "Delay".to_string(),
            plugin_type: PluginType::Effect,
            parameters: vec![
                PluginParameter {
                    id: 0,
                    name: "Delay Time".to_string(),
                    value: 0.25,
                    min: 0.001,
                    max: 2.0,
                    default: 0.25,
                    unit: "s".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Time,
                },
                PluginParameter {
                    id: 1,
                    name: "Feedback".to_string(),
                    value: 0.5,
                    min: 0.0,
                    max: 0.99,
                    default: 0.5,
                    unit: "".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Percentage,
                },
                PluginParameter {
                    id: 2,
                    name: "Mix".to_string(),
                    value: 0.3,
                    min: 0.0,
                    max: 1.0,
                    default: 0.3,
                    unit: "".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Percentage,
                },
            ],
            presets: vec![
                PluginPreset {
                    name: "Slapback".to_string(),
                    parameters: HashMap::from([(0, 0.1), (1, 0.2), (2, 0.3)]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["short".to_string(), "slapback".to_string()],
                        description: "Short slapback delay".to_string(),
                    },
                },
                PluginPreset {
                    name: "Echo".to_string(),
                    parameters: HashMap::from([(0, 0.5), (1, 0.6), (2, 0.4)]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["echo".to_string(), "long".to_string()],
                        description: "Long echo effect".to_string(),
                    },
                },
            ],
            current_preset: None,
            bypass: false,
            wet_dry_mix: 1.0,
        }
    }

    fn create_compressor_plugin(&self, id: usize) -> PluginInstance {
        PluginInstance {
            id,
            name: "Compressor".to_string(),
            plugin_type: PluginType::Effect,
            parameters: vec![
                PluginParameter {
                    id: 0,
                    name: "Threshold".to_string(),
                    value: -20.0,
                    min: -60.0,
                    max: 0.0,
                    default: -20.0,
                    unit: "dB".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Decibel,
                },
                PluginParameter {
                    id: 1,
                    name: "Ratio".to_string(),
                    value: 4.0,
                    min: 1.0,
                    max: 20.0,
                    default: 4.0,
                    unit: ":1".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Linear,
                },
                PluginParameter {
                    id: 2,
                    name: "Attack".to_string(),
                    value: 0.01,
                    min: 0.001,
                    max: 0.1,
                    default: 0.01,
                    unit: "s".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Time,
                },
                PluginParameter {
                    id: 3,
                    name: "Release".to_string(),
                    value: 0.1,
                    min: 0.01,
                    max: 1.0,
                    default: 0.1,
                    unit: "s".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Time,
                },
                PluginParameter {
                    id: 4,
                    name: "Makeup Gain".to_string(),
                    value: 0.0,
                    min: -20.0,
                    max: 20.0,
                    default: 0.0,
                    unit: "dB".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Decibel,
                },
            ],
            presets: vec![
                PluginPreset {
                    name: "Gentle".to_string(),
                    parameters: HashMap::from([
                        (0, -30.0),
                        (1, 2.0),
                        (2, 0.02),
                        (3, 0.2),
                        (4, 2.0),
                    ]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["gentle".to_string(), "subtle".to_string()],
                        description: "Gentle compression".to_string(),
                    },
                },
                PluginPreset {
                    name: "Punch".to_string(),
                    parameters: HashMap::from([
                        (0, -15.0),
                        (1, 8.0),
                        (2, 0.005),
                        (3, 0.05),
                        (4, 5.0),
                    ]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["punch".to_string(), "aggressive".to_string()],
                        description: "Punchy compression".to_string(),
                    },
                },
            ],
            current_preset: None,
            bypass: false,
            wet_dry_mix: 1.0,
        }
    }

    fn create_eq_plugin(&self, id: usize) -> PluginInstance {
        PluginInstance {
            id,
            name: "EQ".to_string(),
            plugin_type: PluginType::Effect,
            parameters: vec![
                PluginParameter {
                    id: 0,
                    name: "Low Freq".to_string(),
                    value: 100.0,
                    min: 20.0,
                    max: 500.0,
                    default: 100.0,
                    unit: "Hz".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Frequency,
                },
                PluginParameter {
                    id: 1,
                    name: "Low Gain".to_string(),
                    value: 0.0,
                    min: -12.0,
                    max: 12.0,
                    default: 0.0,
                    unit: "dB".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Decibel,
                },
                PluginParameter {
                    id: 2,
                    name: "Mid Freq".to_string(),
                    value: 1000.0,
                    min: 200.0,
                    max: 5000.0,
                    default: 1000.0,
                    unit: "Hz".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Frequency,
                },
                PluginParameter {
                    id: 3,
                    name: "Mid Gain".to_string(),
                    value: 0.0,
                    min: -12.0,
                    max: 12.0,
                    default: 0.0,
                    unit: "dB".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Decibel,
                },
                PluginParameter {
                    id: 4,
                    name: "High Freq".to_string(),
                    value: 8000.0,
                    min: 2000.0,
                    max: 20000.0,
                    default: 8000.0,
                    unit: "Hz".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Frequency,
                },
                PluginParameter {
                    id: 5,
                    name: "High Gain".to_string(),
                    value: 0.0,
                    min: -12.0,
                    max: 12.0,
                    default: 0.0,
                    unit: "dB".to_string(),
                    is_automatable: true,
                    display_format: ParameterDisplayFormat::Decibel,
                },
            ],
            presets: vec![
                PluginPreset {
                    name: "Bright".to_string(),
                    parameters: HashMap::from([(1, 0.0), (3, 2.0), (5, 4.0)]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["bright".to_string(), "presence".to_string()],
                        description: "Adds brightness and presence".to_string(),
                    },
                },
                PluginPreset {
                    name: "Warm".to_string(),
                    parameters: HashMap::from([(1, 3.0), (3, 1.0), (5, -2.0)]),
                    metadata: PresetMetadata {
                        author: "YADAW".to_string(),
                        tags: vec!["warm".to_string(), "vintage".to_string()],
                        description: "Warm, vintage tone".to_string(),
                    },
                },
            ],
            current_preset: None,
            bypass: false,
            wet_dry_mix: 1.0,
        }
    }

    pub fn get_plugin(&self, id: usize) -> Option<Arc<RwLock<PluginInstance>>> {
        self.plugins.get(&id).cloned()
    }

    pub fn remove_plugin(&mut self, id: usize) -> Option<Arc<RwLock<PluginInstance>>> {
        self.plugins.remove(&id)
    }

    pub fn get_available_plugins(&self) -> &[PluginDescriptor] {
        &self.available_plugins
    }

    pub fn set_parameter(&self, plugin_id: usize, param_id: usize, value: f32) {
        if let Some(plugin) = self.plugins.get(&plugin_id) {
            let mut plugin = plugin.write();
            if let Some(param) = plugin.parameters.iter_mut().find(|p| p.id == param_id) {
                param.value = value.clamp(param.min, param.max);
            }
        }
    }

    pub fn load_preset(&self, plugin_id: usize, preset_index: usize) {
        if let Some(plugin) = self.plugins.get(&plugin_id) {
            let mut plugin = plugin.write();
            if let Some(preset) = plugin.presets.get(preset_index) {
                for (param_id, value) in &preset.parameters {
                    if let Some(param) = plugin
                        .parameters
                        .clone()
                        .iter_mut()
                        .find(|p| p.id == *param_id)
                    {
                        param.value = *value;
                    }
                }
                plugin.current_preset = Some(preset_index);
            }
        }
    }
}
