use crate::audio_state::{
    AudioState, MidiClipSnapshot, RealtimeCommand, RtAutomationLaneSnapshot, RtAutomationTarget,
    RtCurveType, TrackSnapshot,
};
use crate::audio_utils::{calculate_stereo_gains, soft_clip};
use crate::constants::{
    DEBUG_PLUGIN_AUDIO, MAX_BUFFER_SIZE, PREVIEW_NOTE_DURATION, RECORDING_BUFFER_SIZE,
};
use crate::lv2_plugin_host::LV2PluginInstance;
use crate::messages::UIUpdate;
use crate::midi_utils::{generate_sine_for_note, MidiNoteUtils};
use crate::mixer::{ChannelStrip, MixerEngine};
use crate::model::clip::AudioClip;
use crate::plugin_host;
use crate::time_utils::TimeConverter;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;
use parking_lot::RwLock;
use rtrb::{Consumer, RingBuffer};
use std::sync::atomic::Ordering;
use std::sync::Arc;

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

#[derive(Clone)]
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

    audio_state.sample_rate.store(sample_rate as f32);

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
                        let mut producer = recording_producer.lock();
                        for frame in data.chunks(channels) {
                            let mono = frame.iter().sum::<f32>() / channels as f32;
                            let _ = producer.push(mono);
                            let _ = updates_clone.try_send(UIUpdate::RecordingLevel(mono.abs()));
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
                    std::thread::park();
                }
            }
        }
    });

    // Audio callback
    let audio_callback = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let num_frames = data.len() / channels;

        while let Ok(cmd) = realtime_commands.try_recv() {
            engine.process_realtime_command(cmd);
        }

        // Recording gather
        if engine.audio_state.recording.load(Ordering::Relaxed) {
            if !engine.recording_state.is_recording {
                engine.recording_state.is_recording = true;
                engine.recording_state.recording_start_position = engine.audio_state.get_position();
                engine.recording_state.accumulated_samples.clear();
            }
            while let Ok(sample) = engine.recording_state.recording_consumer.pop() {
                engine.recording_state.accumulated_samples.push(sample);
            }
        } else if engine.recording_state.is_recording {
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
                        ..Default::default()
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

        if !engine.audio_state.playing.load(Ordering::Relaxed) {
            engine.midi_panic();
            for processor in &mut engine.track_processors {
                processor.last_pattern_position = 0.0;
                processor.pattern_loop_count = 0;
                processor.notes_triggered_this_loop.clear();
            }
            return;
        }

        // Process block(s)
        let current_position = engine.audio_state.get_position();
        let next_position = engine.process_audio(data, num_frames, channels, current_position);

        // Update once
        engine.audio_state.set_position(next_position);
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
            RealtimeCommand::SetLoopEnabled(enabled) => {
                self.audio_state
                    .loop_enabled
                    .store(enabled, Ordering::Relaxed);
            }
            RealtimeCommand::SetLoopRegion(start, end) => {
                self.audio_state.loop_start.store(start);
                self.audio_state.loop_end.store(end);
            }
        }
    }

    fn update_track_processors(&mut self, tracks: &[TrackSnapshot]) {
        while self.track_processors.len() < tracks.len() {
            self.track_processors.push(TrackProcessor {
                id: self.track_processors.len(),
                plugins: Vec::new(),
                input_buffer_l: vec![0.0; MAX_BUFFER_SIZE],
                input_buffer_r: vec![0.0; MAX_BUFFER_SIZE],
                output_buffer_l: vec![0.0; MAX_BUFFER_SIZE],
                output_buffer_r: vec![0.0; MAX_BUFFER_SIZE],
                active_notes: Vec::new(),
                last_pattern_position: 0.0,
                automated_volume: f32::NAN,
                automated_pan: f32::NAN,
                automated_plugin_params: DashMap::new(),
                pattern_loop_count: 0,
                notes_triggered_this_loop: Vec::new(),
            });
        }

        for (track_idx, track) in tracks.iter().enumerate() {
            if let Some(processor) = self.track_processors.get_mut(track_idx) {
                if processor.plugins.len() != track.plugin_chain.len() {
                    processor.plugins.clear();
                    for plugin_desc in &track.plugin_chain {
                        let mut instance = plugin_host::instantiate(&plugin_desc.uri).ok();
                        if let Some(inst) = instance.as_mut() {
                            for entry in plugin_desc.params.iter() {
                                inst.get_params()
                                    .insert(entry.key().clone(), *entry.value());
                            }
                            inst.set_params_arc(plugin_desc.params.clone());
                        }
                        processor.plugins.push(PluginProcessor {
                            instance,
                            uri: plugin_desc.uri.clone(),
                            bypass: plugin_desc.bypass,
                        });
                    }
                }
            }
        }

        while self.channel_strips.len() < tracks.len() {
            self.channel_strips.push(ChannelStrip::default());
        }

        for (idx, track) in tracks.iter().enumerate() {
            if let Some(strip) = self.channel_strips.get_mut(idx) {
                strip.gain = track.volume;
                strip.pan = track.pan;
                strip.mute = track.muted;
                strip.solo = track.solo;
            }
        }
    }

    fn process_audio(
        &mut self,
        output: &mut [f32],
        num_frames: usize,
        channels: usize,
        mut current_position: f64,
    ) -> f64 {
        let tracks = self.tracks.read().clone();
        let bpm = self.audio_state.bpm.load();
        let master_volume = self.audio_state.master_volume.load();
        let loop_enabled = self.audio_state.loop_enabled.load(Ordering::Relaxed);
        let loop_start = self.audio_state.loop_start.load();
        let loop_end = self.audio_state.loop_end.load();

        let converter = TimeConverter::new(self.sample_rate as f32, bpm);
        let mut frames_processed = 0usize;

        while frames_processed < num_frames {
            let block_start_pos = current_position;
            let block_start_beat = converter.samples_to_beats(block_start_pos);

            let frames_to_loop_end =
                if loop_enabled && loop_end > loop_start && block_start_beat < loop_end {
                    let samples_to_loop_end =
                        converter.beats_to_samples(loop_end - block_start_beat);
                    samples_to_loop_end as usize
                } else {
                    usize::MAX
                };

            let frames_to_process = frames_to_loop_end
                .min(num_frames - frames_processed)
                .min(MAX_BUFFER_SIZE);

            // Clear sub-block
            for i in frames_processed..(frames_processed + frames_to_process) {
                let out_idx = i * channels;
                output[out_idx] = 0.0;
                if channels > 1 {
                    output[out_idx + 1] = 0.0;
                }
            }

            let any_track_soloed = tracks.iter().any(|t| t.solo);
            let preview_opt = self.preview_note.clone();

            for (track_idx, track) in tracks.iter().enumerate() {
                if track.muted || (any_track_soloed && !track.solo) {
                    continue;
                }

                if let Some(processor) = self.track_processors.get_mut(track_idx) {
                    apply_automation(track, processor, block_start_beat);

                    if track.is_midi {
                        process_midi_track(
                            track,
                            processor,
                            frames_to_process,
                            block_start_pos,
                            bpm,
                            self.sample_rate,
                            loop_enabled,
                            loop_start,
                            loop_end,
                        );
                    } else {
                        process_audio_track(
                            track,
                            processor,
                            frames_to_process,
                            block_start_pos,
                            bpm,
                            self.sample_rate,
                        );
                    }

                    if let Some(ref preview) = preview_opt {
                        if preview.track_id == track_idx {
                            process_preview_note(
                                processor,
                                preview,
                                frames_to_process,
                                block_start_pos,
                                self.sample_rate,
                            );
                        }
                    }

                    if !processor.plugins.is_empty() {
                        process_track_plugins(
                            track,
                            processor,
                            frames_to_process,
                            block_start_pos,
                            bpm,
                            self.sample_rate,
                            loop_enabled,
                            loop_start,
                            loop_end,
                        );
                    }

                    let (left_gain, right_gain) = effective_gains(track, processor);
                    for i in 0..frames_to_process {
                        let out_idx = (frames_processed + i) * channels;
                        output[out_idx] += processor.input_buffer_l[i] * left_gain;
                        if channels > 1 {
                            output[out_idx + 1] += processor.input_buffer_r[i] * right_gain;
                        }
                    }
                }
            }

            // Apply master after summing
            for i in frames_processed..(frames_processed + frames_to_process) {
                let out_idx = i * channels;
                output[out_idx] = soft_clip(output[out_idx] * master_volume);
                if channels > 1 {
                    output[out_idx + 1] = soft_clip(output[out_idx + 1] * master_volume);
                }
            }

            current_position += frames_to_process as f64;
            frames_processed += frames_to_process;

            let new_beat = converter.samples_to_beats(current_position);
            if loop_enabled && loop_end > loop_start && new_beat >= loop_end {
                current_position = converter.beats_to_samples(loop_start);
                for processor in &mut self.track_processors {
                    processor.active_notes.clear();
                }
            }
        }

        current_position
    }

    fn midi_panic(&mut self) {
        for processor in self.track_processors.iter_mut() {
            if !processor.active_notes.is_empty() {
                let mut events: Vec<(u8, u8, u8, i64)> = Vec::new();
                for note in &processor.active_notes {
                    events.push((0x80, note.pitch, 0, 0));
                }
                for plugin in processor.plugins.iter_mut() {
                    if let Some(instance) = &mut plugin.instance {
                        instance.prepare_midi_raw_events(&events);
                        let mut dl = vec![0.0f32; 64];
                        let mut dr = vec![0.0f32; 64];
                        let _ = instance.process(&dl.clone(), &dr.clone(), &mut dl, &mut dr, 64);
                    }
                }
            }
            processor.active_notes.clear();
            processor.last_pattern_position = 0.0;
            processor.pattern_loop_count = 0;
            processor.notes_triggered_this_loop.clear();

            let panic_events: Vec<(u8, u8, u8, i64)> = (0..16)
                .flat_map(|ch| vec![(0xB0 | ch, 123, 0, 0), (0xB0 | ch, 120, 0, 0)])
                .collect();

            for plugin in processor.plugins.iter_mut() {
                if let Some(instance) = &mut plugin.instance {
                    instance.prepare_midi_raw_events(&panic_events);
                    let mut dl = vec![0.0f32; 64];
                    let mut dr = vec![0.0f32; 64];
                    let _ = instance.process(&dl.clone(), &dr.clone(), &mut dl, &mut dr, 64);
                }
            }
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
    let converter = TimeConverter::new(sample_rate as f32, bpm);
    let current_beat = converter.samples_to_beats(current_position);

    // Handle looping
    let effective_beat = if loop_enabled && loop_end > loop_start {
        let loop_length = loop_end - loop_start;
        if current_beat >= loop_end {
            loop_start + ((current_beat - loop_start) % loop_length)
        } else if current_beat < loop_start {
            current_beat
        } else {
            current_beat
        }
    } else {
        current_beat
    };

    // Process MIDI clips at the effective beat position
    for clip in &track.midi_clips {
        let clip_end = clip.start_beat + clip.length_beats;

        // Check if we're within this clip
        if effective_beat >= clip.start_beat && effective_beat < clip_end {
            let clip_position = effective_beat - clip.start_beat;

            // Process notes in the clip
            for note in &clip.notes {
                let note_start_abs = clip.start_beat + note.start;
                let note_end_abs = clip.start_beat + note.start + note.duration;

                // Check if note should be playing
                if effective_beat >= note_start_abs && effective_beat < note_end_abs {
                    // Check if this note needs to be triggered
                    let should_trigger =
                        !processor.active_notes.iter().any(|n| n.pitch == note.pitch);

                    if should_trigger {
                        processor.active_notes.push(ActiveMidiNote {
                            pitch: note.pitch,
                            velocity: note.velocity,
                            start_sample: current_position,
                        });
                    }
                }

                // Check if note should stop
                if processor.active_notes.iter().any(|n| n.pitch == note.pitch) {
                    if effective_beat >= note_end_abs || effective_beat < note_start_abs {
                        processor.active_notes.retain(|n| n.pitch != note.pitch);
                    }
                }
            }
        }
    }

    // Clear input buffers
    processor.input_buffer_l[..num_frames].fill(0.0);
    processor.input_buffer_r[..num_frames].fill(0.0);

    // Generate audio if no plugins
    if processor.plugins.is_empty() && !processor.active_notes.is_empty() {
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
    clip: &MidiClipSnapshot,
    block_start_samples: f64,
    frames: usize,
    sample_rate: f64,
    bpm: f32,
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,
) -> Vec<(u8, u8, u8, i64)> {
    let converter = TimeConverter::new(sample_rate as f32, bpm);
    let block_start_beat = converter.samples_to_beats(block_start_samples);
    let block_end_beat = converter.samples_to_beats(block_start_samples + frames as f64);

    let mut events = Vec::new();

    // Adjust for loop if enabled
    let effective_start_beat =
        if loop_enabled && loop_end > loop_start && block_start_beat >= loop_end {
            let loop_length = loop_end - loop_start;
            loop_start + ((block_start_beat - loop_start) % loop_length)
        } else {
            block_start_beat
        };

    let effective_end_beat = if loop_enabled && loop_end > loop_start && block_end_beat >= loop_end
    {
        let loop_length = loop_end - loop_start;
        loop_start + ((block_end_beat - loop_start) % loop_length)
    } else {
        block_end_beat
    };

    // Check if this clip is active during this block
    let clip_end = clip.start_beat + clip.length_beats;

    if effective_start_beat < clip_end && effective_end_beat > clip.start_beat {
        for note in &clip.notes {
            let note_start_abs = clip.start_beat + note.start;
            let note_end_abs = clip.start_beat + note.start + note.duration;

            // Check if note on occurs in this block
            if note_start_abs >= effective_start_beat && note_start_abs < effective_end_beat {
                let beat_offset = note_start_abs - block_start_beat;
                let sample_offset = converter.beats_to_samples(beat_offset);
                let frame = sample_offset.round() as i64;
                if frame >= 0 && frame < frames as i64 {
                    events.push((0x90, note.pitch, note.velocity, frame));
                }
            }

            // Check if note off occurs in this block
            if note_end_abs > effective_start_beat && note_end_abs <= effective_end_beat {
                let beat_offset = note_end_abs - block_start_beat;
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
    track: &TrackSnapshot,
    processor: &mut TrackProcessor,
    num_frames: usize,
    block_start_samples: f64,
    bpm: f32,
    sample_rate: f64,
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,
) {
    // Build MIDI events for this block if it's a MIDI track
    let mut all_midi_events = Vec::new();

    if track.is_midi {
        for clip in &track.midi_clips {
            let clip_events = build_block_midi_events(
                clip,
                block_start_samples,
                num_frames,
                sample_rate,
                bpm,
                loop_enabled,
                loop_start,
                loop_end,
            );
            all_midi_events.extend(clip_events);
        }
        all_midi_events.sort_by_key(|e| e.3);
    }

    // Clear plugin outputs
    processor.output_buffer_l[..num_frames].fill(0.0);
    processor.output_buffer_r[..num_frames].fill(0.0);

    // If no plugins, nothing to do
    if processor.plugins.is_empty() {
        return;
    }

    // Process chain
    let mut first_active_plugin = true;

    for (plugin_idx, plugin_desc) in track.plugin_chain.iter().enumerate() {
        if plugin_desc.bypass {
            continue;
        }

        if let Some(plugin) = processor.plugins.get_mut(plugin_idx) {
            if let Some(instance) = &mut plugin.instance {
                // Apply automated plugin param values (if any)
                for kv in processor.automated_plugin_params.iter() {
                    let ((p_idx, param_name), value) = (kv.key().clone(), *kv.value());
                    if p_idx == plugin_idx {
                        instance.set_parameter(&param_name, value);
                    }
                }

                // Feed MIDI to the first active plugin on MIDI tracks
                if track.is_midi {
                    if first_active_plugin && !all_midi_events.is_empty() {
                        instance.prepare_midi_raw_events(&all_midi_events);
                    } else {
                        instance.clear_midi_events();
                    }
                    first_active_plugin = false;
                } else {
                    instance.clear_midi_events();
                }

                // Run plugin
                if let Err(e) = instance.process(
                    &processor.input_buffer_l[..num_frames],
                    &processor.input_buffer_r[..num_frames],
                    &mut processor.output_buffer_l[..num_frames],
                    &mut processor.output_buffer_r[..num_frames],
                    num_frames,
                ) {
                    eprintln!("Plugin processing error: {}", e);
                } else {
                    // Copy output to input for next plugin
                    processor.input_buffer_l[..num_frames]
                        .copy_from_slice(&processor.output_buffer_l[..num_frames]);
                    processor.input_buffer_r[..num_frames]
                        .copy_from_slice(&processor.output_buffer_r[..num_frames]);
                }
            }
        }
    }
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
                plugin_idx,
                param_name,
            } => {
                processor
                    .automated_plugin_params
                    .insert((*plugin_idx, param_name.clone()), value);
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
