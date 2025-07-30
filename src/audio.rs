use crate::state::{AppState, AudioCommand, UIUpdate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use std::sync::{Arc, Mutex};

pub fn run_audio_thread(
    state: Arc<Mutex<AppState>>,
    commands: Receiver<AudioCommand>,
    updates: Sender<UIUpdate>,
) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("No output device");
    let config = device.default_output_config().expect("No default config");

    println!("Audio device: {}", device.name().unwrap_or_default());
    println!("Sample rate: {}", config.sample_rate().0);

    // Update state with actual sample rate
    {
        let mut state = state.lock().unwrap();
        state.sample_rate = config.sample_rate().0 as f32;
    }

    let err_fn = |err| eprintln!("Audio stream error: {}", err);

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.config(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                audio_callback(data, &state, &commands, &updates);
            },
            err_fn,
            None,
        ),
        _ => panic!("Unsupported sample format"),
    }
    .expect("Failed to build output stream");

    stream.play().expect("Failed to play stream");

    // Keep thread alive
    std::thread::park();
}

fn audio_callback(
    output: &mut [f32],
    state: &Arc<Mutex<AppState>>,
    commands: &Receiver<AudioCommand>,
    updates: &Sender<UIUpdate>,
) {
    // Process commands
    while let Ok(cmd) = commands.try_recv() {
        let mut state = state.lock().unwrap();
        match cmd {
            AudioCommand::Play => state.playing = true,
            AudioCommand::Stop => {
                state.playing = false;
                state.current_position = 0.0;
            }
            AudioCommand::SetTrackVolume(track_id, volume) => {
                if let Some(track) = state.tracks.get_mut(track_id) {
                    track.volume = volume;
                }
            }
            AudioCommand::SetTrackPan(track_id, pan) => {
                if let Some(track) = state.tracks.get_mut(track_id) {
                    track.pan = pan;
                }
            }
            AudioCommand::MuteTrack(track_id, muted) => {
                if let Some(track) = state.tracks.get_mut(track_id) {
                    track.muted = muted;
                }
            }
            _ => {} // Handle other commands later
        }
    }

    let mut state = state.lock().unwrap();
    let channels = 2; // Stereo
    let frames = output.len() / channels;

    // Clear output buffer
    output.fill(0.0);

    if state.playing {
        // Simple sine wave for testing
        let frequency = 440.0; // A4
        let sample_rate = state.sample_rate;

        for frame in 0..frames {
            let time = (state.current_position + frame as f64) / sample_rate as f64;
            let sample = (2.0 * std::f32::consts::PI * frequency as f32 * time as f32).sin();

            // Apply track processing
            let mut mixed_left = 0.0;
            let mut mixed_right = 0.0;

            for track in &state.tracks {
                if track.muted {
                    continue;
                }

                // Simple panning
                let left_gain = track.volume * (1.0 - track.pan.max(0.0));
                let right_gain = track.volume * (1.0 + track.pan.min(0.0));

                mixed_left += sample * left_gain * 0.1; // Reduce volume for safety
                mixed_right += sample * right_gain * 0.1;
            }

            mixed_left *= state.master_volume;
            mixed_right *= state.master_volume;

            // Write to output, clamping to prevent clipping
            output[frame * channels] = mixed_left.clamp(-1.0, 1.0);
            output[frame * channels + 1] = mixed_right.clamp(-1.0, 1.0);
        }

        // Update position
        state.current_position += frames as f64;

        // Send position update to UI, but don't block
        let _ = updates.try_send(UIUpdate::Position(state.current_position));
    } else {
        // If not playing, ensure the buffer is silent
        output.fill(0.0);
    }
}
