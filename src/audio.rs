use crate::state::{AppState, AudioClip, AudioCommand, PluginDescriptor, PluginParam, UIUpdate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use lazy_static::lazy_static;
use lilv::{World, instance::ActiveInstance, plugin::Plugin};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy)]
struct SendableF32Ptr(*mut f32);
unsafe impl Send for SendableF32Ptr {}

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
    audio_in_ports: Vec<usize>,
    audio_out_ports: Vec<usize>,
    control_ports: HashMap<usize, SendableF32Ptr>,
}

struct TrackAudioState {
    plugins: Vec<PluginState>,
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

    let mut preview_note: Option<(usize, u8, f64)> = None;

    let recording_state_clone = recording_state.clone();
    let updates_clone = updates.clone();

    // Start recording input thread
    std::thread::spawn(move || {
        let host = cpal::default_host();
        if let Some(input_device) = host.default_input_device() {
            if let Ok(input_config) = input_device.default_input_config() {
                let channels = input_config.channels() as usize;
                let recording_callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut rec_state = recording_state_clone.lock().unwrap();
                    if rec_state.is_recording.load(Ordering::Relaxed) {
                        for frame in data.chunks(channels) {
                            let mono_sample = frame.iter().sum::<f32>() / channels as f32;
                            rec_state.recorded_samples.push(mono_sample);
                            let level = mono_sample.abs();
                            let _ = updates_clone.try_send(UIUpdate::RecordingLevel(level));
                        }
                    }
                };
                if let Ok(input_stream) = input_device.build_input_stream(
                    &input_config.config(),
                    recording_callback,
                    |err| eprintln!("Input stream error: {}", err),
                    None,
                ) {
                    if input_stream.play().is_ok() {
                        std::thread::park();
                    }
                }
            }
        }
    });

    let mut audio_callback = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let num_frames = data.len() / channels;

        // 1. Process Commands from UI.
        // Lock is acquired and released for each command individually.
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
                AudioCommand::SoloTrack(id, solo) => {
                    if let Some(track) = state_lock.tracks.get_mut(id) {
                        track.solo = solo;
                    }
                }
                AudioCommand::PreviewNote(track_id, pitch) => {
                    preview_note = Some((track_id, pitch, state_lock.current_position));
                }
                AudioCommand::StopPreviewNote => {
                    preview_note = None;
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
                                let mut audio_in_ports = Vec::new();
                                let mut audio_out_ports = Vec::new();
                                let mut params = HashMap::new();
                                let mut control_values = HashMap::new();

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
                                    } else if port.is_a(
                                        &LV2_WORLD
                                            .new_uri("http://lv2plug.in/ns/lv2core#ControlPort"),
                                    ) && port.is_a(
                                        &LV2_WORLD
                                            .new_uri("http://lv2plug.in/ns/lv2core#InputPort"),
                                    ) {
                                        let name = port
                                            .name()
                                            .expect("Port name")
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string();
                                        let default = 0.5f32;
                                        let min = 0.0f32;
                                        let max = 1.0f32;
                                        params.insert(
                                            name.clone(),
                                            PluginParam {
                                                index,
                                                name: name.clone(),
                                                value: default,
                                                min,
                                                max,
                                                default,
                                            },
                                        );
                                        control_values.insert(index, default);
                                    }
                                }

                                if let Some(track_audio) = audio_state_local.get_mut(track_id) {
                                    let mut active_control_ports = HashMap::new();
                                    unsafe {
                                        for (i, &port_idx) in audio_in_ports.iter().enumerate() {
                                            let ptr = if i == 0 {
                                                track_audio.input_buffer_l.as_ptr()
                                            } else {
                                                track_audio.input_buffer_r.as_ptr()
                                            };
                                            instance.connect_port(
                                                (port_idx as u32).try_into().unwrap(),
                                                ptr,
                                            );
                                        }
                                        for (i, &port_idx) in audio_out_ports.iter().enumerate() {
                                            let ptr = if i == 0 {
                                                track_audio.output_buffer_l.as_mut_ptr()
                                            } else {
                                                track_audio.output_buffer_r.as_mut_ptr()
                                            };
                                            instance.connect_port_mut(
                                                (port_idx as u32).try_into().unwrap(),
                                                ptr,
                                            );
                                        }
                                        for (&port_idx, &value) in &control_values {
                                            let value_ptr = Box::into_raw(Box::new(value));
                                            instance.connect_port(
                                                (port_idx as u32).try_into().unwrap(),
                                                value_ptr,
                                            );
                                            active_control_ports
                                                .insert(port_idx, SendableF32Ptr(value_ptr));
                                        }
                                        let active_instance = instance.activate();
                                        track_audio.plugins.push(PluginState {
                                            plugin_def: plugin.clone(),
                                            instance: active_instance,
                                            audio_in_ports,
                                            audio_out_ports,
                                            control_ports: active_control_ports,
                                        });
                                    }
                                }
                                track.plugin_chain.push(PluginDescriptor {
                                    uri: uri.clone(),
                                    name: plugin_name,
                                    bypass: false,
                                    params,
                                });
                            }
                        }
                    }
                }
                AudioCommand::SetPluginBypass(track_id, plugin_idx, bypass) => {
                    if let Some(plugin) = state_lock
                        .tracks
                        .get_mut(track_id)
                        .and_then(|t| t.plugin_chain.get_mut(plugin_idx))
                    {
                        plugin.bypass = bypass;
                    }
                }
                AudioCommand::SetPluginParam(track_id, plugin_idx, param_name, value) => {
                    if let Some(param) = state_lock
                        .tracks
                        .get_mut(track_id)
                        .and_then(|t| t.plugin_chain.get_mut(plugin_idx))
                        .and_then(|p| p.params.get_mut(&param_name))
                    {
                        param.value = value;
                    }
                }
                AudioCommand::RemovePlugin(track_id, plugin_idx) => {
                    if let Some(track) = state_lock.tracks.get_mut(track_id) {
                        if plugin_idx < track.plugin_chain.len() {
                            track.plugin_chain.remove(plugin_idx);
                            if let Some(track_audio) = audio_state_local.get_mut(track_id) {
                                if plugin_idx < track_audio.plugins.len() {
                                    let removed = track_audio.plugins.remove(plugin_idx);
                                    for (_, ptr_wrapper) in removed.control_ports {
                                        unsafe {
                                            drop(Box::from_raw(ptr_wrapper.0));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                AudioCommand::AddNote(track_id, pattern_id, note) => {
                    if let Some(pattern) = state_lock
                        .tracks
                        .get_mut(track_id)
                        .and_then(|t| t.patterns.get_mut(pattern_id))
                    {
                        pattern.notes.push(note);
                        pattern
                            .notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                    }
                }
                AudioCommand::RemoveNote(track_id, pattern_id, note_index) => {
                    if let Some(pattern) = state_lock
                        .tracks
                        .get_mut(track_id)
                        .and_then(|t| t.patterns.get_mut(pattern_id))
                    {
                        if note_index < pattern.notes.len() {
                            pattern.notes.remove(note_index);
                        }
                    }
                }
                AudioCommand::UpdateNote(track_id, pattern_id, note_index, new_note) => {
                    if let Some(pattern) = state_lock
                        .tracks
                        .get_mut(track_id)
                        .and_then(|t| t.patterns.get_mut(pattern_id))
                    {
                        if let Some(note) = pattern.notes.get_mut(note_index) {
                            *note = new_note;
                        }
                        pattern
                            .notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
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
                    if let Some(track_id) = rec_state.recording_track {
                        if !rec_state.recorded_samples.is_empty() {
                            let start_beat =
                                state_lock.position_to_beats(rec_state.recording_start_position);
                            let end_beat =
                                state_lock.position_to_beats(state_lock.current_position);
                            let clip = AudioClip {
                                name: format!("Rec {}", chrono::Local::now().format("%T")),
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

        // 2. Process Audio.
        // Lock the state only to get the necessary values, then release it.
        let (is_playing, current_pos, master_vol, tracks, bpm, sample_r) = {
            let state_lock = state.lock().unwrap();
            (
                state_lock.playing,
                state_lock.current_position,
                state_lock.master_volume,
                state_lock.tracks.clone(),
                state_lock.bpm,
                state_lock.sample_rate,
            )
        };

        data.fill(0.0);

        if !is_playing {
            return;
        }

        if let Some((track_id, pitch, start_sample)) = preview_note {
            if track_id < audio_state_local.len() {
                let track_audio = &mut audio_state_local[track_id];
                for i in 0..num_frames {
                    let sample_pos = current_pos + i as f64 - start_sample;
                    if sample_pos > 0.0 {
                        let freq = 440.0 * 2.0_f32.powf((pitch as f32 - 69.0) / 12.0);
                        let phase = (sample_pos * freq as f64 / sample_rate) % 1.0;
                        let envelope = (-(sample_pos * 4.0)).exp() as f32;
                        let sample =
                            (phase * 2.0 * std::f64::consts::PI).sin() as f32 * envelope * 0.3;
                        track_audio.input_buffer_l[i] += sample;
                        track_audio.input_buffer_r[i] += sample;
                    }
                }
                if current_pos - start_sample > sample_rate * 0.5 {
                    preview_note = None;
                }
            }
        }

        let any_track_is_soloed = tracks.iter().any(|t| t.solo);

        for (track_idx, track) in tracks.iter().enumerate() {
            if track.muted || (any_track_is_soloed && !track.solo) {
                continue;
            }

            let track_audio = &mut audio_state_local[track_idx];

            if track.is_midi {
                track_audio.input_buffer_l[..num_frames].fill(0.0);
                track_audio.input_buffer_r[..num_frames].fill(0.0);

                if let Some(pattern) = track.patterns.first() {
                    let current_beat = (current_pos / sample_rate as f64) * (bpm / 60.0) as f64;
                    let pattern_position = current_beat % pattern.length;

                    for note in &pattern.notes {
                        let note_start = note.start;
                        let note_end = note.start + note.duration;
                        if pattern_position >= note_start
                            && track_audio.last_pattern_position < note_start
                        {
                            track_audio.active_notes.push(ActiveMidiNote {
                                pitch: note.pitch,
                                velocity: note.velocity,
                                start_sample: current_pos,
                            });
                        }
                        if pattern_position >= note_end
                            && track_audio.last_pattern_position < note_end
                        {
                            track_audio.active_notes.retain(|n| n.pitch != note.pitch);
                        }
                    }
                    if pattern_position < track_audio.last_pattern_position {
                        track_audio.active_notes.clear();
                        for note in &pattern.notes {
                            if note.start < 0.1 {
                                track_audio.active_notes.push(ActiveMidiNote {
                                    pitch: note.pitch,
                                    velocity: note.velocity,
                                    start_sample: current_pos,
                                });
                            }
                        }
                    }
                    track_audio.last_pattern_position = pattern_position;
                }

                for i in 0..num_frames {
                    let mut sample = 0.0;
                    for note in &track_audio.active_notes {
                        let freq = 440.0 * 2.0_f32.powf((note.pitch as f32 - 69.0) / 12.0);
                        let phase = ((current_pos + i as f64 - note.start_sample) * freq as f64
                            / sample_rate)
                            % 1.0;
                        sample += (phase * 2.0 * std::f64::consts::PI).sin() as f32
                            * (note.velocity as f32 / 127.0)
                            * 0.1;
                    }
                    track_audio.input_buffer_l[i] = sample;
                    track_audio.input_buffer_r[i] = sample;
                }
            } else {
                // Is Audio Track
                track_audio.input_buffer_l[..num_frames].fill(0.0);
                track_audio.input_buffer_r[..num_frames].fill(0.0);
                let buffer_start_abs = current_pos;
                let buffer_end_abs = buffer_start_abs + num_frames as f64;

                for clip in &track.audio_clips {
                    let clip_start_abs = (clip.start_beat * 60.0 / bpm as f64) * sample_rate as f64;
                    let clip_end_abs = clip_start_abs
                        + (clip.length_beats * 60.0 / bpm as f64) * sample_rate as f64;

                    let overlap_start = buffer_start_abs.max(clip_start_abs);
                    let overlap_end = buffer_end_abs.min(clip_end_abs);

                    if overlap_start < overlap_end {
                        let start_index = (overlap_start - buffer_start_abs) as usize;
                        let end_index = (overlap_end - buffer_start_abs) as usize;
                        let clip_start_offset = (overlap_start - clip_start_abs) as usize;

                        for i in 0..(end_index - start_index) {
                            if let Some(sample) = clip.samples.get(clip_start_offset + i) {
                                track_audio.input_buffer_l[start_index + i] += *sample;
                                track_audio.input_buffer_r[start_index + i] += *sample;
                            }
                        }
                    }
                }
            }

            for (plugin_idx, plugin_state) in track_audio.plugins.iter_mut().enumerate() {
                if let Some(plugin_desc) = track.plugin_chain.get(plugin_idx) {
                    if !plugin_desc.bypass {
                        track_audio.output_buffer_l[..num_frames].fill(0.0);
                        track_audio.output_buffer_r[..num_frames].fill(0.0);
                        for param in plugin_desc.params.values() {
                            if let Some(ptr_wrapper) = plugin_state.control_ports.get(&param.index)
                            {
                                unsafe {
                                    *ptr_wrapper.0 = param.value;
                                }
                            }
                        }
                        unsafe {
                            plugin_state
                                .instance
                                .run((num_frames as u32).try_into().unwrap());
                        }
                        track_audio.input_buffer_l[..num_frames]
                            .copy_from_slice(&track_audio.output_buffer_l[..num_frames]);
                        track_audio.input_buffer_r[..num_frames]
                            .copy_from_slice(&track_audio.output_buffer_r[..num_frames]);
                    }
                }
            }

            for i in 0..num_frames {
                let left_sample = track_audio.input_buffer_l[i];
                let right_sample = track_audio.input_buffer_r[i];
                let (left_gain, right_gain) = stereo_pan(track.volume, track.pan);
                data[i * channels] += left_sample * left_gain;
                if channels > 1 {
                    data[i * channels + 1] += right_sample * right_gain;
                }
            }
        }

        let mut track_levels = Vec::new();
        for track_idx in 0..tracks.len() {
            let track_audio = &audio_state_local[track_idx];
            let left_peak = track_audio.input_buffer_l[..num_frames]
                .iter()
                .map(|s| s.abs())
                .fold(0.0f32, f32::max);
            let right_peak = track_audio.input_buffer_r[..num_frames]
                .iter()
                .map(|s| s.abs())
                .fold(0.0f32, f32::max);
            track_levels.push((left_peak, right_peak));
        }
        let _ = updates.try_send(UIUpdate::TrackLevels(track_levels));

        let mut master_left_peak = 0.0f32;
        let mut master_right_peak = 0.0f32;
        for i in 0..num_frames {
            let left = data[i * channels];
            let right = if channels > 1 {
                data[i * channels + 1]
            } else {
                left
            };
            master_left_peak = master_left_peak.max(left.abs());
            master_right_peak = master_right_peak.max(right.abs());
        }
        let _ = updates.try_send(UIUpdate::MasterLevel(master_left_peak, master_right_peak));

        for sample in data.iter_mut() {
            *sample = (*sample * master_vol).clamp(-1.0, 1.0);
        }

        // 3. Finalize
        // Lock state briefly at the end to update the position.
        {
            let mut state_lock = state.lock().unwrap();
            state_lock.current_position += num_frames as f64;
            let _ = updates.try_send(UIUpdate::Position(state_lock.current_position));
        }
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
    let p = (pan.clamp(-1.0, 1.0) + 1.0) / 2.0;
    let angle = p * std::f32::consts::FRAC_PI_2;
    (volume * angle.cos(), volume * angle.sin())
}
