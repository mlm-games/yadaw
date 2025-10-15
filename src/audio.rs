use crate::audio_snapshot;
use crate::audio_state::{
    AudioGraphSnapshot, AudioState, MidiClipSnapshot, RealtimeCommand, RtAutomationLaneSnapshot,
    RtAutomationTarget, RtCurveType, TrackSnapshot,
};
use crate::audio_utils::{calculate_stereo_gains, soft_clip};
use crate::constants::{
    DEBUG_PLUGIN_AUDIO, MAX_BUFFER_SIZE, PREVIEW_NOTE_DURATION, RECORDING_BUFFER_SIZE,
};
use crate::messages::UIUpdate;
use crate::midi_utils::generate_sine_for_note;
use crate::mixer::{ChannelStrip, MixerEngine};
use crate::model::clip::AudioClip;
use crate::model::plugin_api::{
    BackendKind, HostConfig, MidiEvent, ParamKey, PluginInstance, ProcessCtx, RtMidiEvent,
};
use crate::plugin_facade::HostFacade;
use crate::time_utils::TimeConverter;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rtrb::{Consumer, RingBuffer};
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::model::plugin_api::PluginInstance as UnifiedInstance;

type PluginInstanceHandle = (usize, u64);
type PluginMap = DashMap<PluginInstanceHandle, Box<dyn UnifiedInstance>>;

static PLUGIN_STORE: Lazy<DashMap<PluginInstanceHandle, Box<dyn UnifiedInstance>>> =
    Lazy::new(DashMap::new);
static PLUGIN_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static PLUGIN_ID_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

fn generate_plugin_handle() -> PluginInstanceHandle {
    let id = PLUGIN_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let r#gen = PLUGIN_GENERATION.fetch_add(1, Ordering::Relaxed);
    (id, r#gen)
}

pub struct AudioEngine {
    graph_snapshot: AudioGraphSnapshot,
    track_processors: HashMap<u64, TrackProcessor>,
    audio_state: Arc<AudioState>,
    recording_state: RecordingState,
    preview_note: Option<PreviewNote>,
    sample_rate: f64,
    updates: Sender<UIUpdate>,
    mixer: MixerEngine,
    channel_strips: HashMap<u64, ChannelStrip>,
    xrun_count: u64,
    paused_last: bool,
    host_facade: HostFacade,
}

struct TrackProcessor {
    track_id: u64,
    plugins: HashMap<u64, PluginProcessorUnified>, // plugin_id -> plugin
    plugin_order: Vec<u64>,                        // Rendering order

    input_buffers: Vec<Vec<f32>>,
    output_buffers: Vec<Vec<f32>>,
    active_notes: Vec<ActiveMidiNote>,
    last_pattern_position: f64,
    automated_volume: f32,
    automated_pan: f32,
    automated_plugin_params: DashMap<(u64, String), f32>, // (plugin_id, param) -> value

    pattern_loop_count: u32,
    notes_triggered_this_loop: Vec<u8>,
    last_block_end_samples: f64,
    plugin_active_notes: Vec<(u8, u8)>,
    automation_sample_buffers: HashMap<String, Vec<f32>>,
}

impl TrackProcessor {
    fn new(track_id: u64) -> Self {
        let mut s = Self {
            track_id,
            plugins: HashMap::new(),
            plugin_order: Vec::new(),
            input_buffers: Vec::new(),
            output_buffers: Vec::new(),
            active_notes: Vec::new(),
            last_pattern_position: 0.0,
            automated_volume: f32::NAN,
            automated_pan: f32::NAN,
            automated_plugin_params: DashMap::new(),
            pattern_loop_count: 0,
            notes_triggered_this_loop: Vec::new(),
            last_block_end_samples: 0.0,
            plugin_active_notes: Vec::new(),
            automation_sample_buffers: HashMap::new(),
        };
        s.ensure_channels(2);
        s
    }

    fn ensure_channels(&mut self, n: usize) {
        let n = n.max(2);
        if self.input_buffers.len() != n {
            self.input_buffers = (0..n).map(|_| vec![0.0; MAX_BUFFER_SIZE]).collect();
            self.output_buffers = (0..n).map(|_| vec![0.0; MAX_BUFFER_SIZE]).collect();
        }
    }
}

struct PluginProcessorUnified {
    plugin_id: u64,
    rt_instance_id: Option<(usize, u64)>,
    backend: BackendKind,
    uri: String,
    bypass: bool,
    param_name_to_key: HashMap<String, ParamKey>,
}

#[derive(Clone)]
struct ActiveMidiNote {
    pitch: u8,
    velocity: u8,
    start_sample: f64,
}

#[derive(Clone)]
struct PreviewNote {
    track_id: u64,
    pitch: u8,
    start_position: f64,
}

struct RecordingState {
    is_recording: bool,
    recording_track: Option<u64>,
    recording_consumer: Consumer<f32>,
    recording_start_position: f64,
    accumulated_samples: Vec<f32>,
    monitor_queue: Vec<f32>,
}

pub fn run_audio_thread(
    audio_state: Arc<AudioState>,
    realtime_commands: Receiver<RealtimeCommand>,
    updates: Sender<UIUpdate>,
    snapshot_rx: Receiver<AudioGraphSnapshot>,
) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("No output device");
    let config = device.default_output_config().expect("No default config");
    let sample_rate = config.sample_rate().0 as f64;
    let channels = config.channels() as usize;

    audio_state.sample_rate.store(sample_rate as f32);

    let host_cfg = HostConfig {
        sample_rate,
        max_block: MAX_BUFFER_SIZE,
    };
    let host_facade =
        crate::plugin_facade::HostFacade::new(host_cfg).expect("HostFacade init failed");

    // Create recording buffer
    let (recording_producer, recording_consumer) = RingBuffer::<f32>::new(RECORDING_BUFFER_SIZE);

    // Initialize engine

    let mut engine = AudioEngine {
        graph_snapshot: AudioGraphSnapshot::default(),
        audio_state: audio_state.clone(),
        track_processors: HashMap::new(),
        recording_state: RecordingState {
            is_recording: false,
            recording_track: None,
            recording_consumer,
            recording_start_position: 0.0,
            accumulated_samples: Vec::new(),
            monitor_queue: Vec::new(),
        },
        preview_note: None,
        sample_rate,
        updates: updates.clone(),
        mixer: MixerEngine::new(),
        channel_strips: HashMap::new(),
        xrun_count: 0,
        paused_last: false,
        host_facade,
    };

    // Start recording input thread
    let recording_producer = Arc::new(parking_lot::Mutex::new(recording_producer));
    let audio_state_clone = audio_state.clone();
    let updates_clone = updates.clone();

    std::thread::spawn(move || {
        let host = cpal::default_host();
        if let Some(input_device) = host.default_input_device()
            && let Ok(input_config) = input_device.default_input_config()
        {
            let channels = input_config.channels() as usize;
            let recording_producer = recording_producer.clone();

            let mut last_meter = std::time::Instant::now();
            let mut peak_acc: f32 = 0.0;

            let input_callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut producer = recording_producer.lock();
                for frame in data.chunks(channels) {
                    let mono_sample = frame.iter().sum::<f32>() / channels as f32;
                    let _ = producer.push(mono_sample);
                    peak_acc = peak_acc.max(mono_sample.abs());
                }

                let elapsed = last_meter.elapsed();
                if elapsed >= std::time::Duration::from_millis(50) {
                    let level = peak_acc;
                    peak_acc = 0.0;
                    last_meter = std::time::Instant::now();
                    let _ = updates_clone.try_send(UIUpdate::RecordingLevel(level));
                }
            };

            if let Ok(input_stream) = input_device.build_input_stream(
                &input_config.config(),
                input_callback,
                |err| eprintln!("Input stream error: {}", err),
                None,
            ) {
                if let Err(e) = input_stream.play() {
                    eprintln!("Failed to play input stream: {}", e);
                }
                std::thread::park();
            }
        }
    });

    // Audio callback
    let audio_callback = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let num_frames = data.len() / channels;
        let cb_start = Instant::now();

        data.fill(0.0);

        let is_playing = engine.audio_state.playing.load(Ordering::Relaxed);
        let should_be_recording = engine.audio_state.recording.load(Ordering::Relaxed);
        let is_actually_recording = engine.recording_state.is_recording;

        // Drain RT commands at block start
        while let Ok(cmd) = realtime_commands.try_recv() {
            engine.process_realtime_command(cmd);
        }

        if let Ok(new_snapshot) = snapshot_rx.try_recv() {
            engine.apply_new_snapshot(new_snapshot);
        }

        // Cap monitor queue
        if engine.recording_state.monitor_queue.len() > 2 * MAX_BUFFER_SIZE {
            let drop_n = engine.recording_state.monitor_queue.len() - 2 * MAX_BUFFER_SIZE;
            engine.recording_state.monitor_queue.drain(0..drop_n);
        }

        if is_playing && should_be_recording && !is_actually_recording {
            // START RECORDING
            if engine.recording_state.recording_track.is_some() {
                engine.recording_state.is_recording = true;
                // Capture the precise start position in samples
                engine.recording_state.recording_start_position = engine.audio_state.get_position();
                engine.recording_state.accumulated_samples.clear();
                let _ = engine
                    .updates
                    .try_send(UIUpdate::RecordingStateChanged(true));
            }
        } else if (!is_playing || !should_be_recording) && is_actually_recording {
            // STOP RECORDING
            engine.recording_state.is_recording = false;
            engine.audio_state.recording.store(false, Ordering::Relaxed); // Sync atomic back
            let _ = engine
                .updates
                .try_send(UIUpdate::RecordingStateChanged(false));

            if let Some(track_id) = engine.recording_state.recording_track {
                if !engine.recording_state.accumulated_samples.is_empty() {
                    let converter = TimeConverter::new(
                        engine.sample_rate as f32,
                        engine.audio_state.bpm.load(),
                    );
                    let start_beat =
                        converter.samples_to_beats(engine.recording_state.recording_start_position);

                    // Calculate length from number of samples recorded
                    let num_samples = engine.recording_state.accumulated_samples.len();
                    let length_beats = converter.samples_to_beats(num_samples as f64);

                    let clip = AudioClip {
                        id: 0, // Will be assigned by UI thread
                        name: format!("Rec {}", chrono::Local::now().format("%H:%M:%S")),
                        start_beat,
                        length_beats,
                        samples: engine.recording_state.accumulated_samples.clone(),
                        sample_rate: engine.sample_rate as f32,
                        ..Default::default()
                    };

                    let _ = engine
                        .updates
                        .send(UIUpdate::RecordingFinished(track_id, clip));
                    engine.recording_state.accumulated_samples.clear();
                }
            }
        }

        // Accumulate samples ONLY if actually recording
        if engine.recording_state.is_recording {
            while let Ok(sample) = engine.recording_state.recording_consumer.pop() {
                engine.recording_state.accumulated_samples.push(sample);
                engine.recording_state.monitor_queue.push(sample);
            }
        } else {
            // If not recording, just drain the consumer into the monitor queue
            while let Ok(sample) = engine.recording_state.recording_consumer.pop() {
                engine.recording_state.monitor_queue.push(sample);
            }
        }

        if !is_playing {
            if !engine.paused_last {
                engine.midi_panic();
                engine.paused_last = true;
                for processor in engine.track_processors.values_mut() {
                    processor.last_pattern_position = 0.0;
                    processor.pattern_loop_count = 0;
                    processor.notes_triggered_this_loop.clear();
                    processor.plugin_active_notes.clear();
                }
            }

            let elapsed = cb_start.elapsed();
            let budget = (num_frames as f64 / engine.sample_rate).max(1e-6);
            let cpu = (elapsed.as_secs_f64() / budget) as f32;
            let health = (1.0 - cpu).clamp(0.0, 1.0);
            let latency_ms = (num_frames as f32 / engine.sample_rate as f32) * 1000.0;

            let _ = engine.updates.try_send(UIUpdate::PerformanceMetric {
                cpu_usage: cpu,
                buffer_fill: health,
                xruns: engine.xrun_count as u32,
                plugin_time_ms: 0.0,
                latency_ms,
            });
            return;
        } else {
            engine.paused_last = false;
        }

        let mut plugin_time_ms_accum: f32 = 0.0;

        let current_position = engine.audio_state.get_position();
        let next_position = engine.process_audio(
            data,
            num_frames,
            channels,
            current_position,
            &mut plugin_time_ms_accum,
        );
        engine.audio_state.set_position(next_position);

        let elapsed = cb_start.elapsed();
        let budget = (num_frames as f64 / engine.sample_rate).max(1e-6);
        let cpu = (elapsed.as_secs_f64() / budget) as f32;
        if cpu > 1.0 {
            engine.xrun_count += 1;
        }
        let health = (1.0 - cpu).clamp(0.0, 1.0);
        let latency_ms = (num_frames as f32 / engine.sample_rate as f32) * 1000.0;

        let _ = engine.updates.try_send(UIUpdate::PerformanceMetric {
            cpu_usage: cpu,
            buffer_fill: health,
            xruns: engine.xrun_count as u32,
            plugin_time_ms: plugin_time_ms_accum,
            latency_ms,
        });

        let _ = engine.updates.try_send(UIUpdate::Position(next_position));
    };

    let stream = device
        .build_output_stream(
            &config.into(),
            audio_callback,
            |err| eprintln!("Audio stream error: {}", err),
            None,
        )
        .expect("Failed to create audio stream");

    stream.play().expect("Failed to start audio stream");
    std::thread::park();
}

impl AudioEngine {
    pub fn new_for_offline_render(
        initial_tracks: &[TrackSnapshot],
        audio_state: &AudioState,
        export_sample_rate: f32,
    ) -> Result<Self, anyhow::Error> {
        // Create a dummy channel since we don't send UI updates
        let (dummy_tx, _) = crossbeam_channel::unbounded();

        let host_cfg = HostConfig {
            sample_rate: export_sample_rate as f64,
            max_block: MAX_BUFFER_SIZE,
        };
        let host_facade = crate::plugin_facade::HostFacade::new(host_cfg)?;

        let offline_audio_state = AudioState::new();

        offline_audio_state
            .loop_enabled
            .store(false, Ordering::Relaxed);

        // Copy BPM from the main project state
        offline_audio_state.bpm.store(audio_state.bpm.load());

        let mut engine = AudioEngine {
            graph_snapshot: AudioGraphSnapshot::default(), // Will be populated by setup method
            audio_state: Arc::new(offline_audio_state),
            track_processors: HashMap::new(),
            recording_state: RecordingState {
                is_recording: false,
                recording_track: None,
                // Use dummy ring buffer for offline mode
                recording_consumer: rtrb::RingBuffer::<f32>::new(1).1,
                recording_start_position: 0.0,
                accumulated_samples: Vec::new(),
                monitor_queue: Vec::new(),
            },
            preview_note: None,
            sample_rate: export_sample_rate as f64,
            updates: dummy_tx,
            mixer: MixerEngine::new(),
            channel_strips: HashMap::new(),
            xrun_count: 0,
            paused_last: false,
            host_facade,
        };

        engine.full_sync_for_offline_setup(initial_tracks);

        Ok(engine)
    }

    fn full_sync_for_offline_setup(&mut self, tracks: &[TrackSnapshot]) {
        // This method combines logic from the old `full_sync_from_snapshot`
        // with the state ownership of the new architecture.

        // 1. Clear any existing state
        self.track_processors.clear();
        self.channel_strips.clear();

        // 2. Build new processors and instantiate all plugins
        for track_snapshot in tracks {
            let track_id = track_snapshot.track_id;

            // Create a processor for the track
            let mut proc = TrackProcessor::new(track_id);

            // Instantiate this track's entire plugin chain
            for plugin_snapshot in &track_snapshot.plugin_chain {
                let plugin_id = plugin_snapshot.plugin_id;

                match self
                    .host_facade
                    .instantiate(plugin_snapshot.backend, &plugin_snapshot.uri)
                {
                    Ok(mut inst) => {
                        // Apply all saved parameters to the new instance
                        for param_entry in plugin_snapshot.params.iter() {
                            let param_name = param_entry.key();
                            let param_value = *param_entry.value();
                            let maybe_key = inst
                                .params()
                                .iter()
                                .find(|p| p.name == *param_name)
                                .map(|p| p.key.clone());
                            if let Some(key) = maybe_key {
                                inst.set_param(&key, param_value);
                            }
                        }

                        let handle = generate_plugin_handle();
                        PLUGIN_STORE.insert(handle, Box::from(inst));

                        let param_name_to_key: HashMap<String, ParamKey> = PLUGIN_STORE
                            .get(&handle)
                            .map(|inst| {
                                inst.params()
                                    .iter()
                                    .map(|p| (p.name.clone(), p.key.clone()))
                                    .collect()
                            })
                            .unwrap_or_default();

                        let plugin_processor = PluginProcessorUnified {
                            plugin_id,
                            rt_instance_id: Some(handle),
                            backend: plugin_snapshot.backend,
                            uri: plugin_snapshot.uri.clone(),
                            bypass: plugin_snapshot.bypass,
                            param_name_to_key,
                        };

                        proc.plugins.insert(plugin_id, plugin_processor);
                        proc.plugin_order.push(plugin_id);
                    }
                    Err(e) => {
                        eprintln!(
                            "Offline Render: Failed to instantiate plugin {}: {}",
                            plugin_snapshot.uri, e
                        );
                        // Create a disabled placeholder to avoid breaking the chain
                        let placeholder = PluginProcessorUnified {
                            plugin_id,
                            rt_instance_id: None,
                            backend: plugin_snapshot.backend,
                            uri: plugin_snapshot.uri.clone(),
                            bypass: true,
                            param_name_to_key: HashMap::new(),
                        };
                        proc.plugins.insert(plugin_id, placeholder);
                        proc.plugin_order.push(plugin_id);
                    }
                }
            }
            self.track_processors.insert(track_id, proc);

            // Configure the channel strip for this track
            let strip = self.channel_strips.entry(track_id).or_default();
            strip.gain = track_snapshot.volume;
            strip.pan = track_snapshot.pan;
            strip.mute = track_snapshot.muted;
            strip.solo = track_snapshot.solo;
        }

        // 3. Set the graph snapshot for the engine
        self.graph_snapshot = AudioGraphSnapshot {
            tracks: tracks.to_vec(),
            track_order: tracks.iter().map(|t| t.track_id).collect(),
        };

        // 4. Update the recording track reference (though it won't be used)
        self.recording_state.recording_track = tracks
            .iter()
            .find(|t| t.armed && !t.is_midi)
            .map(|t| t.track_id);
    }

    fn process_realtime_command(&mut self, cmd: RealtimeCommand) {
        match cmd {
            RealtimeCommand::UpdateTrackVolume(track_id, vol) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.gain = vol;
                }
            }
            RealtimeCommand::UpdateTrackPan(track_id, pan) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.pan = pan;
                }
            }
            RealtimeCommand::UpdateTrackMute(track_id, mute) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.mute = mute;
                }
            }
            RealtimeCommand::UpdateTrackSolo(track_id, solo) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.solo = solo;
                }
            }

            RealtimeCommand::UpdatePluginBypass(track_id, plugin_id, bypass) => {
                if let Some(proc) = self.track_processors.get_mut(&track_id) {
                    if let Some(plugin) = proc.plugins.get_mut(&plugin_id) {
                        plugin.bypass = bypass;
                    }
                }
            }

            RealtimeCommand::PreviewNote(track_id, pitch, start_position) => {
                self.preview_note = Some(PreviewNote {
                    track_id, // Store as index for RT processing
                    pitch,
                    start_position,
                });
            }
            RealtimeCommand::StopPreviewNote => {
                self.preview_note = None;
            }
            RealtimeCommand::SetLoopEnabled(enabled) => {
                self.audio_state
                    .loop_enabled
                    .store(enabled, Ordering::Relaxed);
            }
            RealtimeCommand::SetLoopRegion(start, end) => {
                self.audio_state.loop_start.store(start);
                self.audio_state.loop_end.store(end);
            }
            RealtimeCommand::AddUnifiedPlugin {
                track_id,
                plugin_id,
                backend,
                uri,
            } => {
                let proc = self
                    .track_processors
                    .entry(track_id)
                    .or_insert_with(|| TrackProcessor::new(track_id));

                match self.host_facade.instantiate(backend, &uri) {
                    Ok(inst) => {
                        let mut name_to_key = HashMap::new();
                        for p in inst.params() {
                            name_to_key.insert(p.name.clone(), p.key.clone());
                        }

                        // --- REPLACEMENT ---
                        let handle = generate_plugin_handle();
                        PLUGIN_STORE.insert(handle, inst);

                        let plugin = PluginProcessorUnified {
                            plugin_id,
                            rt_instance_id: Some(handle), // Store the (id, gen) handle
                            backend,
                            uri: uri.clone(),
                            bypass: false,
                            param_name_to_key: name_to_key,
                        };

                        proc.plugins.insert(plugin_id, plugin);
                        proc.plugin_order.push(plugin_id);
                    }
                    Err(e) => eprintln!("Plugin instantiate failed {}: {}", uri, e),
                }
            }

            RealtimeCommand::RemovePluginInstance {
                track_id,
                plugin_id,
            } => {
                if let Some(proc) = self.track_processors.get_mut(&track_id) {
                    if let Some(plugin) = proc.plugins.remove(&plugin_id) {
                        if let Some(handle) = plugin.rt_instance_id {
                            PLUGIN_STORE.remove(&handle);
                        }
                    }
                    proc.plugin_order.retain(|&id| id != plugin_id);
                }
            }

            RealtimeCommand::UpdatePluginParam(track_id, plugin_id, param_name, value) => {
                if let Some(proc) = self.track_processors.get_mut(&track_id) {
                    if let Some(plugin) = proc.plugins.get(&plugin_id) {
                        if let Some(handle) = plugin.rt_instance_id {
                            // handle is (id, gen)
                            let key = match plugin.backend {
                                BackendKind::Lv2 => ParamKey::Lv2(param_name.clone()),
                                BackendKind::Clap => plugin
                                    .param_name_to_key
                                    .get(&param_name)
                                    .cloned()
                                    .unwrap_or(ParamKey::Clap(0)),
                            };

                            if let Some(mut inst) = PLUGIN_STORE.get_mut(&handle) {
                                inst.set_param(&key, value);
                            }
                        }
                    }
                }
            }

            RealtimeCommand::UpdateTracks(new_tracks) => {
                self.full_sync_for_offline_setup(&new_tracks);
            }
            _ => {}
        }
    }

    fn update_track_processors_without_plugins(&mut self, tracks: &[TrackSnapshot]) {
        // Ensure we have processors and strips for all tracks
        for track in tracks {
            self.track_processors
                .entry(track.track_id)
                .or_insert_with(|| TrackProcessor::new(track.track_id));

            let strip = self
                .channel_strips
                .entry(track.track_id)
                .or_insert_with(ChannelStrip::default);

            strip.gain = track.volume;
            strip.pan = track.pan;
            strip.mute = track.muted;
            strip.solo = track.solo;
        }

        // Update recording track
        self.recording_state.recording_track = tracks
            .iter()
            .find(|t| t.armed && !t.is_midi)
            .map(|t| t.track_id);
    }

    pub fn process_audio(
        &mut self,
        output: &mut [f32],
        num_frames: usize,
        channels: usize,
        mut current_position: f64,
        plugin_time_ms_accum: &mut f32,
    ) -> f64 {
        let tracks: &Vec<TrackSnapshot> = &self.graph_snapshot.tracks;
        let track_order: &Vec<u64> = &self.graph_snapshot.track_order;

        let bpm = self.audio_state.bpm.load();
        let master_volume = self.audio_state.master_volume.load();

        let loop_enabled = self.audio_state.loop_enabled.load(Ordering::Relaxed);
        let loop_start_beats = self.audio_state.loop_start.load();
        let loop_end_beats = self.audio_state.loop_end.load();

        let converter = TimeConverter::new(self.sample_rate as f32, bpm);
        let loop_start_samp = converter.beats_to_samples(loop_start_beats);
        let loop_end_samp = converter.beats_to_samples(loop_end_beats);

        // Guard against degenerate/too-short loops (< 1 sample)
        let loop_active = loop_enabled && (loop_end_samp - loop_start_samp) >= 1.0;

        // Meters (max across sub-blocks)
        let mut track_peaks: HashMap<u64, (f32, f32)> = HashMap::new();
        let mut master_peak_l = 0.0f32;
        let mut master_peak_r = 0.0f32;

        let mut frames_processed = 0usize;

        while frames_processed < num_frames {
            let block_start_samples = current_position;
            let block_start_beat = converter.samples_to_beats(block_start_samples);

            // How many frames remain before loop end? Round up so 0<remain<1 -> 1 frame.
            let frames_to_loop_end = if loop_active && block_start_samples < loop_end_samp {
                let remain = loop_end_samp - block_start_samples;
                if remain <= 0.0 {
                    0
                } else {
                    remain.ceil() as usize
                }
            } else {
                usize::MAX
            };

            let mut frames_to_process = (num_frames - frames_processed)
                .min(MAX_BUFFER_SIZE)
                .min(frames_to_loop_end);

            if loop_active && frames_to_process == 0 {
                current_position = loop_start_samp;
                for processor in &mut self.track_processors.values_mut() {
                    processor.active_notes.clear();
                }
                continue; // re-evaluate sizing after the jump
            }

            if frames_to_process == 0 {
                frames_to_process = 1;
            }

            for i in frames_processed..(frames_processed + frames_to_process) {
                let out_idx = i * channels;
                output[out_idx] = 0.0;
                if channels > 1 {
                    output[out_idx + 1] = 0.0;
                }
            }

            let any_track_soloed = tracks.iter().any(|t| t.solo);
            let preview_opt = self.preview_note.clone();
            let is_recording_now = self.audio_state.recording.load(Ordering::Relaxed);
            let rec_track_id = self.recording_state.recording_track;

            for &track_id in track_order {
                let track = match tracks.iter().find(|t| t.track_id == track_id) {
                    Some(t) => t,
                    None => continue,
                };

                let strip = self.channel_strips.get(&track_id);

                let strip_mute = strip.map_or(track.muted, |s| s.mute);
                let strip_solo = strip.map_or(track.solo, |s| s.solo);

                let any_track_soloed = self.channel_strips.values().any(|s| s.solo);

                if strip_mute || (any_track_soloed && !strip_solo) {
                    continue;
                }

                if let Some(processor) = self.track_processors.get_mut(&track_id) {
                    apply_automation_smooth(
                        track,
                        processor,
                        block_start_samples,
                        frames_to_process,
                        &converter,
                    );
                    // Build per-track buffers (post-clip, pre-plugin)
                    if track.is_midi {
                        process_midi_track(
                            track,
                            processor,
                            frames_to_process,
                            block_start_samples,
                            bpm,
                            self.sample_rate,
                            loop_active,
                            loop_start_beats, // pass beats for MIDI event builder
                            loop_end_beats,   // pass beats for MIDI event builder
                        );
                    } else {
                        process_audio_track(
                            track,
                            processor,
                            frames_to_process,
                            block_start_samples,
                            bpm,
                            self.sample_rate,
                        );
                    }

                    // Note preview (simple synth audition)
                    if let Some(ref preview) = preview_opt {
                        // Find the track by ID to see if it matches the preview's track_id
                        if let Some(preview_track) =
                            tracks.iter().find(|t| t.track_id == preview.track_id)
                        {
                            if preview_track.track_id == track.track_id {
                                process_preview_note(
                                    processor,
                                    preview,
                                    frames_to_process,
                                    block_start_samples,
                                    self.sample_rate,
                                );
                            }
                        }
                    }

                    // Mix input monitoring into the recording track (mono -> stereo)
                    if track.monitor_enabled || (is_recording_now && Some(track_id) == rec_track_id)
                    {
                        let take = self
                            .recording_state
                            .monitor_queue
                            .len()
                            .min(frames_to_process);
                        for i in 0..take {
                            let s = self.recording_state.monitor_queue[i];
                            processor.input_buffers[0][i] += s;
                            processor.input_buffers[1][i] += s;
                        }
                        if take > 0 {
                            self.recording_state.monitor_queue.drain(..take);
                        }
                    }

                    // Run plugin chain
                    process_track_plugins(
                        track,
                        processor,
                        frames_to_process,
                        block_start_samples, // pass samples for block start
                        bpm,
                        self.sample_rate,
                        loop_active,
                        loop_start_beats,
                        loop_end_beats,
                        plugin_time_ms_accum,
                    );

                    let vol_automation = processor.automation_sample_buffers.get("volume");
                    let pan_automation = processor.automation_sample_buffers.get("pan");

                    let strip_volume = strip.map_or(track.volume, |s| s.gain);
                    let strip_pan = strip.map_or(track.pan, |s| s.pan);

                    let mut tp_l = 0.0f32;
                    let mut tp_r = 0.0f32;

                    for i in 0..frames_to_process {
                        // Determine gain and pan for this specific sample using the correct fallback logic
                        let vol = vol_automation.map_or_else(
                            || {
                                if processor.automated_volume.is_finite() {
                                    processor.automated_volume
                                } else {
                                    strip_volume
                                }
                            },
                            |buf| buf[i],
                        );

                        let pan = pan_automation.map_or_else(
                            || {
                                if processor.automated_pan.is_finite() {
                                    processor.automated_pan
                                } else {
                                    strip_pan
                                }
                            },
                            |buf| buf[i] * 2.0 - 1.0, // convert 0..1 to -1..1
                        );

                        let (left_gain, right_gain) = calculate_stereo_gains(vol, pan);

                        let out_idx = (frames_processed + i) * channels;
                        let l = processor.input_buffers[0][i] * left_gain;
                        let r = processor.input_buffers[1][i] * right_gain;

                        output[out_idx] += l;
                        if channels > 1 {
                            output[out_idx + 1] += r;
                        }

                        tp_l = tp_l.max(l.abs());
                        tp_r = tp_r.max(r.abs());
                    }
                    track_peaks.insert(track_id, (tp_l, tp_r));
                    processor.automation_sample_buffers.clear();
                }
            }

            // Apply master gain and soft clip; update master peaks
            for i in frames_processed..(frames_processed + frames_to_process) {
                let out_idx = i * channels;
                let l = soft_clip(output[out_idx] * master_volume);
                output[out_idx] = l;
                master_peak_l = master_peak_l.max(l.abs());

                if channels > 1 {
                    let r = soft_clip(output[out_idx + 1] * master_volume);
                    output[out_idx + 1] = r;
                    master_peak_r = master_peak_r.max(r.abs());
                } else {
                    master_peak_r = master_peak_r.max(l.abs());
                }
            }

            current_position += frames_to_process as f64;
            frames_processed += frames_to_process;

            // If we crossed or landed on loop end, jump back for the next sub-iteration
            if loop_active && current_position >= loop_end_samp {
                current_position = loop_start_samp;
                for processor in &mut self.track_processors.values_mut() {
                    processor.active_notes.clear();
                }
            }
        }

        // Send meters once per callback
        let _ = self.updates.try_send(UIUpdate::TrackLevels(track_peaks));
        let _ = self
            .updates
            .try_send(UIUpdate::MasterLevel(master_peak_l, master_peak_r));

        current_position
    }

    fn midi_panic(&mut self) {
        // Build All Notes Off + All Sound Off for channels 0..15
        let panic_events: Vec<MidiEvent> = (0..16)
            .flat_map(|ch| {
                vec![
                    MidiEvent {
                        status: 0xB0 | ch,
                        data1: 123,
                        data2: 0,
                        time_frames: 0,
                    },
                    MidiEvent {
                        status: 0xB0 | ch,
                        data1: 120,
                        data2: 0,
                        time_frames: 0,
                    },
                ]
            })
            .collect();

        for proc in self.track_processors.values_mut() {
            for ppu in proc.plugins.values_mut() {
                if let Some(handle) = ppu.rt_instance_id {
                    let dl = [0.0f32; 64];
                    let mut ol = [0.0f32; 64];
                    let mut or_ = [0.0f32; 64];
                    let inputs: [&[f32]; 2] = [&dl, &dl];
                    let mut outputs: [&mut [f32]; 2] = [&mut ol[..], &mut or_[..]];
                    let ctx = ProcessCtx {
                        frames: 64,
                        bpm: self.audio_state.bpm.load(),
                        time_samples: 0.0,
                        loop_active: false,
                    };
                    let panic_events: Vec<RtMidiEvent> = (0..16)
                        .flat_map(|ch| {
                            vec![
                                RtMidiEvent {
                                    status: 0xB0 | ch,
                                    data1: 123,
                                    data2: 0,
                                    time_frames: 0,
                                },
                                RtMidiEvent {
                                    status: 0xB0 | ch,
                                    data1: 120,
                                    data2: 0,
                                    time_frames: 0,
                                },
                            ]
                        })
                        .collect();
                    if let Some(mut inst) = PLUGIN_STORE.get_mut(&handle) {
                        let _ = inst.process(&ctx, &inputs, &mut outputs, &panic_events);
                    }
                }
            }
            proc.active_notes.clear();
            proc.last_pattern_position = 0.0;
            proc.pattern_loop_count = 0;
            proc.notes_triggered_this_loop.clear();
            proc.last_block_end_samples = 0.0;
        }
    }

    fn apply_new_snapshot(&mut self, new_snapshot: AudioGraphSnapshot) {
        let new_track_ids: std::collections::HashSet<u64> =
            new_snapshot.track_order.iter().cloned().collect();

        // Remove processors and channel strips for tracks that no longer exist.
        self.track_processors
            .retain(|track_id, _| new_track_ids.contains(track_id));
        self.channel_strips
            .retain(|track_id, _| new_track_ids.contains(track_id));

        // Add/update processors and channel strips for all tracks.
        for track_snapshot in &new_snapshot.tracks {
            let track_id = track_snapshot.track_id;

            self.track_processors
                .entry(track_id)
                .or_insert_with(|| TrackProcessor::new(track_id));

            // Update the corresponding channel strip.
            let strip = self.channel_strips.entry(track_id).or_default();
            strip.gain = track_snapshot.volume;
            strip.pan = track_snapshot.pan;
            strip.mute = track_snapshot.muted;
            strip.solo = track_snapshot.solo;
        }

        self.graph_snapshot = new_snapshot;

        self.recording_state.recording_track = self
            .graph_snapshot
            .tracks
            .iter()
            .find(|t| t.armed && !t.is_midi)
            .map(|t| t.track_id);
    }
}

#[inline]
fn effective_gains(track: &TrackSnapshot, processor: &TrackProcessor) -> (f32, f32) {
    let vol = if processor.automated_volume.is_finite() {
        processor.automated_volume
    } else {
        track.volume
    };
    let pan = if processor.automated_pan.is_finite() {
        processor.automated_pan
    } else {
        track.pan
    };
    calculate_stereo_gains(vol, pan)
}

fn process_midi_track(
    track: &TrackSnapshot,
    processor: &mut TrackProcessor,
    num_frames: usize,
    current_position: f64,
    bpm: f32,
    sample_rate: f64,
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,
) {
    use std::collections::HashSet;

    let converter = TimeConverter::new(sample_rate as f32, bpm);
    let current_beat = converter.samples_to_beats(current_position);

    // Handle looping
    let effective_beat = if loop_enabled && loop_end > loop_start {
        let loop_len = loop_end - loop_start;
        if current_beat >= loop_end {
            loop_start + ((current_beat - loop_start) % loop_len)
        } else {
            current_beat
        }
    } else {
        current_beat
    };

    // Compute which notes should be ON at effective_beat
    let mut desired: HashSet<u8> = HashSet::new();
    // Keep velocity and start_beat for proper synth phase alignment
    let mut desired_detail: Vec<(u8, u8, f64)> = Vec::new();

    for clip in &track.midi_clips {
        let clip_end = clip.start_beat + clip.length_beats;
        if effective_beat < clip.start_beat || effective_beat >= clip_end {
            continue;
        }
        for n in &clip.notes {
            let s = clip.start_beat + n.start;
            let e = s + n.duration;
            if s <= effective_beat && effective_beat < e && desired.insert(n.pitch) {
                desired_detail.push((n.pitch, n.velocity, s));
            }
        }
    }

    // Remove any stale active notes that shouldn't be on now
    processor
        .active_notes
        .retain(|n| desired.contains(&n.pitch));

    // Add newly required active notes
    for (pitch, vel, start_abs_beat) in desired_detail {
        if !processor.active_notes.iter().any(|n| n.pitch == pitch) {
            // Start sample so the oscillator phase corresponds to the real note start
            let elapsed_beats = effective_beat - start_abs_beat;
            let elapsed_samples = converter.beats_to_samples(elapsed_beats).max(0.0);
            processor.active_notes.push(ActiveMidiNote {
                pitch,
                velocity: vel,
                start_sample: current_position - elapsed_samples,
            });
        }
    }

    // Clear input buffers
    processor.input_buffers[0][..num_frames].fill(0.0);
    processor.input_buffers[1][..num_frames].fill(0.0);

    // Generate audio if no plugins
    if processor.plugins.is_empty() && !processor.active_notes.is_empty() {
        for i in 0..num_frames {
            let mut sample = 0.0;
            for note in &processor.active_notes {
                let sample_offset = current_position + i as f64 - note.start_sample;
                sample +=
                    generate_sine_for_note(note.pitch, note.velocity, sample_offset, sample_rate);
            }
            processor.input_buffers[0][i] = sample;
            processor.input_buffers[1][i] = sample;
        }
    }
}

fn process_audio_track(
    track: &TrackSnapshot,
    processor: &mut TrackProcessor,
    num_frames: usize,
    current_position: f64,
    bpm: f32,
    sample_rate: f64,
) {
    // Zero
    processor.input_buffers[0][..num_frames].fill(0.0);
    processor.input_buffers[1][..num_frames].fill(0.0);

    let converter = TimeConverter::new(sample_rate as f32, bpm);

    let buffer_start = current_position;
    let buffer_end = current_position + num_frames as f64;

    for clip in &track.audio_clips {
        let clip_start_samples = converter.beats_to_samples(clip.start_beat);

        let clip_length_samples = converter.beats_to_samples(clip.length_beats);
        let clip_end_samples = clip_start_samples + clip_length_samples;

        let overlap_start = buffer_start.max(clip_start_samples);
        let overlap_end = buffer_end.min(clip_end_samples);
        if overlap_end <= overlap_start {
            continue;
        }

        let offset_samples = converter.beats_to_samples(clip.offset_beats);

        let frames = (overlap_end - overlap_start) as usize;
        let start_in_buffer = (overlap_start - buffer_start) as usize;

        // For each output frame, sample from clip at its own rate (linear)
        let ratio = clip.sample_rate as f64 / sample_rate; // src_per_dst
        let clip_length_beats = clip.length_beats;
        let fade_in_beats = clip.fade_in.unwrap_or(0.0).max(0.0);
        let fade_out_beats = clip.fade_out.unwrap_or(0.0).max(0.0);

        for i in 0..frames {
            let buf_idx = start_in_buffer + i;
            if buf_idx >= num_frames {
                break;
            }

            // Project sample offset inside the clip window (dst/project domain)
            let proj_off = (overlap_start - clip_start_samples) + i as f64;
            // Source float index (clip domain)
            let src_pos = (proj_off + offset_samples) * ratio;
            let src_idx = src_pos.floor() as usize;
            let frac = (src_pos - src_idx as f64) as f32;

            // Linear interpolation from clip.samples (mono)
            let s0 = clip.samples.get(src_idx).copied().unwrap_or(0.0);
            let s1 = clip.samples.get(src_idx + 1).copied().unwrap_or(s0);
            let mut s = s0 * (1.0 - frac) + s1 * frac;

            // Apply clip gain
            s *= clip.gain;

            // Apply fades (in beats, relative to clip start)
            let clip_pos_beats = converter.samples_to_beats(proj_off);
            // Fade in
            if fade_in_beats > 0.0 && clip_pos_beats < fade_in_beats {
                let f = (clip_pos_beats / fade_in_beats) as f32;
                s *= f.clamp(0.0, 1.0);
            }
            // Fade out
            if fade_out_beats > 0.0 && clip_pos_beats > (clip_length_beats - fade_out_beats) {
                let rem = (clip_length_beats - clip_pos_beats).max(0.0);
                let f = (rem / fade_out_beats) as f32;
                s *= f.clamp(0.0, 1.0);
            }

            processor.input_buffers[0][buf_idx] += s;
            processor.input_buffers[1][buf_idx] += s;
        }
    }
}

fn process_preview_note(
    processor: &mut TrackProcessor,
    preview: &PreviewNote,
    num_frames: usize,
    current_position: f64,
    sample_rate: f64,
) {
    for i in 0..num_frames {
        let sample_pos = current_position + i as f64 - preview.start_position;
        if sample_pos > 0.0 && sample_pos < sample_rate * PREVIEW_NOTE_DURATION {
            let sample = generate_sine_for_note(
                preview.pitch,
                100, // Default preview velocity
                sample_pos,
                sample_rate,
            );
            let envelope = (-(sample_pos * 4.0 / sample_rate)).exp() as f32;

            processor.input_buffers[0][i] += sample * envelope * 3.0; // Boost for preview
            processor.input_buffers[1][i] += sample * envelope * 3.0;
        }
    }
}

fn build_block_midi_events(
    clip: &MidiClipSnapshot,
    block_start_samples: f64,
    frames: usize,
    sample_rate: f64,
    bpm: f32,
    _loop_enabled: bool,
    _loop_start: f64,
    _loop_end: f64,
    transport_jump: bool,
    plugin_active_notes: &mut Vec<(u8, u8)>,
) -> Vec<(u8, u8, u8, i64)> {
    let conv = TimeConverter::new(sample_rate as f32, bpm);

    let block_start_beat = conv.samples_to_beats(block_start_samples);
    let block_end_beat = conv.samples_to_beats(block_start_samples + frames as f64);

    let clip_start = clip.start_beat;
    let clip_end = clip.start_beat + clip.length_beats.max(0.0);

    if block_end_beat <= clip_start || block_start_beat >= clip_end {
        return Vec::new();
    }

    let content_len = clip.content_len_beats.max(0.000001);
    let repeats = if clip.loop_enabled {
        (clip.length_beats / content_len).ceil().max(1.0) as i32
    } else {
        1
    };

    let mut events: Vec<(u8, u8, u8, i64)> = Vec::with_capacity(64);

    for k in 0..repeats {
        let rep_off = clip_start + (k as f64 * content_len);
        let rep_end = (rep_off + content_len).min(clip_end);

        if rep_end <= block_start_beat || rep_off >= block_end_beat {
            continue;
        }

        let offset = clip.content_offset_beats.rem_euclid(content_len);

        for n in &clip.notes {
            let s_loc = (n.start + offset).rem_euclid(content_len);
            let e_loc_raw = s_loc + n.duration;

            let mut segs: smallvec::SmallVec<[(f64, f64); 2]> = smallvec::smallvec![];
            if e_loc_raw <= content_len {
                segs.push((s_loc, e_loc_raw));
            } else {
                segs.push((s_loc, content_len));
                segs.push((0.0, e_loc_raw - content_len));
            }

            for (s_local, e_local) in segs {
                let s_raw = rep_off + s_local;
                let e_raw = (rep_off + e_local).min(rep_end);

                if e_raw <= block_start_beat || s_raw >= block_end_beat {
                    continue;
                }

                let pitch = (n.pitch as i16 + clip.transpose as i16).clamp(0, 127) as u8;
                let vel = (n.velocity as i16 + clip.velocity_offset as i16).clamp(1, 127) as u8;

                let s_q = quantize_beat(s_raw, clip);
                let e_q = quantize_beat(e_raw, clip).max(s_q + 1e-6);

                let start_frame = conv.beats_to_samples(s_q - block_start_beat).round() as i64;
                let end_frame = conv.beats_to_samples(e_q - block_start_beat).round() as i64;

                if transport_jump && s_q < block_start_beat && e_q > block_start_beat {
                    events.push((0x90, pitch, vel, 0));
                }

                if (0..frames as i64).contains(&start_frame) {
                    events.push((0x90, pitch, vel, start_frame));
                }
                if (0..frames as i64).contains(&end_frame) {
                    events.push((0x80, pitch, 0, end_frame));
                }
            }
        }
    }

    events.sort_by_key(|e| e.3);
    update_active_notes(&events, plugin_active_notes);
    events
}

#[inline]
fn quantize_beat(beat: f64, clip: &MidiClipSnapshot) -> f64 {
    if !clip.quantize_enabled || clip.quantize_grid <= 0.0 {
        return beat;
    }
    let g = clip.quantize_grid as f64;
    let q = (beat / g).round() * g;
    let mut q_swing = q;
    if clip.swing.abs() > 0.0001 {
        let idx = (q_swing / (g * 0.5)).round() as i64;
        if idx % 2 != 0 {
            q_swing += (clip.swing as f64) * 0.5 * g;
        }
    }
    beat + (q_swing - beat) * (clip.quantize_strength as f64).clamp(0.0, 1.0)
}

fn update_active_notes(events: &[(u8, u8, u8, i64)], active: &mut Vec<(u8, u8)>) {
    for e in events {
        let ch = e.0 & 0x0F;
        let status = e.0 & 0xF0;
        let key = e.1;
        match status {
            0x90 if e.2 > 0 => {
                if !active.contains(&(ch, key)) {
                    active.push((ch, key));
                }
            }
            0x80 | 0x90 => {
                active.retain(|&(c, k)| c != ch || k != key);
            }
            _ => {}
        }
    }
}

fn process_track_plugins(
    track: &TrackSnapshot,
    processor: &mut TrackProcessor,
    num_frames: usize,
    block_start_samples: f64,
    bpm: f32,
    sample_rate: f64,
    loop_active: bool,
    loop_start_beats: f64,
    loop_end_beats: f64,
    plugin_time_ms_accum: &mut f32,
) {
    let contiguous = (processor.last_block_end_samples - block_start_samples).abs() <= f64::EPSILON;
    let transport_jump = !contiguous;

    let mut all_midi_events: Vec<MidiEvent> = Vec::new();

    // If transport jumped (loop or seek), send note-offs for all active notes
    if transport_jump && !processor.plugin_active_notes.is_empty() {
        for &(channel, key) in &processor.plugin_active_notes {
            all_midi_events.push(MidiEvent {
                status: 0x80 | channel,
                data1: key,
                data2: 0,
                time_frames: 0, // At start of block
            });
        }
        processor.plugin_active_notes.clear();
    }

    if track.is_midi {
        let contiguous =
            (processor.last_block_end_samples - block_start_samples).abs() <= f64::EPSILON;
        let transport_jump = !contiguous;
        for clip in &track.midi_clips {
            let clip_events = build_block_midi_events(
                clip,
                block_start_samples,
                num_frames,
                sample_rate,
                bpm,
                loop_active,
                loop_start_beats,
                loop_end_beats,
                transport_jump,
                &mut processor.plugin_active_notes,
            );
            all_midi_events.extend(clip_events.into_iter().map(|(st, d1, d2, t)| MidiEvent {
                status: st,
                data1: d1,
                data2: d2,
                time_frames: t,
            }));
        }
        all_midi_events.sort_by_key(|e| e.time_frames);
    }

    // Early out if no plugins (leave input buffers intact)
    if processor.plugin_order.is_empty() {
        processor.last_block_end_samples = block_start_samples + num_frames as f64;
        return;
    }

    let mut first_active_plugin = true;

    for &plugin_id in &processor.plugin_order {
        let Some(ppu) = processor.plugins.get_mut(&plugin_id) else {
            continue;
        };
        if ppu.bypass {
            continue;
        }
        let Some(handle) = ppu.rt_instance_id else {
            continue;
        };

        // Apply automation
        for kv in processor.automated_plugin_params.iter() {
            let ((p_id, param_name), value) = (kv.key().clone(), *kv.value());
            if p_id == plugin_id {
                let key = match ppu.backend {
                    BackendKind::Lv2 => ParamKey::Lv2(param_name.clone()),
                    BackendKind::Clap => ppu
                        .param_name_to_key
                        .get(&param_name)
                        .cloned()
                        .unwrap_or(ParamKey::Clap(0)),
                };
                if let Some(mut inst) = PLUGIN_STORE.get_mut(&handle) {
                    inst.set_param(&key, value);
                }
            }
        }

        let in_l = &processor.input_buffers[0][..num_frames];
        let in_r = &processor.input_buffers[1][..num_frames];

        let (left_vecs, right_vecs) = processor.output_buffers.split_at_mut(1);
        let out_l = &mut left_vecs[0][..num_frames];
        let out_r = &mut right_vecs[0][..num_frames];
        out_l.fill(0.0);
        out_r.fill(0.0);

        let inputs: [&[f32]; 2] = [in_l, in_r];
        let mut outputs: [&mut [f32]; 2] = [out_l, out_r];

        let events_slice: &[RtMidiEvent] = if track.is_midi && first_active_plugin {
            &all_midi_events
        } else {
            &[]
        };

        let ctx = ProcessCtx {
            frames: num_frames,
            bpm,
            time_samples: block_start_samples,
            loop_active,
        };

        let t0 = Instant::now();
        if let Some(mut inst) = PLUGIN_STORE.get_mut(&handle) {
            // panic catcher
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let _ = inst.process(&ctx, &inputs, &mut outputs, events_slice);
            }));

            if let Err(e) = result {
                log::error!(
                    "Plugin '{}' (URI: {}) panicked during processing. Bypassing automatically.",
                    ppu.uri, // Using URI as name might not be available here
                    ppu.uri
                );

                ppu.bypass = true;

                // let _ = self.updates.try_send(UIUpdate::Error(format!(
                //     "Plugin {} crashed and has been disabled. Please save your work and restart.",
                //     ppu.uri
                // ))); // Hack passing self
            }
        }
        *plugin_time_ms_accum += t0.elapsed().as_secs_f32() * 1000.0;

        // Feed next plugin
        processor.input_buffers[0][..num_frames]
            .copy_from_slice(&processor.output_buffers[0][..num_frames]);
        processor.input_buffers[1][..num_frames]
            .copy_from_slice(&processor.output_buffers[1][..num_frames]);

        if track.is_midi && first_active_plugin {
            first_active_plugin = false;
        }
    }

    processor.last_block_end_samples = block_start_samples + num_frames as f64;
}

fn value_at_beat_snapshot(lane: &RtAutomationLaneSnapshot, beat: f64) -> f32 {
    if lane.points.is_empty() {
        return 0.0;
    }
    // Find neighbors
    let mut prev = &lane.points[0];
    let mut next = &lane.points[lane.points.len() - 1];
    for p in &lane.points {
        if p.beat <= beat {
            prev = p;
        } else {
            next = p;
            break;
        }
    }
    if (next.beat - prev.beat).abs() < f64::EPSILON {
        return next.value;
    }
    let t = ((beat - prev.beat) / (next.beat - prev.beat)).clamp(0.0, 1.0);
    match next.curve_type {
        RtCurveType::Step => prev.value,
        RtCurveType::Linear => prev.value + ((next.value - prev.value) * t as f32),
        RtCurveType::Exponential => {
            let t2 = (t as f32).powf(2.0);
            prev.value + (next.value - prev.value) * t2
        }
    }
}

fn apply_automation_smooth(
    track: &TrackSnapshot,
    processor: &mut TrackProcessor,
    block_start_samples: f64,
    num_frames: usize,
    converter: &TimeConverter,
) {
    // Reset per-block automation state
    processor.automated_volume = f32::NAN;
    processor.automated_pan = f32::NAN;
    processor.automated_plugin_params.clear();

    let block_start_beat = converter.samples_to_beats(block_start_samples);

    for lane in &track.automation_lanes {
        let has_point_in_block = lane.points.iter().any(|p| {
            let beat = p.beat;
            beat >= block_start_beat
                && beat < converter.samples_to_beats(block_start_samples + num_frames as f64)
        });

        if has_point_in_block {
            // Per-sample automation path
            let param_key = match &lane.parameter {
                RtAutomationTarget::TrackVolume => "volume".to_string(),
                RtAutomationTarget::TrackPan => "pan".to_string(),
                RtAutomationTarget::PluginParam {
                    plugin_id,
                    param_name,
                } => {
                    format!("plugin_{}_{}", plugin_id, param_name)
                }
                _ => continue,
            };

            let buf = processor
                .automation_sample_buffers
                .entry(param_key)
                .or_insert_with(|| vec![0.0; num_frames]);

            if buf.len() < num_frames {
                buf.resize(num_frames, 0.0);
            }

            // Sample automation curve for each frame in the block
            for i in 0..num_frames {
                let beat = converter.samples_to_beats(block_start_samples + i as f64);
                buf[i] = value_at_beat_snapshot(lane, beat);
            }
        } else {
            // Per-block automation path
            let value = value_at_beat_snapshot(lane, block_start_beat);
            match &lane.parameter {
                RtAutomationTarget::TrackVolume => {
                    processor.automated_volume = value;
                }
                RtAutomationTarget::TrackPan => {
                    processor.automated_pan = value * 2.0 - 1.0; // convert 0..1 to -1..1
                }
                RtAutomationTarget::PluginParam {
                    plugin_id,
                    param_name,
                } => {
                    processor
                        .automated_plugin_params
                        .insert((*plugin_id, param_name.clone()), value);
                }
                _ => {}
            }
        }
    }
}

fn debug_print_midi_events(uri: &str, events: &[(u8, u8, u8, i64)]) {
    if !DEBUG_PLUGIN_AUDIO {
        return;
    }
    if events.is_empty() {
        println!("[LV2][{}] MIDI: none this block", uri);
        return;
    }
    let show: Vec<_> = events.iter().take(8).copied().collect();
    println!(
        "[LV2][{}] MIDI: {} events (showing {}): {:?}",
        uri,
        events.len(),
        show.len(),
        show.iter()
            .map(|(st, p, v, t)| (*st, *p, *v, *t))
            .collect::<Vec<_>>()
    );
}

// Compute and print output peaks per plugin run
fn debug_print_output_peak(uri: &str, left: &[f32], right: &[f32]) {
    if !DEBUG_PLUGIN_AUDIO {
        return;
    }
    let lp = left.iter().fold(0.0_f32, |a, &s| a.max(s.abs()));
    let rp = right.iter().fold(0.0_f32, |a, &s| a.max(s.abs()));
    println!("[LV2][{}] OUT peak: L={:.6} R={:.6}", uri, lp, rp);
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // When the engine is dropped, remove all its plugin instances
        for processor in self.track_processors.values() {
            for plugin in processor.plugins.values() {
                if let Some(handle) = plugin.rt_instance_id {
                    PLUGIN_STORE.remove(&handle);
                }
            }
        }
        log::debug!(
            "AudioEngine dropped and cleaned up {} plugin instances.",
            self.track_processors
                .values()
                .map(|p| p.plugins.len())
                .sum::<usize>()
        );
    }
}
