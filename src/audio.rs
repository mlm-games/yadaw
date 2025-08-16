use crate::audio_state::{AudioState, RealtimeCommand, TrackSnapshot};
use crate::audio_utils::calculate_stereo_gains;
use crate::automation_lane::AutomationLaneWidget;
use crate::constants::{
    DEBUG_PLUGIN_AUDIO, MAX_BUFFER_SIZE, PREVIEW_NOTE_AMPLITUDE, PREVIEW_NOTE_DURATION,
    RECORDING_BUFFER_SIZE, SINE_WAVE_AMPLITUDE,
};
use crate::lv2_plugin_host::{LV2PluginHost, LV2PluginInstance};
use crate::midi_utils::{generate_sine_for_note, MidiNoteUtils};
use crate::mixer::{ChannelStrip, MixerEngine};
use crate::state::{AudioClip, AutomationTarget, UIUpdate};
use crate::time_utils::TimeConverter;
use core::f32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rtrb::{Consumer, RingBuffer};
use std::sync::atomic::Ordering;
use std::sync::Arc;

static PLUGIN_HOST: Lazy<parking_lot::Mutex<Option<LV2PluginHost>>> =
    Lazy::new(|| parking_lot::Mutex::new(None));

pub(crate) struct AudioEngine {
    tracks: Arc<RwLock<Vec<TrackSnapshot>>>,
    audio_state: Arc<AudioState>,
    track_processors: Vec<TrackProcessor>,
    recording_state: RecordingState,
    preview_note: Option<PreviewNote>,
    sample_rate: f64,
    updates: Sender<UIUpdate>,
    mixer: MixerEngine,
    channel_strips: Vec<ChannelStrip>,
    // last_playing: bool,
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
    pattern_loop_count: u32,
    notes_triggered_this_loop: Vec<u8>,
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
        mixer: MixerEngine::new(),
        channel_strips: Vec::new(),
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
                    let converter =
                        TimeConverter::new(sample_rate as f32, engine.audio_state.bpm.load());
                    let start_beat =
                        converter.samples_to_beats(engine.recording_state.recording_start_position);
                    let end_beat = converter.samples_to_beats(engine.audio_state.get_position());

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
            engine.midi_panic();

            // Reset pattern positions when stopped
            for processor in &mut engine.track_processors {
                processor.last_pattern_position = 0.0;
                processor.pattern_loop_count = 0;
                processor.notes_triggered_this_loop.clear();
            }
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
                pattern_loop_count: 0,
                notes_triggered_this_loop: Vec::new(),
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
                            let mut instance = host.instantiate_plugin(&plugin_desc.uri).ok();

                            if let Some(inst) = instance.as_mut() {
                                for entry in plugin_desc.params.iter() {
                                    inst.get_params()
                                        .insert(entry.key().clone(), *entry.value());
                                }
                                // Bind the instance's param cache to the same Arc as the snapshot
                                inst.set_params_arc(plugin_desc.params.clone());
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

        while self.channel_strips.len() < tracks.len() {
            self.channel_strips.push(ChannelStrip::default());
        }

        // Update channel strips from track data
        for (idx, track) in tracks.iter().enumerate() {
            if let Some(strip) = self.channel_strips.get_mut(idx) {
                strip.gain = track.volume;
                strip.pan = track.pan;
                strip.mute = track.muted;
                strip.solo = track.solo;
            }
        }
    }

    fn get_converter(&self) -> TimeConverter {
        TimeConverter::new(
            self.audio_state.sample_rate.load(),
            self.audio_state.bpm.load(),
        )
    }

    fn process_audio(
        &mut self,
        output: &mut [f32],
        num_frames: usize,
        channels: usize,
        current_position: f64,
    ) {
        let start_time = std::time::Instant::now();
        let tracks = self.tracks.read().clone();
        let bpm = self.audio_state.bpm.load();
        let master_volume = self.audio_state.master_volume.load();

        let frames_to_process = num_frames.min(MAX_BUFFER_SIZE);

        // Use converter for time calculations
        let converter = TimeConverter::new(self.sample_rate as f32, bpm);
        let current_beat = converter.samples_to_beats(current_position);

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
                let (left_gain, right_gain) = calculate_stereo_gains(final_volume, final_pan);

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
                process_track_plugins(
                    track,
                    processor,
                    frames_to_process,
                    current_position,
                    bpm,
                    self.sample_rate,
                );

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
            // Track performance
            let processing_time = start_time.elapsed();
            let cpu_usage =
                processing_time.as_secs_f32() / (num_frames as f32 / self.sample_rate as f32);

            // Send metric update
            let _ = self.updates.send(UIUpdate::PerformanceMetric {
                cpu_usage,
                buffer_fill: 0.9, // Calculate actual buffer health
                xruns: 0,         // Track actual xruns
            });
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

    fn midi_panic(&mut self) {
        for processor in self.track_processors.iter_mut() {
            // Send note off for all active notes
            if !processor.active_notes.is_empty() {
                let mut events: Vec<(u8, u8, u8, i64)> = Vec::new();

                for note in &processor.active_notes {
                    events.push((0x80, note.pitch, 0, 0)); // Note off at frame 0
                }

                // Send to all plugins in the chain
                for plugin in processor.plugins.iter_mut() {
                    if let Some(instance) = &mut plugin.instance {
                        instance.prepare_midi_raw_events(&events);
                        // Process a small buffer to ensure the events are handled
                        let mut dummy_l = vec![0.0f32; 64];
                        let mut dummy_r = vec![0.0f32; 64];
                        let _ = instance.process(
                            &mut dummy_l.clone(),
                            &mut dummy_r.clone(),
                            &mut dummy_l,
                            &mut dummy_r,
                            64,
                        );
                    }
                }
            }

            // Clear state
            processor.active_notes.clear();
            processor.last_pattern_position = 0.0;
            processor.pattern_loop_count = 0;
            processor.notes_triggered_this_loop.clear();

            // Send All Notes Off and All Sound Off
            let panic_events: Vec<(u8, u8, u8, i64)> = (0..16)
                .flat_map(|ch| {
                    vec![
                        (0xB0 | ch, 123, 0, 0), // All Notes Off
                        (0xB0 | ch, 120, 0, 0), // All Sound Off
                    ]
                })
                .collect();

            for plugin in processor.plugins.iter_mut() {
                if let Some(instance) = &mut plugin.instance {
                    instance.prepare_midi_raw_events(&panic_events);
                    let mut dummy_l = vec![0.0f32; 64];
                    let mut dummy_r = vec![0.0f32; 64];
                    let _ = instance.process(
                        &mut dummy_l.clone(),
                        &mut dummy_r.clone(),
                        &mut dummy_l,
                        &mut dummy_r,
                        64,
                    );
                }
            }
        }
    }
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
        let converter = TimeConverter::new(sample_rate as f32, bpm);
        let current_beat = converter.samples_to_beats(current_position);

        // Calculate pattern position and loop count
        let total_beats_elapsed = current_beat;
        let new_loop_count = (total_beats_elapsed / pattern.length) as u32;
        let pattern_position = total_beats_elapsed % pattern.length;

        // Detect pattern loop
        let pattern_looped = new_loop_count > processor.pattern_loop_count;
        if pattern_looped {
            processor.pattern_loop_count = new_loop_count;
            processor.notes_triggered_this_loop.clear();

            // Stop all active notes from previous loop
            processor.active_notes.clear();
        }

        // Process each note in the pattern
        for note in &pattern.notes {
            let note_start = note.start;
            let note_end = (note.start + note.duration).min(pattern.length);

            // Check if we should trigger note on
            let should_trigger = if pattern_looped && note_start == 0.0 {
                // Special case: note starts at beat 0 and pattern just looped
                true
            } else if processor.last_pattern_position <= pattern_position {
                // Normal progression within pattern
                pattern_position >= note_start && processor.last_pattern_position < note_start
            } else {
                // We've wrapped around - check if note starts before current position
                note_start <= pattern_position
                    && !processor.notes_triggered_this_loop.contains(&note.pitch)
            };

            if should_trigger {
                // Remove any existing instance of this note
                processor.active_notes.retain(|n| n.pitch != note.pitch);

                // Add new note
                processor.active_notes.push(ActiveMidiNote {
                    pitch: note.pitch,
                    velocity: note.velocity,
                    start_sample: current_position,
                });

                processor.notes_triggered_this_loop.push(note.pitch);
            }

            // Check if we should trigger note off
            let should_stop = if note_end >= pattern.length {
                // Note extends to or past pattern boundary
                pattern_looped
                    || (pattern_position < note_start
                        && processor.last_pattern_position >= note_start)
            } else if processor.last_pattern_position <= pattern_position {
                // Normal progression
                pattern_position >= note_end && processor.last_pattern_position < note_end
            } else {
                // Wrapped around
                note_end <= pattern_position
            };

            if should_stop {
                processor.active_notes.retain(|n| n.pitch != note.pitch);
            }
        }

        processor.last_pattern_position = pattern_position;
    }

    // Generate audio for active notes if no plugins
    if processor.plugins.is_empty() {
        for i in 0..num_frames {
            let mut sample = 0.0;

            for note in &processor.active_notes {
                let sample_offset = current_position + i as f64 - note.start_sample;
                sample +=
                    generate_sine_for_note(note.pitch, note.velocity, sample_offset, sample_rate);
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
    // Ensure buffers are zeroed
    processor.input_buffer_l[..num_frames].fill(0.0);
    processor.input_buffer_r[..num_frames].fill(0.0);

    let converter = TimeConverter::new(sample_rate as f32, bpm);

    for clip in &track.audio_clips {
        let clip_start_samples = converter.beats_to_samples(clip.start_beat);
        let clip_duration_samples = clip.samples.len() as f64;
        let clip_end_samples = clip_start_samples + clip_duration_samples;

        // Calculate overlap with current buffer
        let buffer_start = current_position;
        let buffer_end = current_position + num_frames as f64;

        if clip_end_samples > buffer_start && clip_start_samples < buffer_end {
            let start_in_buffer =
                ((clip_start_samples - buffer_start).max(0.0) as usize).min(num_frames);
            let end_in_buffer =
                ((clip_end_samples - buffer_start).min(num_frames as f64) as usize).max(0);
            let start_in_clip = ((buffer_start - clip_start_samples).max(0.0) as usize);

            for i in start_in_buffer..end_in_buffer {
                let clip_idx = start_in_clip + (i - start_in_buffer);
                if clip_idx < clip.samples.len() {
                    processor.input_buffer_l[i] += clip.samples[clip_idx];
                    processor.input_buffer_r[i] += clip.samples[clip_idx];
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
        if sample_pos > 0.0 && sample_pos < sample_rate * PREVIEW_NOTE_DURATION {
            let sample = generate_sine_for_note(
                preview.pitch,
                100, // Default preview velocity
                sample_pos,
                sample_rate,
            );
            let envelope = (-(sample_pos * 4.0 / sample_rate)).exp() as f32;

            processor.input_buffer_l[i] += sample * envelope * 3.0; // Boost for preview
            processor.input_buffer_r[i] += sample * envelope * 3.0;
        }
    }
}

fn build_block_midi_events(
    pattern: &crate::audio_state::PatternSnapshot,
    processor: &TrackProcessor,
    block_start_samples: f64,
    frames: usize,
    sample_rate: f64,
    bpm: f32,
) -> Vec<(u8, u8, u8, i64)> {
    let converter = TimeConverter::new(sample_rate as f32, bpm);
    let block_start_beat = converter.samples_to_beats(block_start_samples);
    let block_end_beat = converter.samples_to_beats(block_start_samples + frames as f64);

    let mut events = Vec::new();

    // Calculate pattern positions for this block
    let pattern_start = block_start_beat % pattern.length;
    let pattern_end = block_end_beat % pattern.length;

    for note in &pattern.notes {
        let note_start = note.start;
        let note_end = (note.start + note.duration).min(pattern.length);

        // Check if note on occurs in this block
        if pattern_end >= pattern_start {
            // No wrap in this block
            if note_start >= pattern_start && note_start < pattern_end {
                let beat_offset = note_start - pattern_start;
                let sample_offset = converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x90, note.pitch, note.velocity, frame));
                }
            }

            if note_end > pattern_start && note_end <= pattern_end {
                let beat_offset = note_end - pattern_start;
                let sample_offset = converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x80, note.pitch, 0, frame));
                }
            }
        } else {
            // Pattern wraps in this block
            // Check events after wrap point (beginning of pattern)
            if note_start < pattern_end {
                let samples_until_wrap = converter.beats_to_samples(pattern.length - pattern_start);
                let beat_offset = note_start;
                let sample_offset = samples_until_wrap + converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x90, note.pitch, note.velocity, frame));
                }
            }

            // Check events before wrap point
            if note_start >= pattern_start {
                let beat_offset = note_start - pattern_start;
                let sample_offset = converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x90, note.pitch, note.velocity, frame));
                }
            }

            // Similar logic for note offs
            if note_end <= pattern_end {
                let samples_until_wrap = converter.beats_to_samples(pattern.length - pattern_start);
                let beat_offset = note_end;
                let sample_offset = samples_until_wrap + converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x80, note.pitch, 0, frame));
                }
            }

            if note_end > pattern_start && note_end <= pattern.length {
                let beat_offset = note_end - pattern_start;
                let sample_offset = converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x80, note.pitch, 0, frame));
                }
            }
        }
    }

    // Sort events by frame time
    events.sort_by_key(|e| e.3);
    events
}

fn process_track_plugins(
    track: &crate::audio_state::TrackSnapshot,
    processor: &mut TrackProcessor,
    num_frames: usize,
    block_start_samples: f64,
    bpm: f32,
    sample_rate: f64,
) {
    // Prep. MIDI for this block (Note On/Off with proper frame offsets)
    let midi_events: Vec<(u8, u8, u8, i64)> = if track.is_midi {
        if let Some(pattern) = track.patterns.first() {
            build_block_midi_events(
                pattern,
                processor,
                block_start_samples,
                num_frames,
                sample_rate,
                bpm,
            )
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Clear output buffers for plugin chain
    processor.output_buffer_l[..num_frames].fill(0.0);
    processor.output_buffer_r[..num_frames].fill(0.0);

    for (plugin_idx, plugin_desc) in track.plugin_chain.iter().enumerate() {
        if plugin_desc.bypass {
            continue;
        }

        if let Some(plugin) = processor.plugins.get_mut(plugin_idx) {
            if let Some(instance) = &mut plugin.instance {
                // First plugin: feed MIDI (or clear)
                if plugin_idx == 0 && track.is_midi {
                    if !midi_events.is_empty() {
                        instance.prepare_midi_raw_events(&midi_events);
                        if DEBUG_PLUGIN_AUDIO {
                            debug_print_midi_events(&plugin_desc.uri, &midi_events);
                        }
                    } else {
                        instance.clear_midi_events();
                        if DEBUG_PLUGIN_AUDIO {
                            println!("[LV2][{}] MIDI: cleared (no events)", plugin_desc.uri);
                        }
                    }
                } else {
                    // Ensure downstream plugins don’t get stale MIDI. Also doesn't fix the mid stop problem
                    instance.clear_midi_events();
                }

                // Zero plugin outputs before run
                processor.output_buffer_l[..num_frames].fill(0.0);
                processor.output_buffer_r[..num_frames].fill(0.0);

                // Run plugin
                match instance.process(
                    &processor.input_buffer_l[..num_frames],
                    &processor.input_buffer_r[..num_frames],
                    &mut processor.output_buffer_l[..num_frames],
                    &mut processor.output_buffer_r[..num_frames],
                    num_frames,
                ) {
                    Ok(_) => {
                        if DEBUG_PLUGIN_AUDIO {
                            debug_print_output_peak(
                                &plugin_desc.uri,
                                &processor.output_buffer_l[..num_frames],
                                &processor.output_buffer_r[..num_frames],
                            );
                        }
                        // Pass plugin output to next stage (copy output->input)
                        processor.input_buffer_l[..num_frames]
                            .copy_from_slice(&processor.output_buffer_l[..num_frames]);
                        processor.input_buffer_r[..num_frames]
                            .copy_from_slice(&processor.output_buffer_r[..num_frames]);
                    }
                    Err(e) => {
                        eprintln!("[LV2][{}] run error: {}", plugin_desc.uri, e);
                        // Keep input buffers unchanged so the chain continues
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
