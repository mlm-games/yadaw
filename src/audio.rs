use crate::state::{AppState, AudioClip, AudioCommand, PluginInstance as PluginDesc, UIUpdate};
use crate::state::{MidiNote, Pattern};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use lazy_static::lazy_static;
use lilv::{World, instance::ActiveInstance, plugin::Plugin};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

lazy_static! {
    static ref LV2_WORLD: World = {
        let world = World::new();
        world.load_all();
        world
    };
}

#[derive(Debug, Clone)]
struct ActiveMidiNote {
    pitch: u8,
    velocity: u8,
    start_sample: f64,
}
struct RecordingState {
    is_recording: AtomicBool,
    recording_track: Option<usize>,
    recorded_samples: Vec<f32>,
    recording_start_position: f64,
}
struct PluginState {
    plugin_def: Plugin,
    instance: ActiveInstance,
    // Store port indices for efficiency
    audio_in_ports: Vec<usize>,
    audio_out_ports: Vec<usize>,
}

struct TrackAudioState {
    plugins: Vec<PluginState>,
    // Buffers for plugin I/O
    input_buffer_l: Vec<f32>,
    input_buffer_r: Vec<f32>,
    output_buffer_l: Vec<f32>,
    output_buffer_r: Vec<f32>,

    active_notes: Vec<ActiveMidiNote>,
    last_pattern_position: f64,
}

pub fn run_audio_thread(
    state: Arc<Mutex<AppState>>,
    commands: Receiver<AudioCommand>,
    updates: Sender<UIUpdate>,
) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("No output device");
    let config = device.default_output_config().expect("No default config");
    let sample_rate = config.sample_rate().0 as f64;
    let channels = config.channels() as usize;
    let max_buffer_size = 2048;

    let mut audio_state_local: Vec<TrackAudioState> = Vec::new();

    {
        let mut state_lock = state.lock().unwrap();
        state_lock.sample_rate = sample_rate as f32;
        for _ in 0..state_lock.tracks.len() {
            audio_state_local.push(TrackAudioState {
                plugins: Vec::new(),
                input_buffer_l: vec![0.0; max_buffer_size],
                input_buffer_r: vec![0.0; max_buffer_size],
                output_buffer_l: vec![0.0; max_buffer_size],
                output_buffer_r: vec![0.0; max_buffer_size],
                active_notes: Vec::new(),
                last_pattern_position: 0.0,
            });
        }
    }

    let recording_state = Arc::new(Mutex::new(RecordingState {
        is_recording: AtomicBool::new(false),
        recording_track: None,
        recorded_samples: Vec::new(),
        recording_start_position: 0.0,
    }));

    let recording_state_clone = recording_state.clone();
    let state_clone = state.clone();
    let updates_clone = updates.clone();

    // Start recording input thread
    std::thread::spawn(move || {
        let host = cpal::default_host(); // Create a new host for this thread
        match host.default_input_device() {
            Some(input_device) => {
                let input_config = match input_device.default_input_config() {
                    Ok(config) => config,
                    Err(e) => {
                        eprintln!("Failed to get input config: {}", e);
                        return;
                    }
                };

                let channels = input_config.channels() as usize;

                let recording_callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut rec_state = recording_state_clone.lock().unwrap();
                    if rec_state.is_recording.load(Ordering::Relaxed) {
                        // Record mono by averaging stereo channels
                        for frame in data.chunks(channels) {
                            let mono_sample = frame.iter().sum::<f32>() / channels as f32;
                            rec_state.recorded_samples.push(mono_sample);

                            // Send level update
                            let level = mono_sample.abs();
                            let _ = updates_clone.try_send(UIUpdate::RecordingLevel(level));
                        }
                    }
                };

                match input_device.build_input_stream(
                    &input_config.config(),
                    recording_callback,
                    |err| eprintln!("Input stream error: {}", err),
                    None,
                ) {
                    Ok(input_stream) => {
                        if let Err(e) = input_stream.play() {
                            eprintln!("Failed to play input stream: {}", e);
                            return;
                        }
                        std::thread::park();
                    }
                    Err(e) => {
                        eprintln!("Failed to build input stream: {}", e);
                    }
                }
            }
            None => {
                eprintln!("No input device available");
            }
        }
    });

    let mut audio_callback = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let num_frames = data.len() / channels;

        // 1. Process Commands from UI
        while let Ok(cmd) = commands.try_recv() {
            let mut state_lock = state.lock().unwrap();
            match cmd {
                AudioCommand::Play => state_lock.playing = true,
                AudioCommand::Stop => {
                    state_lock.playing = false;
                    state_lock.current_position = 0.0;
                }
                AudioCommand::SetTrackVolume(id, vol) => {
                    if let Some(track) = state_lock.tracks.get_mut(id) {
                        track.volume = vol;
                    }
                }
                AudioCommand::SetTrackPan(id, pan) => {
                    if let Some(track) = state_lock.tracks.get_mut(id) {
                        track.pan = pan;
                    }
                }
                AudioCommand::MuteTrack(id, mute) => {
                    if let Some(track) = state_lock.tracks.get_mut(id) {
                        track.muted = mute;
                    }
                }
                AudioCommand::AddPlugin(track_id, uri) => {
                    if let Some(track) = state_lock.tracks.get_mut(track_id) {
                        let plugin = LV2_WORLD
                            .plugins()
                            .iter()
                            .find(|p| p.uri().as_uri().unwrap_or("") == uri);
                        if let Some(plugin) = plugin {
                            if let Some(mut instance) =
                                unsafe { plugin.instantiate(sample_rate, &[]) }
                            {
                                let plugin_name =
                                    plugin.name().as_str().unwrap_or(&uri).to_string();

                                // Find audio ports
                                let mut audio_in_ports = Vec::new();
                                let mut audio_out_ports = Vec::new();

                                for port in plugin.iter_ports() {
                                    let index = port.index() as usize;
                                    if port.is_a(
                                        &LV2_WORLD
                                            .new_uri("http://lv2plug.in/ns/lv2core#AudioPort"),
                                    ) {
                                        if port.is_a(
                                            &LV2_WORLD
                                                .new_uri("http://lv2plug.in/ns/lv2core#InputPort"),
                                        ) {
                                            audio_in_ports.push(index);
                                        } else if port.is_a(
                                            &LV2_WORLD
                                                .new_uri("http://lv2plug.in/ns/lv2core#OutputPort"),
                                        ) {
                                            audio_out_ports.push(index);
                                        }
                                    }
                                }

                                // Connect all ports to buffers
                                let track_audio = &mut audio_state_local[track_id];

                                // Connect audio input ports
                                unsafe {
                                    if audio_in_ports.len() > 0 {
                                        instance.connect_port(
                                            audio_in_ports[0],
                                            track_audio.input_buffer_l.as_ptr(),
                                        );
                                    }
                                    if audio_in_ports.len() > 1 {
                                        instance.connect_port(
                                            audio_in_ports[1],
                                            track_audio.input_buffer_r.as_ptr(),
                                        );
                                    }

                                    // Connect audio output ports
                                    if audio_out_ports.len() > 0 {
                                        instance.connect_port_mut(
                                            audio_out_ports[0],
                                            track_audio.output_buffer_l.as_mut_ptr(),
                                        );
                                    }
                                    if audio_out_ports.len() > 1 {
                                        instance.connect_port_mut(
                                            audio_out_ports[1],
                                            track_audio.output_buffer_r.as_mut_ptr(),
                                        );
                                    }

                                    // Activate the instance
                                    let active_instance = instance.activate();

                                    track_audio.plugins.push(PluginState {
                                        plugin_def: plugin.clone(),
                                        instance: active_instance,
                                        audio_in_ports,
                                        audio_out_ports,
                                    });
                                }

                                track.plugin_chain.push(PluginDesc {
                                    uri: uri.clone(),
                                    name: plugin_name,
                                    bypass: false,
                                    params: HashMap::new(),
                                });
                            } else {
                                eprintln!("Failed to instantiate plugin: {}", uri);
                            }
                        }
                    }
                }
                AudioCommand::RemovePlugin(track_id, plugin_idx) => {
                    if let Some(track) = state_lock.tracks.get_mut(track_id) {
                        if plugin_idx < track.plugin_chain.len() {
                            track.plugin_chain.remove(plugin_idx);
                            // Remove the plugin state
                            if let Some(track_audio) = audio_state_local.get_mut(track_id) {
                                if plugin_idx < track_audio.plugins.len() {
                                    // The ActiveInstance will be deactivated on drop
                                    track_audio.plugins.remove(plugin_idx);
                                }
                            }
                        }
                    }
                }
                AudioCommand::AddNote(track_id, pattern_id, note) => {
                    if let Some(track) = state_lock.tracks.get_mut(track_id) {
                        if let Some(pattern) = track.patterns.get_mut(pattern_id) {
                            pattern.notes.push(note);
                            // Sort notes by start time
                            pattern
                                .notes
                                .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                        }
                    }
                }
                AudioCommand::RemoveNote(track_id, pattern_id, note_index) => {
                    if let Some(track) = state_lock.tracks.get_mut(track_id) {
                        if let Some(pattern) = track.patterns.get_mut(pattern_id) {
                            if note_index < pattern.notes.len() {
                                pattern.notes.remove(note_index);
                            }
                        }
                    }
                }
                AudioCommand::UpdateNote(track_id, pattern_id, note_index, new_note) => {
                    if let Some(track) = state_lock.tracks.get_mut(track_id) {
                        if let Some(pattern) = track.patterns.get_mut(pattern_id) {
                            if let Some(note) = pattern.notes.get_mut(note_index) {
                                *note = new_note;
                            }
                            // Re-sort notes
                            pattern
                                .notes
                                .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                        }
                    }
                }
                AudioCommand::StartRecording(track_id) => {
                    let mut rec_state = recording_state.lock().unwrap();
                    rec_state.is_recording.store(true, Ordering::Relaxed);
                    rec_state.recording_track = Some(track_id);
                    rec_state.recorded_samples.clear();
                    rec_state.recording_start_position = state_lock.current_position;
                    state_lock.recording = true;
                }
                AudioCommand::StopRecording => {
                    let mut rec_state = recording_state.lock().unwrap();
                    rec_state.is_recording.store(false, Ordering::Relaxed);
                    state_lock.recording = false;

                    // Create audio clip from recorded samples
                    if let Some(track_id) = rec_state.recording_track {
                        if !rec_state.recorded_samples.is_empty() {
                            let start_beat =
                                state_lock.position_to_beats(rec_state.recording_start_position);
                            let end_beat =
                                state_lock.position_to_beats(state_lock.current_position);

                            let clip = AudioClip {
                                name: format!(
                                    "Recording {}",
                                    chrono::Local::now().format("%H:%M:%S")
                                ),
                                start_beat,
                                length_beats: end_beat - start_beat,
                                samples: rec_state.recorded_samples.clone(),
                                sample_rate: state_lock.sample_rate,
                            };

                            let _ = updates.send(UIUpdate::RecordingFinished(track_id, clip));
                        }
                    }
                }
                _ => (),
            }
        }

        let mut state_lock = state.lock().unwrap();

        // 2. Process Audio
        data.fill(0.0);

        if !state_lock.playing {
            return;
        }

        for (track_idx, track) in state_lock.tracks.iter().enumerate() {
            if track.muted {
                continue;
            }

            let track_audio = &mut audio_state_local[track_idx];

            if track.is_midi {
                // Clear audio buffers for MIDI tracks
                track_audio.input_buffer_l[..num_frames].fill(0.0);
                track_audio.input_buffer_r[..num_frames].fill(0.0);

                // Process MIDI patterns
                if let Some(pattern) = track.patterns.first() {
                    let current_beat = state_lock.position_to_beats(state_lock.current_position);
                    let pattern_position = current_beat % pattern.length;

                    // Check for new notes to trigger
                    for note in &pattern.notes {
                        let note_start = note.start;
                        let note_end = note.start + note.duration;

                        // Check if note should start
                        if pattern_position >= note_start
                            && track_audio.last_pattern_position < note_start
                        {
                            // Trigger note on
                            track_audio.active_notes.push(ActiveMidiNote {
                                pitch: note.pitch,
                                velocity: note.velocity,
                                start_sample: state_lock.current_position,
                            });
                        }

                        // Check if note should end
                        if pattern_position >= note_end
                            && track_audio.last_pattern_position < note_end
                        {
                            // Remove note
                            track_audio.active_notes.retain(|n| n.pitch != note.pitch);
                        }
                    }

                    // Handle pattern loop
                    if pattern_position < track_audio.last_pattern_position {
                        // Pattern looped, clear all notes and retrigger
                        track_audio.active_notes.clear();
                        for note in &pattern.notes {
                            if note.start < 0.1 {
                                // Notes at the very beginning
                                track_audio.active_notes.push(ActiveMidiNote {
                                    pitch: note.pitch,
                                    velocity: note.velocity,
                                    start_sample: state_lock.current_position,
                                });
                            }
                        }
                    }

                    track_audio.last_pattern_position = pattern_position;
                }

                // Generate audio for active MIDI notes
                for i in 0..num_frames {
                    let mut sample = 0.0;
                    for note in &track_audio.active_notes {
                        let freq = 440.0 * 2.0_f32.powf((note.pitch as f32 - 69.0) / 12.0);
                        let phase = ((state_lock.current_position + i as f64 - note.start_sample)
                            * freq as f64
                            / state_lock.sample_rate as f64)
                            % 1.0;
                        sample += (phase * 2.0 * std::f64::consts::PI).sin() as f32
                            * (note.velocity as f32 / 127.0)
                            * 0.1;
                    }
                    track_audio.input_buffer_l[i] = sample;
                    track_audio.input_buffer_r[i] = sample;
                }
            } else {
                if track_audio.plugins.is_empty() {
                    for i in 0..num_frames {
                        track_audio.input_buffer_l[i] = 0.0; // Silence for now
                        track_audio.input_buffer_r[i] = 0.0;
                    }
                } else {
                    // Clear buffers, plugins will generate/process audio
                    track_audio.input_buffer_l[..num_frames].fill(0.0);
                    track_audio.input_buffer_r[..num_frames].fill(0.0);
                }
            }

            // Process through plugin chain
            for plugin_state in &mut track_audio.plugins {
                // Clear output buffers
                track_audio.output_buffer_l[..num_frames].fill(0.0);
                track_audio.output_buffer_r[..num_frames].fill(0.0);

                // Run the plugin
                unsafe {
                    plugin_state.instance.run(num_frames);
                }

                // Copy output to input for next plugin in chain
                track_audio.input_buffer_l[..num_frames]
                    .copy_from_slice(&track_audio.output_buffer_l[..num_frames]);
                track_audio.input_buffer_r[..num_frames]
                    .copy_from_slice(&track_audio.output_buffer_r[..num_frames]);
            }

            // Mix track output to master
            for i in 0..num_frames {
                let left_sample = if track_audio.plugins.is_empty() {
                    track_audio.input_buffer_l[i]
                } else {
                    track_audio.output_buffer_l[i]
                };
                let right_sample = if track_audio.plugins.is_empty() {
                    track_audio.input_buffer_r[i]
                } else {
                    track_audio.output_buffer_r[i]
                };

                let (left_gain, right_gain) = stereo_pan(track.volume, track.pan);
                data[i * channels] += left_sample * left_gain;
                if channels > 1 {
                    data[i * channels + 1] += right_sample * right_gain;
                }
            }
        }

        // 3. Finalize
        for sample in data.iter_mut() {
            *sample = (*sample * state_lock.master_volume).clamp(-1.0, 1.0);
        }

        state_lock.current_position += num_frames as f64;
        let _ = updates.try_send(UIUpdate::Position(state_lock.current_position));
    };

    let stream = device
        .build_output_stream(
            &config.into(),
            audio_callback,
            |err| eprintln!("an error occurred on stream: {}", err),
            None,
        )
        .unwrap();
    stream.play().unwrap();

    std::thread::park();
}

fn stereo_pan(volume: f32, pan: f32) -> (f32, f32) {
    let angle = pan * std::f32::consts::FRAC_PI_4;
    (volume * angle.cos(), volume * angle.sin())
}
