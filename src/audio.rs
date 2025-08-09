use crate::audio_state::{AudioState, RealtimeCommand, TrackSnapshot};
use crate::automation_lane::AutomationLaneWidget;
use crate::lv2_plugin_host::{LV2PluginHost, LV2PluginInstance};
use crate::state::{AudioClip, AutomationTarget, UIUpdate};
use core::f32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rtrb::{Consumer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::Ordering;

const MAX_BUFFER_SIZE: usize = 8192;
const RECORDING_BUFFER_SIZE: usize = 44100 * 60 * 5; // 5 minutes at 44.1kHz

static PLUGIN_HOST: Lazy<parking_lot::Mutex<Option<LV2PluginHost>>> =
    Lazy::new(|| parking_lot::Mutex::new(None));

struct AudioEngine {
    tracks: Arc<RwLock<Vec<TrackSnapshot>>>,
    audio_state: Arc<AudioState>,
    track_processors: Vec<TrackProcessor>,
    recording_state: RecordingState,
    preview_note: Option<PreviewNote>,
    sample_rate: f64,
    updates: Sender<UIUpdate>,
}

struct TrackProcessor {
    id: usize,
    plugins: Vec<PluginProcessor>,
    input_buffer_l: Vec<f32>,
    input_buffer_r: Vec<f32>,
    output_buffer_l: Vec<f32>,
    output_buffer_r: Vec<f32>,
    active_notes: Vec<ActiveMidiNote>,
    last_pattern_position: f64,
    automated_volume: f32,
    automated_pan: f32,
    automated_plugin_params: DashMap<(usize, String), f32>,
}

struct PluginProcessor {
    instance: Option<LV2PluginInstance>,
    uri: String,
    bypass: bool,
}

#[derive(Clone)]
struct ActiveMidiNote {
    pitch: u8,
    velocity: u8,
    start_sample: f64,
}

struct PreviewNote {
    track_id: usize,
    pitch: u8,
    start_position: f64,
}

struct RecordingState {
    is_recording: bool,
    recording_track: Option<usize>,
    recording_consumer: Consumer<f32>,
    recording_start_position: f64,
    accumulated_samples: Vec<f32>,
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

    // Update audio state
    audio_state.sample_rate.store(sample_rate as f32);

    // Initialize plugin host
    {
        let mut host_lock = PLUGIN_HOST.lock();
        *host_lock = Some(
            LV2PluginHost::new(sample_rate, MAX_BUFFER_SIZE).expect("Failed to create plugin host"),
        );
    }

    // Create recording buffer
    let (recording_producer, recording_consumer) = RingBuffer::<f32>::new(RECORDING_BUFFER_SIZE);

    // Initialize engine
    let tracks = Arc::new(RwLock::new(Vec::new()));
    let mut engine = AudioEngine {
        tracks: tracks.clone(),
        audio_state: audio_state.clone(),
        track_processors: Vec::new(),
        recording_state: RecordingState {
            is_recording: false,
            recording_track: None,
            recording_consumer,
            recording_start_position: 0.0,
            accumulated_samples: Vec::new(),
        },
        preview_note: None,
        sample_rate,
        updates: updates.clone(),
    };

    // Start recording input thread if available
    let recording_producer = Arc::new(parking_lot::Mutex::new(recording_producer));
    let audio_state_clone = audio_state.clone();
    let updates_clone = updates.clone();

    std::thread::spawn(move || {
        let host = cpal::default_host();
        if let Some(input_device) = host.default_input_device() {
            if let Ok(input_config) = input_device.default_input_config() {
                let channels = input_config.channels() as usize;
                let recording_producer = recording_producer.clone();

                let input_callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if audio_state_clone.recording.load(Ordering::Relaxed) {
                        // Convert to mono and push to ring buffer
                        let mut producer = recording_producer.lock();
                        for frame in data.chunks(channels) {
                            let mono_sample = frame.iter().sum::<f32>() / channels as f32;
                            let _ = producer.push(mono_sample);

                            // Send level update
                            let level = mono_sample.abs();
                            let _ = updates_clone.try_send(UIUpdate::RecordingLevel(level));
                        }
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
                    std::thread::park(); // Keep thread alive
                }
            }
        }
    });

    // Audio callback
    let audio_callback = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let num_frames = data.len() / channels;

        // Process realtime commands
        while let Ok(cmd) = realtime_commands.try_recv() {
            engine.process_realtime_command(cmd);
        }

        // Handle recording
        if engine.audio_state.recording.load(Ordering::Relaxed) {
            if !engine.recording_state.is_recording {
                // Start recording
                engine.recording_state.is_recording = true;
                engine.recording_state.recording_start_position = engine.audio_state.get_position();
                engine.recording_state.accumulated_samples.clear();
            }

            // Collect recorded samples
            while let Ok(sample) = engine.recording_state.recording_consumer.pop() {
                engine.recording_state.accumulated_samples.push(sample);
            }
        } else if engine.recording_state.is_recording {
            // Stop recording and create audio clip
            engine.recording_state.is_recording = false;

            if let Some(track_id) = engine.recording_state.recording_track {
                if !engine.recording_state.accumulated_samples.is_empty() {
                    let start_beat = (engine.recording_state.recording_start_position
                        / sample_rate)
                        * (engine.audio_state.bpm.load() as f64 / 60.0);
                    let end_beat = (engine.audio_state.get_position() / sample_rate)
                        * (engine.audio_state.bpm.load() as f64 / 60.0);

                    let clip = AudioClip {
                        name: format!("Recording {}", chrono::Local::now().format("%H:%M:%S")),
                        start_beat,
                        length_beats: end_beat - start_beat,
                        samples: engine.recording_state.accumulated_samples.clone(),
                        sample_rate: sample_rate as f32,
                    };

                    let _ = engine
                        .updates
                        .send(UIUpdate::RecordingFinished(track_id, clip));
                    engine.recording_state.accumulated_samples.clear();
                }
            }
        }

        // Clear output buffer
        data.fill(0.0);

        // Check if playing
        if !engine.audio_state.playing.load(Ordering::Relaxed) {
            return;
        }

        // Get current position
        let current_position = engine.audio_state.get_position();

        // Process audio
        engine.process_audio(data, num_frames, channels, current_position);

        // Update position
        engine
            .audio_state
            .set_position(current_position + num_frames as f64);

        // Send position update
        let _ = engine
            .updates
            .try_send(UIUpdate::Position(engine.audio_state.get_position()));
    };

    // Create and start audio stream
    let stream = device
        .build_output_stream(
            &config.into(),
            audio_callback,
            |err| eprintln!("Audio stream error: {}", err),
            None,
        )
        .expect("Failed to create audio stream");

    stream.play().expect("Failed to start audio stream");

    // Keep thread alive
    std::thread::park();
}

impl AudioEngine {
    fn process_realtime_command(&mut self, cmd: RealtimeCommand) {
        match cmd {
            RealtimeCommand::UpdateTracks(new_tracks) => {
                *self.tracks.write() = new_tracks.clone();
                self.update_track_processors(&new_tracks);
            }

            RealtimeCommand::UpdateTrackVolume(id, vol) => {
                if let Some(track) = self.tracks.write().get_mut(id) {
                    track.volume = vol;
                }
            }

            RealtimeCommand::UpdateTrackPan(id, pan) => {
                if let Some(track) = self.tracks.write().get_mut(id) {
                    track.pan = pan;
                }
            }

            RealtimeCommand::UpdateTrackMute(id, mute) => {
                if let Some(track) = self.tracks.write().get_mut(id) {
                    track.muted = mute;
                }
            }

            RealtimeCommand::UpdateTrackSolo(id, solo) => {
                if let Some(track) = self.tracks.write().get_mut(id) {
                    track.solo = solo;
                }
            }

            RealtimeCommand::UpdatePluginBypass(track_id, plugin_idx, bypass) => {
                if let Some(processor) = self.track_processors.get_mut(track_id) {
                    if let Some(plugin) = processor.plugins.get_mut(plugin_idx) {
                        plugin.bypass = bypass;
                    }
                }
            }

            RealtimeCommand::UpdatePluginParam(track_id, plugin_idx, param_name, value) => {
                if let Some(processor) = self.track_processors.get_mut(track_id) {
                    if let Some(plugin) = processor.plugins.get_mut(plugin_idx) {
                        if let Some(instance) = &mut plugin.instance {
                            instance.set_parameter(&param_name, value);
                        }
                    }
                }
            }

            RealtimeCommand::PreviewNote(track_id, pitch, start_position) => {
                self.preview_note = Some(PreviewNote {
                    track_id,
                    pitch,
                    start_position,
                });
            }

            RealtimeCommand::StopPreviewNote => {
                self.preview_note = None;
            }
        }
    }

    fn update_track_processors(&mut self, tracks: &[TrackSnapshot]) {
        // Ensure we have enough processors
        while self.track_processors.len() < tracks.len() {
            self.track_processors.push(TrackProcessor {
                id: self.track_processors.len(),
                plugins: Vec::new(),
                input_buffer_l: Vec::with_capacity(MAX_BUFFER_SIZE),
                input_buffer_r: Vec::with_capacity(MAX_BUFFER_SIZE),
                output_buffer_l: Vec::with_capacity(MAX_BUFFER_SIZE),
                output_buffer_r: Vec::with_capacity(MAX_BUFFER_SIZE),
                active_notes: Vec::new(),
                last_pattern_position: 0.0,
                automated_volume: f32::NAN,
                automated_pan: f32::NAN,
                automated_plugin_params: DashMap::new(),
            });
        }

        // Update plugin processors
        for (track_idx, track) in tracks.iter().enumerate() {
            if let Some(processor) = self.track_processors.get_mut(track_idx) {
                // Update plugins if changed
                if processor.plugins.len() != track.plugin_chain.len() {
                    processor.plugins.clear();
                    let host_lock = PLUGIN_HOST.lock();
                    if let Some(host) = host_lock.as_ref() {
                        for plugin_desc in &track.plugin_chain {
                            let instance = host.instantiate_plugin(&plugin_desc.uri).ok();

                            // Set initial parameter values
                            if let Some(inst) = &instance {
                                for entry in plugin_desc.params.iter() {
                                    inst.get_params()
                                        .insert(entry.key().clone(), *entry.value());
                                }
                            }

                            let plugin_processor = PluginProcessor {
                                instance,
                                uri: plugin_desc.uri.clone(),
                                bypass: plugin_desc.bypass,
                            };
                            processor.plugins.push(plugin_processor);
                        }
                    }
                }
            }
        }
    }

    fn process_audio(
        &mut self,
        output: &mut [f32],
        num_frames: usize,
        channels: usize,
        current_position: f64,
    ) {
        let tracks = self.tracks.read().clone();
        let bpm = self.audio_state.bpm.load();
        let master_volume = self.audio_state.master_volume.load();

        let frames_to_process = num_frames.min(MAX_BUFFER_SIZE);

        // Calculate current beat for automation
        let current_beat = (current_position / self.sample_rate) * (bpm as f64 / 60.0);

        // Check for soloed tracks
        let any_track_soloed = tracks.iter().any(|t| t.solo);

        // Collect track levels for meters
        let mut track_levels = Vec::new();

        // Set recording track if recording
        if self.recording_state.is_recording {
            self.recording_state.recording_track =
                tracks.iter().position(|t| t.armed && !t.is_midi);
        }

        // Process each track
        for (track_idx, track) in tracks.iter().enumerate() {
            if track.muted || (any_track_soloed && !track.solo) {
                track_levels.push((0.0, 0.0));
                continue;
            }

            if let Some(processor) = self.track_processors.get_mut(track_idx) {
                // Ensure buffers are large enough
                if processor.input_buffer_l.len() < frames_to_process {
                    processor.input_buffer_l.resize(frames_to_process, 0.0);
                    processor.input_buffer_r.resize(frames_to_process, 0.0);
                    processor.output_buffer_l.resize(frames_to_process, 0.0);
                    processor.output_buffer_r.resize(frames_to_process, 0.0);
                }

                // Clear input buffers
                processor.input_buffer_l[..frames_to_process].fill(0.0);
                processor.input_buffer_r[..frames_to_process].fill(0.0);

                apply_automation(track, processor, current_beat);

                // Use automated values with fallback
                let final_volume =
                    if !processor.automated_volume.is_nan() && processor.automated_volume > 0.0 {
                        processor.automated_volume
                    } else {
                        track.volume
                    };

                let final_pan = if !processor.automated_pan.is_nan() {
                    processor.automated_pan
                } else {
                    track.pan
                };

                // Use final_volume and final_pan for mixing
                let (left_gain, right_gain) = calculate_stereo_pan(final_volume, final_pan);

                // Process track content
                if track.is_midi {
                    process_midi_track(
                        track,
                        processor,
                        frames_to_process,
                        current_position,
                        bpm,
                        self.sample_rate,
                    );
                } else {
                    process_audio_track(
                        track,
                        processor,
                        frames_to_process,
                        current_position,
                        bpm,
                        self.sample_rate,
                    );
                }

                // Process preview note if on this track
                if let Some(preview) = &self.preview_note {
                    if preview.track_id == track_idx {
                        process_preview_note(
                            processor,
                            preview,
                            frames_to_process,
                            current_position,
                            self.sample_rate,
                        );
                    }
                }

                // Process plugins
                process_track_plugins(track, processor, frames_to_process);

                // Calculate levels for meters
                let left_peak = processor.input_buffer_l[..frames_to_process]
                    .iter()
                    .map(|s| s.abs())
                    .fold(0.0f32, f32::max);
                let right_peak = processor.input_buffer_r[..frames_to_process]
                    .iter()
                    .map(|s| s.abs())
                    .fold(0.0f32, f32::max);
                track_levels.push((left_peak, right_peak));

                for i in 0..frames_to_process {
                    output[i * channels] += processor.input_buffer_l[i] * left_gain;
                    if channels > 1 {
                        output[i * channels + 1] += processor.input_buffer_r[i] * right_gain;
                    }
                }
            }
        }

        // Apply master volume and calculate master levels
        let mut master_left_peak = 0.0f32;
        let mut master_right_peak = 0.0f32;

        for i in 0..num_frames {
            let left = output[i * channels] * master_volume;
            let right = if channels > 1 {
                output[i * channels + 1] * master_volume
            } else {
                left
            };

            output[i * channels] = left.clamp(-1.0, 1.0);
            if channels > 1 {
                output[i * channels + 1] = right.clamp(-1.0, 1.0);
            }

            master_left_peak = master_left_peak.max(left.abs());
            master_right_peak = master_right_peak.max(right.abs());
        }

        // Send level updates
        let _ = self.updates.try_send(UIUpdate::TrackLevels(track_levels));
        let _ = self
            .updates
            .try_send(UIUpdate::MasterLevel(master_left_peak, master_right_peak));
    }
}

fn calculate_stereo_pan(volume: f32, pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) / 2.0;
    let angle = p * std::f32::consts::FRAC_PI_2;
    (volume * angle.cos(), volume * angle.sin())
}

fn process_midi_track(
    track: &TrackSnapshot,
    processor: &mut TrackProcessor,
    num_frames: usize,
    current_position: f64,
    bpm: f32,
    sample_rate: f64,
) {
    if let Some(pattern) = track.patterns.first() {
        let current_beat = (current_position / sample_rate) * (bpm as f64 / 60.0);
        let pattern_position = current_beat % pattern.length;

        // Check for new notes to trigger
        for note in &pattern.notes {
            let note_start = note.start;
            let note_end = note.start + note.duration;

            // Trigger note on
            if pattern_position >= note_start && processor.last_pattern_position < note_start {
                processor.active_notes.push(ActiveMidiNote {
                    pitch: note.pitch,
                    velocity: note.velocity,
                    start_sample: current_position,
                });
            }

            // Trigger note off
            if pattern_position >= note_end && processor.last_pattern_position < note_end {
                processor.active_notes.retain(|n| n.pitch != note.pitch);
            }
        }

        // Handle pattern loop
        if pattern_position < processor.last_pattern_position {
            processor.active_notes.clear();
            for note in &pattern.notes {
                if note.start < 0.1 {
                    processor.active_notes.push(ActiveMidiNote {
                        pitch: note.pitch,
                        velocity: note.velocity,
                        start_sample: current_position,
                    });
                }
            }
        }

        processor.last_pattern_position = pattern_position;
    }

    // Generate audio for active notes (simple sine wave if no plugins)
    if processor.plugins.is_empty() {
        for i in 0..num_frames {
            let mut sample = 0.0;

            for note in &processor.active_notes {
                let freq = 440.0 * 2.0_f32.powf((note.pitch as f32 - 69.0) / 12.0);
                let phase = ((current_position + i as f64 - note.start_sample) * freq as f64
                    / sample_rate)
                    % 1.0;
                sample += (phase * 2.0 * std::f64::consts::PI).sin() as f32
                    * (note.velocity as f32 / 127.0)
                    * 0.1;
            }

            processor.input_buffer_l[i] = sample;
            processor.input_buffer_r[i] = sample;
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
    let buffer_start_abs = current_position;
    let buffer_end_abs = buffer_start_abs + num_frames as f64;

    for clip in &track.audio_clips {
        let clip_start_abs = (clip.start_beat * 60.0 / bpm as f64) * sample_rate;
        let clip_end_abs = clip_start_abs + (clip.length_beats * 60.0 / bpm as f64) * sample_rate;

        let overlap_start = buffer_start_abs.max(clip_start_abs);
        let overlap_end = buffer_end_abs.min(clip_end_abs);

        if overlap_start < overlap_end {
            let start_index = (overlap_start - buffer_start_abs) as usize;
            let end_index = (overlap_end - buffer_start_abs) as usize;
            let clip_start_offset = (overlap_start - clip_start_abs) as usize;

            for i in start_index..end_index {
                let clip_index = clip_start_offset + (i - start_index);
                if clip_index < clip.samples.len() {
                    let sample = clip.samples[clip_index];
                    processor.input_buffer_l[i] += sample;
                    processor.input_buffer_r[i] += sample;
                }
            }
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
        if sample_pos > 0.0 && sample_pos < sample_rate * 0.5 {
            let freq = 440.0 * 2.0_f32.powf((preview.pitch as f32 - 69.0) / 12.0);
            let phase = (sample_pos * freq as f64 / sample_rate) % 1.0;
            let envelope = (-(sample_pos * 4.0)).exp() as f32;
            let sample = (phase * 2.0 * std::f64::consts::PI).sin() as f32 * envelope * 0.3;

            processor.input_buffer_l[i] += sample;
            processor.input_buffer_r[i] += sample;
        }
    }
}

fn process_track_plugins(track: &TrackSnapshot, processor: &mut TrackProcessor, num_frames: usize) {
    // If no plugins, nothing to do
    if processor.plugins.is_empty() {
        return;
    }

    // Prepare MIDI events for MIDI tracks
    let midi_notes: Vec<(u8, u8, i64)> = if track.is_midi {
        processor
            .active_notes
            .iter()
            .map(|note| (note.pitch, note.velocity, 0i64))
            .collect()
    } else {
        Vec::new()
    };

    // Clear output buffers at the start
    processor.output_buffer_l[..num_frames].fill(0.0);
    processor.output_buffer_r[..num_frames].fill(0.0);

    for (plugin_idx, plugin_desc) in track.plugin_chain.iter().enumerate() {
        if plugin_desc.bypass {
            continue;
        }

        if let Some(plugin) = processor.plugins.get_mut(plugin_idx) {
            if let Some(instance) = &mut plugin.instance {
                // Update parameters - make sure they're actually applied
                for entry in plugin_desc.params.iter() {
                    instance.set_parameter(entry.key(), *entry.value());
                }

                // For the first plugin in chain, prepare MIDI if needed
                if plugin_idx == 0 && !midi_notes.is_empty() {
                    instance.prepare_midi_events(&midi_notes);
                } else {
                    // Clear MIDI for subsequent plugins
                    instance.clear_midi_events();
                }

                // Clear output buffers before each plugin processes
                processor.output_buffer_l[..num_frames].fill(0.0);
                processor.output_buffer_r[..num_frames].fill(0.0);

                // Process audio
                match instance.process(
                    &processor.input_buffer_l[..num_frames],
                    &processor.input_buffer_r[..num_frames],
                    &mut processor.output_buffer_l[..num_frames],
                    &mut processor.output_buffer_r[..num_frames],
                    num_frames,
                ) {
                    Ok(_) => {
                        // Success - copy output to input for next plugin
                        processor.input_buffer_l[..num_frames]
                            .copy_from_slice(&processor.output_buffer_l[..num_frames]);
                        processor.input_buffer_r[..num_frames]
                            .copy_from_slice(&processor.output_buffer_r[..num_frames]);
                    }
                    Err(e) => {
                        // Error - audio remains in input buffers
                        eprintln!("Plugin {} processing error: {}", plugin_desc.uri, e);
                    }
                }
            }
        }
    }
}

fn apply_automation(track: &TrackSnapshot, processor: &mut TrackProcessor, current_beat: f64) {
    for lane in &track.automation_lanes {
        let value = AutomationLaneWidget::get_value_at_beat(lane, current_beat);

        match &lane.parameter {
            AutomationTarget::TrackVolume => {
                processor.automated_volume = value;
            }
            AutomationTarget::TrackPan => {
                processor.automated_pan = value * 2.0 - 1.0; // Convert 0-1 to -1 to 1
            }
            AutomationTarget::PluginParam {
                plugin_idx,
                param_name,
            } => {
                processor
                    .automated_plugin_params
                    .insert((*plugin_idx, param_name.clone()), value);
            }
        }
    }
}
