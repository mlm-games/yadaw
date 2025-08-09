use crate::audio_state::{
    AudioClipSnapshot, AudioState, MidiNoteSnapshot, PatternSnapshot, PluginSnapshot,
    RealtimeCommand, TrackSnapshot,
};
use crate::plugin;
use crate::state::{AppState, AudioCommand, AutomationLane, OrderedFloat, Track, UIUpdate};
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub fn run_command_processor(
    app_state: Arc<Mutex<AppState>>,
    audio_state: Arc<AudioState>,
    commands: Receiver<AudioCommand>,
    realtime_tx: Sender<RealtimeCommand>,
    ui_tx: Sender<UIUpdate>,
) {
    loop {
        match commands.recv() {
            Ok(cmd) => {
                process_command(&app_state, &audio_state, cmd, &realtime_tx, &ui_tx);
            }
            Err(_) => {
                println!("Command processor: channel closed");
                break;
            }
        }
    }
}

fn process_command(
    app_state: &Arc<Mutex<AppState>>,
    audio_state: &Arc<AudioState>,
    cmd: AudioCommand,
    realtime_tx: &Sender<RealtimeCommand>,
    ui_tx: &Sender<UIUpdate>,
) {
    let should_push_undo = matches!(
        cmd,
        AudioCommand::AddPlugin(..)
            | AudioCommand::RemovePlugin(..)
            | AudioCommand::AddNote(..)
            | AudioCommand::RemoveNote(..)
            | AudioCommand::UpdateNote(..)
            | AudioCommand::AddAutomationPoint(..)
            | AudioCommand::RemoveAutomationPoint(..)
            | AudioCommand::UpdateAutomationPoint(..)
    );

    if should_push_undo {
        if let Ok(state) = app_state.lock() {
            let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
        }
    }

    println!("Processing command: {:?}", cmd);
    match cmd {
        AudioCommand::Play => {
            audio_state.playing.store(true, Ordering::Relaxed);
        }

        AudioCommand::Stop => {
            audio_state.playing.store(false, Ordering::Relaxed);
            audio_state.set_position(0.0);
            let _ = ui_tx.send(UIUpdate::Position(0.0));
        }

        AudioCommand::SetTrackVolume(id, vol) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(id) {
                    track.volume = vol;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackVolume(id, vol));
        }

        AudioCommand::SetTrackPan(id, pan) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(id) {
                    track.pan = pan;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackPan(id, pan));
        }

        AudioCommand::MuteTrack(id, mute) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(id) {
                    track.muted = mute;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackMute(id, mute));
        }

        AudioCommand::SoloTrack(id, solo) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(id) {
                    track.solo = solo;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackSolo(id, solo));
        }

        AudioCommand::AddPlugin(track_id, uri) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(track_id) {
                    // Create plugin descriptor
                    if let Ok(plugin_desc) =
                        plugin::create_plugin_instance(&uri, audio_state.sample_rate.load())
                    {
                        track.plugin_chain.push(plugin_desc);

                        // Send updated tracks to audio thread
                        let tracks = create_track_snapshots(&state.tracks);
                        let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                        let _ = ui_tx.send(UIUpdate::PluginAdded(track_id, uri));
                    }
                }
            }
        }

        AudioCommand::RemovePlugin(track_id, plugin_idx) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(track_id) {
                    if plugin_idx < track.plugin_chain.len() {
                        track.plugin_chain.remove(plugin_idx);

                        let tracks = create_track_snapshots(&state.tracks);
                        let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                    }
                }
            }
        }

        AudioCommand::SetPluginBypass(track_id, plugin_idx, bypass) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(plugin) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.plugin_chain.get_mut(plugin_idx))
                {
                    plugin.bypass = bypass;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginBypass(
                track_id, plugin_idx, bypass,
            ));
        }

        AudioCommand::SetPluginParam(track_id, plugin_idx, param_name, value) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(param) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.plugin_chain.get_mut(plugin_idx))
                    .and_then(|p| p.params.get_mut(&param_name))
                {
                    param.value = value;
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginParam(
                track_id, plugin_idx, param_name, value,
            ));
        }

        AudioCommand::AddNote(track_id, pattern_id, note) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(pattern) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.patterns.get_mut(pattern_id))
                {
                    pattern.notes.push(note);
                    pattern
                        .notes
                        .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

                    let tracks = create_track_snapshots(&state.tracks);
                    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                }
            }
        }

        AudioCommand::RemoveNote(track_id, pattern_id, note_index) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(pattern) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.patterns.get_mut(pattern_id))
                {
                    if note_index < pattern.notes.len() {
                        pattern.notes.remove(note_index);

                        let tracks = create_track_snapshots(&state.tracks);
                        let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                    }
                }
            }
        }

        AudioCommand::UpdateNote(track_id, pattern_id, note_index, new_note) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(pattern) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.patterns.get_mut(pattern_id))
                {
                    if let Some(note) = pattern.notes.get_mut(note_index) {
                        *note = new_note;
                        pattern
                            .notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

                        let tracks = create_track_snapshots(&state.tracks);
                        let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                    }
                }
            }
        }
        AudioCommand::AddAutomationPoint(track_id, target, beat, value) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(track) = state.tracks.get_mut(track_id) {
                    // Check if lane already exists
                    let lane_exists = track.automation_lanes.iter().any(|l| l.parameter == target);

                    if !lane_exists {
                        // Create new lane
                        track.automation_lanes.push(AutomationLane {
                            parameter: target.clone(),
                            points: BTreeMap::new(),
                            visible: true,
                        });
                    }

                    // Now find and update the lane
                    if let Some(lane) = track
                        .automation_lanes
                        .iter_mut()
                        .find(|l| l.parameter == target)
                    {
                        lane.points.insert(OrderedFloat(beat), value);
                    }

                    let tracks = create_track_snapshots(&state.tracks);
                    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                }
            }
        }
        AudioCommand::SetAutomationVisible(track_id, lane_idx, visible) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(lane) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.automation_lanes.get_mut(lane_idx))
                {
                    lane.visible = visible;

                    let tracks = create_track_snapshots(&state.tracks);
                    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                }
            }
        }

        AudioCommand::RemoveAutomationPoint(track_id, lane_idx, beat) => {
            if let Ok(mut state) = app_state.lock() {
                if let Some(lane) = state
                    .tracks
                    .get_mut(track_id)
                    .and_then(|t| t.automation_lanes.get_mut(lane_idx))
                {
                    lane.points.remove(&OrderedFloat(beat));

                    let tracks = create_track_snapshots(&state.tracks);
                    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
                }
            }
        }

        AudioCommand::UpdateTracks => {
            println!("Updating tracks in audio thread");
            if let Ok(state) = app_state.lock() {
                let tracks = create_track_snapshots(&state.tracks);
                println!("Sending {} track snapshots to audio thread", tracks.len());
                let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(tracks));
            }
        }

        AudioCommand::PreviewNote(track_id, pitch) => {
            let position = audio_state.get_position();
            let _ = realtime_tx.send(RealtimeCommand::PreviewNote(track_id, pitch, position));
        }

        AudioCommand::StopPreviewNote => {
            let _ = realtime_tx.send(RealtimeCommand::StopPreviewNote);
        }

        AudioCommand::StartRecording(track_id) => {
            if let Ok(mut state) = app_state.lock() {
                state.recording = true;
                audio_state.recording.store(true, Ordering::Relaxed);
            }
        }

        AudioCommand::StopRecording => {
            if let Ok(mut state) = app_state.lock() {
                state.recording = false;
                audio_state.recording.store(false, Ordering::Relaxed);
            }
        }

        _ => {
            // Handle other commands...
        }
    }
}

fn create_track_snapshots(tracks: &[Track]) -> Vec<TrackSnapshot> {
    println!("Creating snapshots for {} tracks", tracks.len());

    tracks
        .iter()
        .map(|track| TrackSnapshot {
            id: track.id,
            volume: track.volume,
            pan: track.pan,
            muted: track.muted,
            solo: track.solo,
            armed: track.armed,
            plugin_chain: track
                .plugin_chain
                .iter()
                .map(|p| {
                    let params = Arc::new(DashMap::new());
                    for (name, param) in &p.params {
                        params.insert(name.clone(), param.value);
                    }
                    PluginSnapshot {
                        uri: p.uri.clone(),
                        bypass: p.bypass,
                        params,
                    }
                })
                .collect(),
            patterns: track
                .patterns
                .iter()
                .map(|p| PatternSnapshot {
                    length: p.length,
                    notes: p
                        .notes
                        .iter()
                        .map(|n| MidiNoteSnapshot {
                            pitch: n.pitch,
                            velocity: n.velocity,
                            start: n.start,
                            duration: n.duration,
                        })
                        .collect(),
                })
                .collect(),
            is_midi: track.is_midi,
            audio_clips: track
                .audio_clips
                .iter()
                .map(|clip| {
                    Arc::new(AudioClipSnapshot {
                        start_beat: clip.start_beat,
                        length_beats: clip.length_beats,
                        samples: Arc::new(clip.samples.clone()),
                        sample_rate: clip.sample_rate,
                    })
                })
                .collect(),
            automation_lanes: track.automation_lanes.clone(),
        })
        .collect()
}
