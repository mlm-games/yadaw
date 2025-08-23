use crate::audio_state::{
    AudioClipSnapshot, AudioState, AutomationLaneSnapshot, AutomationPoint as AutomationPointSnap,
    CurveType as CurveTypeSnap, MidiClipSnapshot, MidiNoteSnapshot, PluginDescriptorSnapshot,
    RealtimeCommand, TrackSnapshot,
};
use crate::state::{
    AppState, AudioCommand, AutomationLane, AutomationMode, AutomationPoint, MidiClip, UIUpdate,
};

use crate::plugin;
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;
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
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
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
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::AddPlugin(track_id, uri) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Ok(plugin) =
                    crate::plugin::create_plugin_instance(uri, audio_state.sample_rate.load())
                {
                    track.plugin_chain.push(plugin);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::RemovePlugin(track_id, plugin_idx) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if *plugin_idx < track.plugin_chain.len() {
                    track.plugin_chain.remove(*plugin_idx);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
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
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::UpdateMidiClip(track_id, clip_id, notes) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.notes = notes.clone();
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::DeleteMidiClip(track_id, clip_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if *clip_id < track.midi_clips.len() {
                    track.midi_clips.remove(*clip_id);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::MoveMidiClip(track_id, clip_id, new_start_beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.start_beat = *new_start_beat;
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::AddAutomationPoint(track_id, target, beat, value) => {
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
                    track.automation_lanes.push(crate::state::AutomationLane {
                        parameter: target.clone(),
                        points: Vec::new(),
                        visible: true,
                        height: 30.0,
                        color: None,
                        write_mode: crate::state::AutomationMode::Read,
                        read_enabled: true,
                    });
                    track.automation_lanes.len() - 1
                };

                // Push point as a struct (not a tuple)
                if let Some(lane) = track.automation_lanes.get_mut(lane_idx) {
                    lane.points.push(crate::state::AutomationPoint {
                        beat: *beat,
                        value: *value,
                    });
                    lane.points
                        .sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
                }

                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::RemoveAutomationPoint(track_id, lane_idx, beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(lane) = track.automation_lanes.get_mut(*lane_idx) {
                    lane.points.retain(|p| (p.beat - beat).abs() > 0.001);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let snapshots = create_track_snapshots(app_state);
            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
        }

        AudioCommand::PreviewNote(track_id, pitch) => {
            let converter = crate::time_utils::TimeConverter::new(
                audio_state.sample_rate.load(),
                audio_state.bpm.load(),
            );
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

        _ => {
            // Nothin'
        }
    }
}

fn create_track_snapshots(app_state: &Arc<Mutex<AppState>>) -> Vec<TrackSnapshot> {
    let state = app_state.lock().unwrap();

    state
        .tracks
        .iter()
        .map(|track| {
            // Convert plugin descriptors to snapshots
            let plugin_chain = track
                .plugin_chain
                .iter()
                .map(|plugin| {
                    let params = Arc::new(DashMap::new());
                    for (key, value) in &plugin.params {
                        params.insert(key.clone(), *value);
                    }

                    PluginDescriptorSnapshot {
                        uri: plugin.uri.clone(),
                        name: plugin.name.clone(),
                        bypass: plugin.bypass,
                        params,
                    }
                })
                .collect();

            // Convert automation lanes to snapshots
            let automation_lanes = track
                .automation_lanes
                .iter()
                .map(|lane| AutomationLaneSnapshot {
                    parameter: lane.parameter.clone(),
                    points: lane
                        .points
                        .iter()
                        .map(|p| AutomationPointSnap {
                            beat: p.beat,
                            value: p.value,
                            curve_type: CurveTypeSnap::Linear,
                        })
                        .collect(),
                    visible: lane.visible,
                    height: lane.height,
                    color: lane.color,
                })
                .collect();

            // Convert audio clips to snapshots
            let audio_clips = track
                .audio_clips
                .iter()
                .map(|clip| AudioClipSnapshot {
                    name: clip.name.clone(),
                    start_beat: clip.start_beat,
                    length_beats: clip.length_beats,
                    samples: clip.samples.clone(),
                    sample_rate: clip.sample_rate,
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                    gain: clip.gain,
                })
                .collect();

            // Convert MIDI clips to snapshots
            let midi_clips = track
                .midi_clips
                .iter()
                .map(|clip| MidiClipSnapshot {
                    name: clip.name.clone(),
                    start_beat: clip.start_beat,
                    length_beats: clip.length_beats,
                    notes: clip
                        .notes
                        .iter()
                        .map(|note| MidiNoteSnapshot {
                            pitch: note.pitch,
                            velocity: note.velocity,
                            start: note.start,
                            duration: note.duration,
                        })
                        .collect(),
                    color: clip.color,
                })
                .collect();

            TrackSnapshot {
                name: track.name.clone(),
                volume: track.volume,
                pan: track.pan,
                muted: track.muted,
                solo: track.solo,
                armed: track.armed,
                is_midi: track.is_midi,
                audio_clips,
                midi_clips,
                plugin_chain,
                automation_lanes,
            }
        })
        .collect()
}
