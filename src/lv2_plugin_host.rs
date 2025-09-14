use anyhow::{Result, anyhow};
use dashmap::DashMap;
use livi::event::LV2AtomSequence;
use livi::{
    EmptyPortConnections, Features, FeaturesBuilder, Instance, PortCounts, PortType, World,
};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;

use crate::constants::MAX_BUFFER_SIZE;

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
        // 1) Apply control inputs
        for entry in self.params.iter() {
            if let Some(&pi) = self.control_port_indices.get(entry.key()) {
                self.instance.set_control_input(pi, *entry.value());
            }
        }

        // 2) Prepare lengths and take mutable self fields out to avoid aliasing
        let need_ai = self.port_counts.audio_inputs;
        let need_ao = self.port_counts.audio_outputs;
        let ai = self.port_counts.atom_sequence_inputs;
        let len = samples.min(self.max_block_size);

        // Take atom outputs out of self so we can borrow scratch and atom outs independently
        let mut atom_outs = std::mem::take(&mut self.atom_outputs);
        for seq in atom_outs.iter_mut() {
            seq.clear_as_chunk();
        }

        // 3) Audio inputs (pad with silence)
        let mut in_refs: Vec<&[f32]> = Vec::with_capacity(need_ai);
        for i in 0..need_ai {
            let src = audio_inputs
                .get(i)
                .copied()
                .unwrap_or(&self.silent_audio[..len]);
            in_refs.push(&src[..len.min(src.len())]);
        }

        // 4) Zero any provided outputs the plugin will not write to (beyond need_ao)
        if audio_outputs.len() > need_ao {
            for extra in &mut audio_outputs[need_ao..] {
                let l = extra.len().min(len);
                extra[..l].fill(0.0);
            }
        }

        // 5) Audio outputs: use provided ones first, then scratch for extras
        let take_n = need_ao.min(audio_outputs.len());
        let mut out_refs: Vec<&mut [f32]> = Vec::with_capacity(need_ao);

        // Provided outputs first (no indexing, use iter_mut)
        {
            let mut it = audio_outputs.iter_mut().take(take_n);
            for _ in 0..take_n {
                if let Some(dst) = it.next() {
                    let l = dst.len().min(len);
                    out_refs.push(&mut dst[..l]);
                }
            }
        }

        // Scratch for extras
        let extra_out = need_ao - take_n;
        let mut scratch = std::mem::take(&mut self.scratch_audio_out);

        if extra_out > 0 {
            if scratch.len() < extra_out {
                scratch.extend(
                    (0..(extra_out - scratch.len())).map(|_| vec![0.0; self.max_block_size]),
                );
            }
            let (head, _) = scratch.split_at_mut(extra_out);
            for buf in head.iter_mut() {
                buf[..len].fill(0.0);
                out_refs.push(&mut buf[..len]);
            }
        }

        // 6) Atom inputs: build exact-length iterator after we’re done with other mut borrows
        let mut atom_in_vec: Vec<&LV2AtomSequence> = Vec::with_capacity(ai);
        if ai > 0 {
            let atom = if use_midi && self.has_midi_events() {
                self.midi_sequence.as_ref().unwrap()
            } else {
                &self.empty_atom_in
            };
            for _ in 0..ai {
                atom_in_vec.push(atom);
            }
        }

        // 7) Build ports in one expression (no reassignments)
        let ports = livi::EmptyPortConnections::new()
            .with_audio_inputs(in_refs.into_iter())
            .with_audio_outputs(out_refs.into_iter())
            .with_atom_sequence_inputs(atom_in_vec.into_iter())
            .with_atom_sequence_outputs(atom_outs.iter_mut());

        // 8) Run
        unsafe {
            if let Err(e) = self.instance.run(samples, ports) {
                eprintln!("[LV2] run() error: {}", e);
                // Move fields back before returning
                self.atom_outputs = atom_outs;
                self.scratch_audio_out = scratch;
                return Err(e.into());
            }
        }

        // 9) Move taken fields back into self
        self.atom_outputs = atom_outs;
        self.scratch_audio_out = scratch;

        Ok(())
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
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (1, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (2, 0, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (0, 1, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs(std::iter::once(&mut *output_l))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
                output_r[..samples].copy_from_slice(&output_l[..samples]);
            }
            (0, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_outputs([&mut *output_l, &mut *output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (1, 1, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
                output_r[..samples].copy_from_slice(&output_l[..samples]);
            }
            (1, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([&mut *output_l, &mut *output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (2, 1, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs(std::iter::once(&mut *output_l))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
                output_r[..samples].copy_from_slice(&output_l[..samples]);
            }
            (2, 2, false) => {
                let ports = EmptyPortConnections::new()
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([&mut *output_l, &mut *output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }

            // With MIDI input
            (0, 0, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (0, 2, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_outputs([&mut *output_l, &mut *output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (1, 2, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l].into_iter())
                    .with_audio_outputs([&mut *output_l, &mut *output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
                }
            }
            (2, 2, true) => {
                let atom_in = atom_in_opt.expect("atom input required");
                let ports = EmptyPortConnections::new()
                    .with_atom_sequence_inputs(std::iter::once(atom_in))
                    .with_audio_inputs([input_l, input_r].into_iter())
                    .with_audio_outputs([&mut *output_l, &mut *output_r].into_iter())
                    .with_atom_sequence_outputs(self.atom_outputs.iter_mut());
                unsafe {
                    if let Err(e) = self.instance.run(samples, ports) {
                        eprintln!("[LV2] run() error: {}", e);
                        return Err(e.into());
                    }
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
