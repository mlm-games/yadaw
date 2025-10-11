use crate::audio_state::{
    AudioState, MidiClipSnapshot, RealtimeCommand, RtAutomationLaneSnapshot, RtAutomationTarget,
    RtCurveType, TrackSnapshot,
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
use parking_lot::RwLock;
use rtrb::{Consumer, RingBuffer};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::model::plugin_api::PluginInstance as UnifiedInstance;

thread_local! {
    static PLUGIN_STORE: std::cell::RefCell<PluginStore> =
        std::cell::RefCell::new(PluginStore { slots: Vec::new() });
}

struct PluginStore {
    slots: Vec<Option<Box<dyn UnifiedInstance>>>,
}

impl PluginStore {
    fn insert(&mut self, inst: Box<dyn UnifiedInstance>) -> usize {
        // reuse holes first
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(inst);
                return i;
            }
        }
        self.slots.push(Some(inst));
        self.slots.len() - 1
    }
    fn get_mut(&mut self, id: usize) -> Option<&mut Box<dyn UnifiedInstance>> {
        self.slots.get_mut(id).and_then(|s| s.as_mut())
    }
    fn remove(&mut self, id: usize) {
        if let Some(slot) = self.slots.get_mut(id) {
            *slot = None;
        }
    }
}

pub(crate) struct AudioEngine {
    tracks: Arc<RwLock<Vec<TrackSnapshot>>>,
    track_order: Arc<RwLock<Vec<u64>>>,
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
    rt_instance_id: Option<usize>,
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
) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("No output device");
    let config = device.default_output_config().expect("No default config");
    let sample_rate = config.sample_rate().0 as f64;
    let channels = config.channels() as usize;
    let track_order = Arc::new(RwLock::new(Vec::new()));

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
    let tracks = Arc::new(RwLock::new(Vec::new()));

    let mut engine = AudioEngine {
        tracks: tracks.clone(),
        track_order: track_order.clone(),
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

        let now_playing = engine.audio_state.playing.load(Ordering::Relaxed);

        // Drain RT commands at block start
        while let Ok(cmd) = realtime_commands.try_recv() {
            engine.process_realtime_command(cmd);
        }

        // Pull new input samples
        while let Ok(sample) = engine.recording_state.recording_consumer.pop() {
            engine.recording_state.monitor_queue.push(sample);
            if engine.recording_state.is_recording {
                engine.recording_state.accumulated_samples.push(sample);
            }
        }

        // Cap monitor queue
        if engine.recording_state.monitor_queue.len() > 2 * MAX_BUFFER_SIZE {
            let drop_n = engine.recording_state.monitor_queue.len() - 2 * MAX_BUFFER_SIZE;
            engine.recording_state.monitor_queue.drain(0..drop_n);
        }

        // Handle recording finalization
        if !engine.audio_state.recording.load(Ordering::Relaxed)
            && engine.recording_state.is_recording
        {
            engine.recording_state.is_recording = false;

            if let Some(track_id) = engine.recording_state.recording_track
                && !engine.recording_state.accumulated_samples.is_empty()
            {
                {
                    let converter =
                        TimeConverter::new(sample_rate as f32, engine.audio_state.bpm.load());
                    let start_beat =
                        converter.samples_to_beats(engine.recording_state.recording_start_position);
                    let end_beat = converter.samples_to_beats(engine.audio_state.get_position());

                    let clip = AudioClip {
                        id: 0, // Will be assigned by UI thread
                        name: format!("Recording {}", chrono::Local::now().format("%H:%M:%S")),
                        start_beat,
                        length_beats: end_beat - start_beat,
                        samples: engine.recording_state.accumulated_samples.clone(),
                        sample_rate: sample_rate as f32,
                        ..Default::default()
                    };

                    let _ = engine
                        .updates
                        .send(UIUpdate::RecordingFinished(track_id, clip));
                    engine.recording_state.accumulated_samples.clear();
                }
            }
        }

        if !now_playing {
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
    fn process_realtime_command(&mut self, cmd: RealtimeCommand) {
        match cmd {
            RealtimeCommand::UpdateTrackVolume(track_id, vol) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.gain = vol;
                }
                let mut tracks_guard = self.tracks.write();
                if let Some(track) = tracks_guard.iter_mut().find(|t| t.track_id == track_id) {
                    track.volume = vol;
                }
            }
            RealtimeCommand::UpdateTrackPan(track_id, pan) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.pan = pan;
                }
                let mut tracks_guard = self.tracks.write();
                if let Some(track) = tracks_guard.iter_mut().find(|t| t.track_id == track_id) {
                    track.pan = pan;
                }
            }
            RealtimeCommand::UpdateTrackMute(track_id, mute) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.mute = mute;
                }
                let mut tracks_guard = self.tracks.write();
                if let Some(track) = tracks_guard.iter_mut().find(|t| t.track_id == track_id) {
                    track.muted = mute;
                }
            }
            RealtimeCommand::UpdateTrackSolo(track_id, solo) => {
                if let Some(strip) = self.channel_strips.get_mut(&track_id) {
                    strip.solo = solo;
                }
                let mut tracks_guard = self.tracks.write();
                if let Some(track) = tracks_guard.iter_mut().find(|t| t.track_id == track_id) {
                    track.solo = solo;
                }
            }

            RealtimeCommand::UpdatePluginBypass(track_id, plugin_id, bypass) => {
                if let Some(proc) = self.track_processors.get_mut(&track_id) {
                    if let Some(plugin) = proc.plugins.get_mut(&plugin_id) {
                        plugin.bypass = bypass;
                    }
                }
            }

            RealtimeCommand::UpdatePluginParam(track_id, plugin_id, param_name, value) => {
                if let Some(proc) = self.track_processors.get_mut(&track_id) {
                    if let Some(plugin) = proc.plugins.get(&plugin_id) {
                        if let Some(rt_id) = plugin.rt_instance_id {
                            let key = match plugin.backend {
                                BackendKind::Lv2 => ParamKey::Lv2(param_name.clone()),
                                BackendKind::Clap => plugin
                                    .param_name_to_key
                                    .get(&param_name)
                                    .cloned()
                                    .unwrap_or(ParamKey::Clap(0)),
                            };
                            PLUGIN_STORE.with(|st| {
                                if let Some(inst) = st.borrow_mut().get_mut(rt_id) {
                                    inst.set_param(&key, value);
                                }
                            });
                        }
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
                        let rt_id = PLUGIN_STORE.with(|st| st.borrow_mut().insert(inst));

                        let plugin = PluginProcessorUnified {
                            plugin_id,
                            rt_instance_id: Some(rt_id),
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
                        if let Some(rt_id) = plugin.rt_instance_id {
                            PLUGIN_STORE.with(|st| st.borrow_mut().remove(rt_id));
                        }
                    }
                    proc.plugin_order.retain(|&id| id != plugin_id);
                }
            }

            RealtimeCommand::UpdateTracks(new_tracks) => {
                *self.tracks.write() = new_tracks.clone();
                *self.track_order.write() = new_tracks.iter().map(|t| t.track_id).collect();
                self.update_track_processors_without_plugins(&new_tracks);
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

    fn process_audio(
        &mut self,
        output: &mut [f32],
        num_frames: usize,
        channels: usize,
        mut current_position: f64,
        plugin_time_ms_accum: &mut f32,
    ) -> f64 {
        let tracks_guard = self.tracks.read();
        let tracks: &Vec<TrackSnapshot> = &*tracks_guard;
        let track_order = self.track_order.read().clone();

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
            let block_start_samp = current_position;
            let block_start_beat = converter.samples_to_beats(block_start_samp);

            // How many frames remain before loop end? Round up so 0<remain<1 -> 1 frame.
            let frames_to_loop_end = if loop_active && block_start_samp < loop_end_samp {
                let remain = loop_end_samp - block_start_samp;
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

            for &track_id in &track_order {
                let track = match tracks.iter().find(|t| t.track_id == track_id) {
                    Some(t) => t,
                    None => continue,
                };

                if track.muted || (any_track_soloed && !track.solo) {
                    continue;
                }

                if let Some(processor) = self.track_processors.get_mut(&track_id) {
                    apply_automation(track, processor, block_start_beat);

                    // Build per-track buffers (post-clip, pre-plugin)
                    if track.is_midi {
                        process_midi_track(
                            track,
                            processor,
                            frames_to_process,
                            block_start_samp,
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
                            block_start_samp,
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
                                    block_start_samp,
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
                        block_start_samp, // pass samples for block start
                        bpm,
                        self.sample_rate,
                        loop_active,
                        loop_start_beats,
                        loop_end_beats,
                        plugin_time_ms_accum,
                    );

                    // Mix into output and compute per-track peaks (post pan/track volume)
                    let (left_gain, right_gain) = effective_gains(track, processor);

                    let mut tp_l = 0.0f32;
                    let mut tp_r = 0.0f32;

                    for i in 0..frames_to_process {
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
                if let Some(id) = ppu.rt_instance_id {
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
                    PLUGIN_STORE.with(|st| {
                        if let Some(inst) = st.borrow_mut().get_mut(id) {
                            let _ = inst.process(&ctx, &inputs, &mut outputs, &panic_events);
                        }
                    });
                }
            }
            proc.active_notes.clear();
            proc.last_pattern_position = 0.0;
            proc.pattern_loop_count = 0;
            proc.notes_triggered_this_loop.clear();
            proc.last_block_end_samples = 0.0;
        }
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
        let clip_end_samples = clip_start_samples
            + clip.samples.len() as f64 * (sample_rate / clip.sample_rate as f64);

        // Intersect with current buffer (all in project-rate samples)
        let overlap_start = buffer_start.max(clip_start_samples);
        let overlap_end = buffer_end.min(clip_end_samples);
        if overlap_end <= overlap_start {
            continue;
        }

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
            let src_pos = proj_off * ratio;
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
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,
    transport_jump: bool,
    plugin_active_notes: &mut Vec<(u8, u8)>,
) -> Vec<(u8, u8, u8, i64)> {
    let conv = TimeConverter::new(sample_rate as f32, bpm);

    let block_start_beat = conv.samples_to_beats(block_start_samples);
    let block_end_beat = conv.samples_to_beats(block_start_samples + frames as f64);

    // Effective start/end with loop range (project loop)
    let eff_start = if loop_enabled && loop_end > loop_start && block_start_beat >= loop_end {
        let len = loop_end - loop_start;
        loop_start + ((block_start_beat - loop_start) % len)
    } else {
        block_start_beat
    };
    let eff_end = if loop_enabled && loop_end > loop_start && block_end_beat >= loop_end {
        let len = loop_end - loop_start;
        loop_start + ((block_end_beat - loop_start) % len)
    } else {
        block_end_beat
    };

    // Clip instance window
    let clip_start = clip.start_beat;
    let clip_end = clip.start_beat + clip.length_beats.max(0.0);

    // Fast reject vs block
    if eff_end <= clip_start || eff_start >= clip_end {
        return Vec::new();
    }

    // Number of repeats that can fit into instance length
    let content_len = clip.content_len_beats.max(0.000001);
    let repeats = if clip.loop_enabled {
        (clip.length_beats / content_len).ceil().max(1.0) as i32
    } else {
        1
    };

    // Helper: quantize a beat non-destructively
    let quantize = |beat: f64| -> f64 {
        if !clip.quantize_enabled || clip.quantize_grid <= 0.0 {
            return beat;
        }
        let g = clip.quantize_grid as f64;
        let q = (beat / g).round() * g;

        // swing: shift odd subdivisions by +/- swing*0.5*g
        let mut q_swing = q;
        if clip.swing.abs() > 0.0001 {
            let idx = (q_swing / (g * 0.5)).round() as i64;
            if idx % 2 != 0 {
                q_swing += (clip.swing as f64) * 0.5 * g;
            }
        }

        // strength blend
        beat + (q_swing - beat) * (clip.quantize_strength as f64).clamp(0.0, 1.0)
    };

    // Build events
    let mut events: Vec<(u8, u8, u8, i64)> = Vec::new();
    for k in 0..repeats {
        let rep_off = clip_start + (k as f64 * content_len);

        // End of this repeat in project beats (clamped to instance end)
        let rep_end = (rep_off + content_len).min(clip_end);

        // Skip if outside block
        if rep_end <= eff_start || rep_off >= eff_end {
            continue;
        }

        let offset = clip.content_offset_beats.rem_euclid(content_len);
        for n in &clip.notes {
            // local start/end with offset, modulo content_len
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

                // Global transforms
                let pitch = (n.pitch as i16 + clip.transpose as i16).clamp(0, 127) as u8;
                let vel = (n.velocity as i16 + clip.velocity_offset as i16).clamp(1, 127) as u8;

                // Quantize start/end (separately)
                let s_q = quantize(s_raw);
                let e_q = quantize(e_raw).max(s_q + 1e-6);

                // Convert to frames inside this audio block
                let start_frame = conv.beats_to_samples(s_q - eff_start).round() as i64;
                let end_frame = conv.beats_to_samples(e_q - eff_start).round() as i64;

                // Note chase on transport jump (send Note On at t=0 if block lands inside a sustaining note)
                if transport_jump && s_q < eff_start && e_q > eff_start {
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
    let mut new_active = plugin_active_notes.clone();

    for event in &events {
        let channel = event.0 & 0x0F;
        let status = event.0 & 0xF0;
        let key = event.1;

        match status {
            0x90 if event.2 > 0 => {
                // Note on - add to active
                if !new_active.contains(&(channel, key)) {
                    new_active.push((channel, key));
                }
            }
            0x80 | 0x90 => {
                // Note off - remove from active
                new_active.retain(|&(ch, k)| ch != channel || k != key);
            }
            _ => {}
        }
    }

    *plugin_active_notes = new_active;
    events
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

    // Build MIDI events for this block if it's a MIDI track
    let mut all_midi_events: Vec<MidiEvent> = Vec::new();
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

        let Some(rt_id) = ppu.rt_instance_id else {
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
                PLUGIN_STORE.with(|st| {
                    if let Some(inst) = st.borrow_mut().get_mut(rt_id) {
                        inst.set_param(&key, value);
                    }
                });
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
        PLUGIN_STORE.with(|st| {
            if let Some(inst) = st.borrow_mut().get_mut(rt_id) {
                let _ = inst.process(&ctx, &inputs, &mut outputs, events_slice);
            }
        });
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

fn apply_automation(
    track: &crate::audio_state::TrackSnapshot,
    processor: &mut TrackProcessor,
    current_beat: f64,
) {
    for lane in &track.automation_lanes {
        let value = value_at_beat_snapshot(lane, current_beat);
        match &lane.parameter {
            RtAutomationTarget::TrackVolume => {
                processor.automated_volume = value;
            }
            RtAutomationTarget::TrackPan => {
                processor.automated_pan = value * 2.0 - 1.0;
            }
            RtAutomationTarget::PluginParam {
                plugin_id,
                param_name,
            } => {
                processor
                    .automated_plugin_params
                    .insert((*plugin_id, param_name.clone()), value);
            }
            RtAutomationTarget::TrackSend(_) => {
                // TODO: implement
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
