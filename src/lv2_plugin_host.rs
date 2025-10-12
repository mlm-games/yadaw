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
    max_block_size: usize,
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
            max_block_size,
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

        if cfg!(debug_assertions) {
            println!(
                "[LV2][instantiate] {} | audio_in={} audio_out={} atom_in={} atom_out={}",
                plugin.uri(),
                plugin.port_counts().audio_inputs,
                plugin.port_counts().audio_outputs,
                plugin.port_counts().atom_sequence_inputs,
                plugin.port_counts().atom_sequence_outputs
            );
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

            max_block_size: self.max_block_size,
            silent_audio: vec![0.0; self.max_block_size],
            scratch_audio_out: Vec::new(), // will be sized lazily but not allocated per block
        })
    }
}

#[derive(Debug)]

pub struct LV2PluginInstance {
    instance: Instance,
    features: Arc<Features>,
    port_counts: PortCounts,
    params: Arc<DashMap<String, f32>>,
    pub(crate) midi_sequence: Option<LV2AtomSequence>,
    control_port_indices: HashMap<String, livi::PortIndex>,

    // Pre-allocated for RT safety
    empty_atom_in: LV2AtomSequence,
    atom_outputs: Vec<LV2AtomSequence>,

    max_block_size: usize,
    silent_audio: Vec<f32>,
    scratch_audio_out: Vec<Vec<f32>>,
}

impl LV2PluginInstance {
    pub fn audio_in_out_counts(&self) -> (usize, usize) {
        (
            self.port_counts.audio_inputs,
            self.port_counts.audio_outputs,
        )
    }
    pub fn has_midi_events(&self) -> bool {
        self.midi_sequence
            .as_ref()
            .map(|s| s.iter().next().is_some())
            .unwrap_or(false)
    }

    // Ensure we have N scratch output buffers (each max_block_size long) and zero first `samples`
    fn ensure_scratch_out(&mut self, n: usize, samples: usize) {
        if self.scratch_audio_out.len() < n {
            let add = n - self.scratch_audio_out.len();
            self.scratch_audio_out
                .extend((0..add).map(|_| vec![0.0; self.max_block_size]));
        }
        for buf in self.scratch_audio_out.iter_mut().take(n) {
            buf[..samples.min(self.max_block_size)].fill(0.0);
        }
    }

    pub fn process_multi(
        &mut self,
        audio_inputs: &[&[f32]],
        audio_outputs: &mut [&mut [f32]],
        use_midi: bool,
        samples: usize,
    ) -> Result<()> {
        let len = samples.min(self.max_block_size);
        let need_ai = self.port_counts.audio_inputs;
        let need_ao = self.port_counts.audio_outputs;
        let ai = self.port_counts.atom_sequence_inputs;

        // 1) Apply control parameters
        for entry in self.params.iter() {
            if let Some(&pi) = self.control_port_indices.get(entry.key()) {
                self.instance.set_control_input(pi, *entry.value());
            }
        }

        let mut atom_outputs = std::mem::take(&mut self.atom_outputs);
        let mut scratch_audio = std::mem::take(&mut self.scratch_audio_out);

        // Clear atom outputs
        for seq in atom_outputs.iter_mut() {
            seq.clear_as_chunk();
        }

        // 3) Build audio input refs (read-only, no conflicts)
        let mut in_refs: Vec<&[f32]> = Vec::with_capacity(need_ai);
        for i in 0..need_ai {
            let src = audio_inputs
                .get(i)
                .copied()
                .unwrap_or(&self.silent_audio[..len]);
            in_refs.push(&src[..len.min(src.len())]);
        }

        // 4) Ensure scratch buffers exist and are zeroed
        if scratch_audio.len() < need_ao {
            scratch_audio.resize(need_ao, vec![0.0; self.max_block_size]);
        }
        for buf in scratch_audio.iter_mut().take(need_ao) {
            buf[..len].fill(0.0);
        }

        let provided_count = audio_outputs.len().min(need_ao);
        let mut out_refs: Vec<&mut [f32]> = Vec::with_capacity(need_ao);

        // Split audio_outputs to avoid aliasing
        if provided_count > 0 {
            // Use split_at_mut to safely get non-overlapping slices
            let (provided, _rest) = audio_outputs.split_at_mut(provided_count);
            for out_buf in provided.iter_mut() {
                let l = out_buf.len().min(len);
                out_refs.push(&mut out_buf[..l]);
            }
        }

        // Add scratch buffers for remaining outputs
        let extra_needed = need_ao.saturating_sub(provided_count);
        for buf in scratch_audio.iter_mut().take(extra_needed) {
            out_refs.push(&mut buf[..len]);
        }

        // 6) Build atom input refs
        let atom_in_refs: Vec<&LV2AtomSequence> = if ai > 0 {
            let atom = if use_midi && self.has_midi_events() {
                self.midi_sequence.as_ref().unwrap()
            } else {
                &self.empty_atom_in
            };
            vec![atom; ai]
        } else {
            Vec::new()
        };

        // 7) Build ports in ONE expression (no intermediate borrows)
        let ports = livi::EmptyPortConnections::new()
            .with_audio_inputs(in_refs.into_iter())
            .with_audio_outputs(out_refs.into_iter())
            .with_atom_sequence_inputs(atom_in_refs.into_iter())
            .with_atom_sequence_outputs(atom_outputs.iter_mut());

        // 8) Run plugin
        let result = unsafe {
            self.instance
                .run(samples, ports)
                .map_err(|e| anyhow!("[LV2] run() error: {}", e))
        };

        // 9) Move fields back into self
        self.atom_outputs = atom_outputs;
        self.scratch_audio_out = scratch_audio;

        result
    }

    // Keep stereo convenience; auto-enable MIDI if events exist
    pub fn process(
        &mut self,
        input_l: &[f32],
        input_r: &[f32],
        output_l: &mut [f32],
        output_r: &mut [f32],
        samples: usize,
    ) -> Result<()> {
        let ins: [&[f32]; 2] = [input_l, input_r];
        let mut outs: [&mut [f32]; 2] = [output_l, output_r];
        let use_midi = self.has_midi_events() && self.port_counts.atom_sequence_inputs > 0;
        self.process_multi(&ins, &mut outs, use_midi, samples)
    }

    pub fn set_parameter(&mut self, symbol: &str, value: f32) {
        self.params.insert(symbol.to_string(), value);
    }

    pub fn get_parameter(&self, symbol: &str) -> Option<f32> {
        self.params.get(symbol).map(|v| *v)
    }

    pub fn prepare_midi_raw_events(&mut self, events: &[(u8, u8, u8, i64)]) {
        // Reuse pre-allocated sequence
        let mut sequence = self
            .midi_sequence
            .take()
            .unwrap_or_else(|| LV2AtomSequence::new(&self.features, 4096));
        sequence.clear();

        for &(status, data1, data2, time_in_frames) in events {
            let midi_data = [status, data1, data2];
            let _ = sequence.push_midi_event::<3>(
                time_in_frames,
                self.features.midi_urid(),
                &midi_data,
            );
        }

        self.midi_sequence = Some(sequence);
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

    pub fn set_params_arc(&mut self, params: Arc<DashMap<String, f32>>) {
        self.params = params;
    }
}
