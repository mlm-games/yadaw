use anyhow::{anyhow, Result};
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
            midi_sequence: Some(LV2AtomSequence::new(&self.features, 4096)),
            control_port_indices,
            empty_atom_in: LV2AtomSequence::new(&self.features, 1024),
            atom_outputs: (0..plugin.port_counts().atom_sequence_outputs)
                .map(|_| LV2AtomSequence::new(&self.features, 4096))
                .collect(),
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

    // Pre-allocated for RT safety
    empty_atom_in: LV2AtomSequence,
    atom_outputs: Vec<LV2AtomSequence>,
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
        // 1) Apply control inputs (param cache)
        for entry in self.params.iter() {
            if let Some(&port_index) = self.control_port_indices.get(entry.key()) {
                self.instance.set_control_input(port_index, *entry.value());
            }
        }

        // 2) Reset per-block atom buffers (RT-safe reuse)
        self.empty_atom_in.clear();
        for seq in self.atom_outputs.iter_mut() {
            seq.clear_as_chunk();
        }

        let has_midi_data = self
            .midi_sequence
            .as_ref()
            .map(|s| s.iter().next().is_some())
            .unwrap_or(false);
        let needs_atom_input = self.port_counts.atom_sequence_inputs > 0;

        // 3) Build ports and run (no pre-run output mirroring)
        match (
            self.port_counts.audio_inputs,
            self.port_counts.audio_outputs,
            needs_atom_input,
            self.port_counts.atom_sequence_outputs,
        ) {
            // No audio/atom I/O
            (0, 0, false, 0) => {
                let ports = EmptyPortConnections::new();
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }

            // Audio I/O without atom
            (1, 1, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l));
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                // Mirror mono to right only after successful run
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
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
            (0, 1, false, 0) => {
                let ports =
                    EmptyPortConnections::new().with_audio_outputs(std::iter::once(&mut *output_l));
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
            (0, 2, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 2, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 1, false, 0) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l));
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
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

            // With atom input and no atom outputs: always pass an atom in (empty if no MIDI)
            (0, 0, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &self.empty_atom_in
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
                    &self.empty_atom_in
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 1, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &self.empty_atom_in
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l));
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
            (1, 2, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &self.empty_atom_in
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
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &self.empty_atom_in
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l));
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
            (2, 2, true, 0) => {
                let atom_in = if has_midi_data {
                    self.midi_sequence.as_ref().unwrap()
                } else {
                    &self.empty_atom_in
                };
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }

            // With atom outputs
            (_, _, _, atom_outs) if atom_outs > 0 => {
                self.run_with_atom_outputs(
                    input_l,
                    input_r,
                    output_l,
                    output_r,
                    samples,
                    needs_atom_input,
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

    fn run_with_atom_outputs(
        &mut self,
        input_l: &[f32],
        input_r: &[f32],
        output_l: &mut [f32],
        output_r: &mut [f32],
        samples: usize,
        needs_atom_input: bool,
        has_midi_data: bool,
    ) -> Result<()> {
        // Ensure chunk state for outputs
        for seq in self.atom_outputs.iter_mut() {
            seq.clear_as_chunk();
        }

        let atom_in_opt = if needs_atom_input {
            if has_midi_data {
                self.midi_sequence.as_ref()
            } else {
                Some(&self.empty_atom_in)
            }
        } else {
            None
        };

        match (
            self.port_counts.audio_inputs,
            self.port_counts.audio_outputs,
            needs_atom_input,
        ) {
            (0, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (0, 1, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs(std::iter::once(&mut *output_l))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
            (0, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 1, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
            (1, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 1, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
                if self.port_counts.audio_outputs == 1 {
                    output_r[..samples].copy_from_slice(&output_l[..samples]);
                }
            }
            (2, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }

            // With MIDI input
            (0, 0, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (0, 2, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (1, 2, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    self.instance.run(samples, ports)?;
                }
            }
            (2, 2, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([output_l, output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
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
        // Reuse pre-allocated LV2AtomSequence
        let mut sequence = self
            .midi_sequence
            .take()
            .unwrap_or_else(|| LV2AtomSequence::new(&self.features, 4096));
        sequence.clear();

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
