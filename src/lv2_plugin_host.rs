use anyhow::{Result, anyhow};
use dashmap::DashMap;
use livi::event::LV2AtomSequence;
use livi::{
    EmptyPortConnections, Features, FeaturesBuilder, Instance, PortCounts, PortType, World,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub uri: String,
    pub name: String,
    pub is_instrument: bool,
    pub audio_inputs: usize,
    pub audio_outputs: usize,
    pub has_midi: bool,
    pub control_ports: Vec<ControlPortInfo>,
}

#[derive(Clone, Debug)]
pub struct ControlPortInfo {
    pub index: usize,
    pub name: String,
    pub symbol: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

pub struct LV2PluginHost {
    world: World,
    features: Arc<Features>,
    available_plugins: Vec<PluginInfo>,
    sample_rate: f64,
}

impl LV2PluginHost {
    pub fn new(sample_rate: f64, max_block_size: usize) -> Result<Self> {
        let world = World::new();

        // Build features with your audio engine's requirements
        let features = world.build_features(FeaturesBuilder {
            min_block_length: 1,
            max_block_length: max_block_size,
        });

        // Collect available plugins
        let available_plugins: Vec<PluginInfo> = world
            .iter_plugins()
            .map(|plugin| {
                let port_counts = plugin.port_counts();

                // Collect control port info
                let control_ports = plugin
                    .ports()
                    .filter(|p| p.port_type == PortType::ControlInput)
                    .map(|p| ControlPortInfo {
                        index: p.index.0,
                        name: p.name.clone(),
                        symbol: p.symbol.clone(),
                        default: p.default_value,
                        min: p.min_value.unwrap_or(0.0),
                        max: p.max_value.unwrap_or(1.0),
                    })
                    .collect();

                PluginInfo {
                    uri: plugin.uri(),
                    name: plugin.name(),
                    is_instrument: plugin.is_instrument(),
                    audio_inputs: port_counts.audio_inputs,
                    audio_outputs: port_counts.audio_outputs,
                    has_midi: port_counts.atom_sequence_inputs > 0,
                    control_ports,
                }
            })
            .collect();

        Ok(Self {
            world,
            features,
            available_plugins,
            sample_rate,
        })
    }

    pub fn get_available_plugins(&self) -> &[PluginInfo] {
        &self.available_plugins
    }

    pub fn instantiate_plugin(&self, uri: &str) -> Result<LV2PluginInstance> {
        let plugin = self
            .world
            .plugin_by_uri(uri)
            .ok_or_else(|| anyhow!("Plugin not found: {}", uri))?;

        let instance = unsafe {
            plugin
                .instantiate(self.features.clone(), self.sample_rate)
                .map_err(|_| anyhow!("Failed to instantiate plugin"))?
        };

        // Initialize parameter cache and control port mapping
        let params = Arc::new(DashMap::new());
        let mut control_port_indices = HashMap::new();

        for port in plugin.ports() {
            if port.port_type == PortType::ControlInput {
                params.insert(port.symbol.clone(), port.default_value);
                control_port_indices.insert(port.symbol.clone(), port.index);
            }
        }

        Ok(LV2PluginInstance {
            instance,
            features: self.features.clone(),
            port_counts: *plugin.port_counts(),
            params,
            midi_sequence: None,
            control_port_indices,
        })
    }
}

pub struct LV2PluginInstance {
    instance: Instance,
    features: Arc<Features>,
    port_counts: PortCounts,
    params: Arc<DashMap<String, f32>>,
    midi_sequence: Option<LV2AtomSequence>,
    control_port_indices: HashMap<String, livi::PortIndex>,
}

impl LV2PluginInstance {
    pub fn process(
        &mut self,
        input_l: &[f32],
        input_r: &[f32],
        output_l: &mut [f32],
        output_r: &mut [f32],
        samples: usize,
    ) -> Result<()> {
        // Update control parameters using our mapping
        for entry in self.params.iter() {
            let symbol = entry.key();
            let value = *entry.value();
            if let Some(&port_index) = self.control_port_indices.get(symbol) {
                self.instance.set_control_input(port_index, value);
            }
        }

        // Build the complete port connections based on what the plugin needs
        match (
            self.port_counts.audio_inputs,
            self.port_counts.audio_outputs,
            self.port_counts.atom_sequence_inputs > 0 && self.midi_sequence.is_some(),
        ) {
            // No audio I/O, no MIDI
            (0, 0, false) => {
                let ports = EmptyPortConnections::new();
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio input only, no MIDI
            (1, 0, false) => {
                let ports = EmptyPortConnections::new().with_audio_inputs([input_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 0, false) => {
                let ports =
                    EmptyPortConnections::new().with_audio_inputs([input_l, input_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio output only, no MIDI
            (0, 1, false) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new().with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (0, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio I/O, no MIDI
            (1, 1, false) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 1, false) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // With MIDI
            (0, 0, true) => {
                if let Some(midi_seq) = &self.midi_sequence {
                    let ports = EmptyPortConnections::new()
                        .with_atom_sequence_inputs(std::iter::once(midi_seq));
                    unsafe {
                        self.instance.run(samples, ports)?;
                    }
                }
            }
            (0, 2, true) => {
                if let Some(midi_seq) = &self.midi_sequence {
                    let ports = EmptyPortConnections::new()
                        .with_atom_sequence_inputs(std::iter::once(midi_seq))
                        .with_audio_outputs([output_l, output_r].into_iter());
                    unsafe {
                        self.instance.run(samples, ports)?;
                    }
                }
            }
            (1, 2, true) => {
                if let Some(midi_seq) = &self.midi_sequence {
                    let ports = EmptyPortConnections::new()
                        .with_atom_sequence_inputs(std::iter::once(midi_seq))
                        .with_audio_inputs([input_l].into_iter())
                        .with_audio_outputs([output_l, output_r].into_iter());
                    unsafe {
                        self.instance.run(samples, ports)?;
                    }
                }
            }
            (2, 2, true) => {
                if let Some(midi_seq) = &self.midi_sequence {
                    let ports = EmptyPortConnections::new()
                        .with_atom_sequence_inputs(std::iter::once(midi_seq))
                        .with_audio_inputs([input_l, input_r].into_iter())
                        .with_audio_outputs([output_l, output_r].into_iter());
                    unsafe {
                        self.instance.run(samples, ports)?;
                    }
                }
            }
            _ => {
                return Err(anyhow!(
                    "Unsupported port configuration: {} inputs, {} outputs",
                    self.port_counts.audio_inputs,
                    self.port_counts.audio_outputs
                ));
            }
        }

        Ok(())
    }

    pub fn set_parameter(&mut self, symbol: &str, value: f32) {
        self.params.insert(symbol.to_string(), value);
    }

    pub fn get_parameter(&self, symbol: &str) -> Option<f32> {
        self.params.get(symbol).map(|v| *v)
    }

    pub fn prepare_midi_events(&mut self, notes: &[(u8, u8, i64)]) {
        // notes: Vec of (pitch, velocity, time_in_frames)
        let mut sequence = LV2AtomSequence::new(&self.features, 4096);

        for &(pitch, velocity, time) in notes {
            let midi_data = [0x90, pitch, velocity]; // Note On
            let _ = sequence.push_midi_event::<3>(time, self.features.midi_urid(), &midi_data);
        }

        self.midi_sequence = Some(sequence);
    }

    pub fn clear_midi_events(&mut self) {
        if let Some(seq) = &mut self.midi_sequence {
            seq.clear();
        }
    }

    pub fn get_params(&self) -> Arc<DashMap<String, f32>> {
        self.params.clone()
    }
}
