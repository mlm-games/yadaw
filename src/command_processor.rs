use crate::audio_state::{AudioState, RealtimeCommand};
use crate::messages::{AudioCommand, UIUpdate};
use crate::plugin;
use crate::project::AppState;
use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub fn run_command_processor(
    app_state: Arc<Mutex<AppState>>,
    audio_state: Arc<AudioState>,
    command_rx: Receiver<AudioCommand>,
    realtime_tx: Sender<RealtimeCommand>,
    ui_tx: Sender<UIUpdate>,
) {
    while let Ok(command) = command_rx.recv() {
        process_command(&command, &app_state, &audio_state, &realtime_tx, &ui_tx);
    }
}

fn process_command(
    command: &AudioCommand,
    app_state: &Arc<Mutex<AppState>>,
    audio_state: &Arc<AudioState>,
    realtime_tx: &Sender<RealtimeCommand>,
    ui_tx: &Sender<UIUpdate>,
) {
    match command {
        AudioCommand::Play => {
            audio_state.playing.store(true, Ordering::Relaxed);
        }

        AudioCommand::Stop => {
            audio_state.playing.store(false, Ordering::Relaxed);
            audio_state.recording.store(false, Ordering::Relaxed);
        }

        AudioCommand::Pause => {
            audio_state.playing.store(false, Ordering::Relaxed);
        }

        AudioCommand::Record => {
            audio_state.recording.store(true, Ordering::Relaxed);
            audio_state.playing.store(true, Ordering::Relaxed);
        }

        AudioCommand::SetPosition(position) => {
            audio_state.set_position(*position);
        }

        AudioCommand::SetBPM(bpm) => {
            audio_state.bpm.store(*bpm);
            let mut state = app_state.lock().unwrap();
            state.bpm = *bpm;
        }

        AudioCommand::SetMasterVolume(volume) => {
            audio_state.master_volume.store(*volume);
        }

        AudioCommand::UpdateTracks => {
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::SetTrackVolume(track_id, volume) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.volume = *volume;
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackVolume(*track_id, *volume));
        }

        AudioCommand::SetTrackPan(track_id, pan) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.pan = *pan;
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackPan(*track_id, *pan));
        }

        AudioCommand::SetTrackMute(track_id, mute) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.muted = *mute;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackMute(*track_id, *mute));
        }

        AudioCommand::SetTrackSolo(track_id, solo) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.solo = *solo;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackSolo(*track_id, *solo));
        }

        AudioCommand::SetTrackArmed(track_id, armed) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.armed = *armed;
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::AddPlugin(track_id, uri) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Ok(plugin_desc) =
                    crate::plugin::create_plugin_instance(uri, audio_state.sample_rate.load())
                {
                    track.plugin_chain.push(plugin_desc);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::RemovePlugin(track_id, plugin_idx) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if *plugin_idx < track.plugin_chain.len() {
                    track.plugin_chain.remove(*plugin_idx);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::SetPluginBypass(track_id, plugin_idx, bypass) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(plugin) = track.plugin_chain.get_mut(*plugin_idx) {
                    plugin.bypass = *bypass;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginBypass(
                *track_id,
                *plugin_idx,
                *bypass,
            ));
        }

        AudioCommand::SetPluginParam(track_id, plugin_idx, param_name, value) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(plugin) = track.plugin_chain.get_mut(*plugin_idx) {
                    plugin.params.insert(param_name.clone(), *value);
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginParam(
                *track_id,
                *plugin_idx,
                param_name.clone(),
                *value,
            ));
        }

        AudioCommand::SetLoopEnabled(enabled) => {
            audio_state.loop_enabled.store(*enabled, Ordering::Relaxed);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopEnabled(*enabled));
        }

        AudioCommand::SetLoopRegion(start, end) => {
            audio_state.loop_start.store(*start);
            audio_state.loop_end.store(*end);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopRegion(*start, *end));
        }

        AudioCommand::CreateMidiClip(track_id, start_beat, length_beats) => {
            use crate::model::clip::MidiClip;
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                let clip = MidiClip {
                    name: format!("MIDI Clip {}", track.midi_clips.len() + 1),
                    start_beat: *start_beat,
                    length_beats: *length_beats,
                    notes: Vec::new(),
                    color: Some((100, 150, 200)),
                    ..Default::default()
                };
                track.midi_clips.push(clip);
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::UpdateMidiClip(track_id, clip_id, notes) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.notes = notes.clone();
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::DeleteMidiClip(track_id, clip_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if *clip_id < track.midi_clips.len() {
                    track.midi_clips.remove(*clip_id);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::MoveMidiClip(track_id, clip_id, new_start_beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.start_beat = *new_start_beat;
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::AddAutomationPoint(track_id, target, beat, value) => {
            use crate::model::automation::{AutomationLane, AutomationMode, AutomationPoint};
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                // Find existing lane index or create a new one
                let lane_idx = if let Some(idx) = track
                    .automation_lanes
                    .iter()
                    .position(|l| l.parameter == *target)
                {
                    idx
                } else {
                    track.automation_lanes.push(AutomationLane {
                        parameter: target.clone(),
                        points: Vec::new(),
                        visible: true,
                        height: 30.0,
                        color: None,
                        write_mode: AutomationMode::Read,
                        read_enabled: true,
                    });
                    track.automation_lanes.len() - 1
                };

                // Push point
                if let Some(lane) = track.automation_lanes.get_mut(lane_idx) {
                    lane.points.push(AutomationPoint {
                        beat: *beat,
                        value: *value,
                    });
                    lane.points
                        .sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
                }

                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::RemoveAutomationPoint(track_id, lane_idx, beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(lane) = track.automation_lanes.get_mut(*lane_idx) {
                    lane.points.retain(|p| (p.beat - beat).abs() > 0.001);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot(app_state, realtime_tx);
        }

        AudioCommand::PreviewNote(track_id, pitch) => {
            let current_position = audio_state.get_position();
            let _ = realtime_tx.send(RealtimeCommand::PreviewNote(
                *track_id,
                *pitch,
                current_position,
            ));
        }

        AudioCommand::StopPreviewNote => {
            let _ = realtime_tx.send(RealtimeCommand::StopPreviewNote);
        }

        AudioCommand::SetTrackMonitor(track_id, enabled) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.monitor_enabled = *enabled;
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            // so the new monitor flag reaches the audio thread
            send_tracks_snapshot(app_state, realtime_tx);
        }

        _ => {
            // No-op
        }
    }
}

fn send_tracks_snapshot(app_state: &Arc<Mutex<AppState>>, realtime_tx: &Sender<RealtimeCommand>) {
    let state = app_state.lock().unwrap();
    let snapshots = crate::audio_snapshot::build_track_snapshots(&state.tracks);
    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
}
