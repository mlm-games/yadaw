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

        // Prepare atom sequence inputs if needed (even if we don't have MIDI data)
        let mut empty_atom_input = LV2AtomSequence::new(&self.features, 1024);

        // Prepare atom sequence outputs if needed
        let mut atom_outputs = Vec::new();
        for _ in 0..self.port_counts.atom_sequence_outputs {
            atom_outputs.push(LV2AtomSequence::new(&self.features, 4096));
        }

        // Determine if we should use MIDI input or empty atom input
        let has_midi_data = self.midi_sequence.is_some()
            && !self.midi_sequence.as_ref().unwrap().iter().next().is_none();
        let needs_atom_input = self.port_counts.atom_sequence_inputs > 0;

        // Build the complete port connections based on what the plugin needs
        match (
            self.port_counts.audio_inputs,
            self.port_counts.audio_outputs,
            needs_atom_input,
            self.port_counts.atom_sequence_outputs,
        ) {
            // No audio I/O, no atom I/O
            (0, 0, false, 0) => {
                let ports = EmptyPortConnections::new();
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Mono in/out, no atom I/O
            (1, 1, false, 0) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio input only, no atom I/O
            (1, 0, false, 0) => {
                let ports = EmptyPortConnections::new().with_audio_inputs([input_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 0, false, 0) => {
                let ports =
                    EmptyPortConnections::new().with_audio_inputs([input_l, input_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio output only, no atom I/O
            (0, 1, false, 0) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new().with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (0, 2, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio I/O, no atom I/O
            (1, 2, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 1, false, 0) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 2, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // With atom sequence input but no atom outputs
            (0, 0, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &empty_atom_input
                };
                let ports =
                    EmptyPortConnections::new().with_atom_sequence_inputs(std::iter::once(atom_in));
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (0, 2, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &empty_atom_input
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 1, true, 0) => {
                output_r.copy_from_slice(output_l);
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &empty_atom_input
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 2, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &empty_atom_input
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 1, true, 0) => {
                output_r.copy_from_slice(output_l);
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &empty_atom_input
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 2, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &empty_atom_input
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Cases with atom outputs - handle separately
            (_, _, _, atom_outs) if atom_outs > 0 => {
                self.run_with_atom_outputs(
                    input_l,
                    input_r,
                    output_l,
                    output_r,
                    samples,
                    &mut atom_outputs,
                    if needs_atom_input {
                        Some(&empty_atom_input)
                    } else {
                        None
                    },
                    has_midi_data,
                )?;
            }
            _ => {
                return Err(anyhow!(
                    "Unsupported port configuration: {} inputs, {} outputs, {} atom outputs",
                    self.port_counts.audio_inputs,
                    self.port_counts.audio_outputs,
                    self.port_counts.atom_sequence_outputs
                ));
            }
        }

        Ok(())
    }

    // helper method to handle atom inputs
    fn run_with_atom_outputs(
        &mut self,
        input_l: &[f32],
        input_r: &[f32],
        output_l: &mut [f32],
        output_r: &mut [f32],
        samples: usize,
        atom_outputs: &mut Vec<LV2AtomSequence>,
        empty_atom_input: Option<&LV2AtomSequence>,
        has_midi_data: bool,
    ) -> Result<()> {
        let needs_atom_input = self.port_counts.atom_sequence_inputs > 0;
        let default_atom_sequence = LV2AtomSequence::new(&self.features, 0);

        // Get the atom input to use
        let atom_in = if needs_atom_input {
            if has_midi_data && self.midi_sequence.is_some() {
                self.midi_sequence.as_ref().unwrap()
            } else {
                empty_atom_input.ok_or_else(|| anyhow!("Need atom input but none provided"))?
            }
        } else {
            empty_atom_input.unwrap_or(&default_atom_sequence)
        };

        // Handle all combinations by building the complete port connections
        match (
            self.port_counts.audio_inputs,
            self.port_counts.audio_outputs,
            needs_atom_input,
        ) {
            // No audio, no MIDI, only atom outputs
            (0, 0, false) => {
                let ports =
                    EmptyPortConnections::new().with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio input only
            (1, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio output only
            (0, 1, false) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([output_l].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (0, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // Audio I/O combinations
            (1, 1, false) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 1, false) => {
                output_r.copy_from_slice(output_l);
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            // With MIDI input
            (0, 0, true) => {
                if let Some(midi_seq) = &self.midi_sequence {
                    let ports = EmptyPortConnections::new()
                        .with_atom_sequence_inputs(std::iter::once(midi_seq))
                        .with_atom_sequence_outputs(atom_outputs.iter_mut());
                    unsafe {
                        self.instance.run(samples, ports)?;
                    }
                }
            }
            (0, 2, true) => {
                if let Some(midi_seq) = &self.midi_sequence {
                    let ports = EmptyPortConnections::new()
                        .with_atom_sequence_inputs(std::iter::once(midi_seq))
                        .with_audio_outputs([output_l, output_r].into_iter())
                        .with_atom_sequence_outputs(atom_outputs.iter_mut());
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
                        .with_audio_outputs([output_l, output_r].into_iter())
                        .with_atom_sequence_outputs(atom_outputs.iter_mut());
                    unsafe {
                        self.instance.run(samples, ports)?;
                    }
                }
            }
            (2, 2, true) => {
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            _ => {
                return Err(anyhow!(
                    "Unsupported port configuration with atom outputs: {} inputs, {} outputs",
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
