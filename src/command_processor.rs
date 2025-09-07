use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;

use crate::audio_state::{AudioState, MidiNoteSnapshot, RealtimeCommand};
use crate::messages::{AudioCommand, NoteDelta, UIUpdate};
use crate::project::AppState;

const MIDI_PREVIEW_THROTTLE_MS: u64 = 30;

struct MidiEditSession {
    pending: Vec<crate::model::MidiNote>,
    last_send: Instant,
    base_count: usize,
}

pub fn run_command_processor(
    app_state: Arc<std::sync::Mutex<AppState>>,
    audio_state: Arc<AudioState>,
    command_rx: Receiver<AudioCommand>,
    realtime_tx: Sender<RealtimeCommand>,
    ui_tx: Sender<UIUpdate>,
) {
    let mut sessions: HashMap<(usize, usize, u64), MidiEditSession> = HashMap::new();

    while let Ok(command) = command_rx.recv() {
        process_command(
            &command,
            &app_state,
            &audio_state,
            &realtime_tx,
            &ui_tx,
            &mut sessions,
        );
    }
}

fn apply_delta_vec(vec: &mut Vec<crate::model::MidiNote>, delta: &NoteDelta) {
    match *delta {
        NoteDelta::Set { index, note } => {
            if index < vec.len() {
                vec[index] = note;
            }
        }
        NoteDelta::Add { index, note } => {
            if index <= vec.len() {
                vec.insert(index, note);
            } else {
                vec.push(note);
            }
        }
        NoteDelta::Remove { index } => {
            if index < vec.len() {
                vec.remove(index);
            }
        }
    }
}

fn notes_to_snapshot(notes: &[crate::model::MidiNote]) -> Vec<MidiNoteSnapshot> {
    notes
        .iter()
        .map(|n| MidiNoteSnapshot {
            pitch: n.pitch,
            velocity: n.velocity,
            start: n.start,
            duration: n.duration,
        })
        .collect()
}

fn process_command(
    command: &AudioCommand,
    app_state: &Arc<std::sync::Mutex<AppState>>,
    audio_state: &Arc<AudioState>,
    realtime_tx: &Sender<RealtimeCommand>,
    ui_tx: &Sender<UIUpdate>,
    sessions: &mut HashMap<(usize, usize, u64), MidiEditSession>,
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
            let state = app_state.lock().unwrap();
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::SetTrackVolume(track_id, volume) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.volume = *volume;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackVolume(*track_id, *volume));
        }
        AudioCommand::SetTrackPan(track_id, pan) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                track.pan = *pan;
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
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::SetTrackInput(_, _)
        | AudioCommand::SetTrackOutput(_, _)
        | AudioCommand::FreezeTrack(_)
        | AudioCommand::UnfreezeTrack(_) => {}

        AudioCommand::AddPlugin(track_id, uri) => {
            let sample_rate = audio_state.sample_rate.load();
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Ok(plugin_desc) = crate::plugin::create_plugin_instance(uri, sample_rate) {
                    let plugin_idx = track.plugin_chain.len();
                    track.plugin_chain.push(plugin_desc.clone());
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));

                    let params_dashmap = Arc::new(DashMap::new());
                    for (key, value) in plugin_desc.params.iter() {
                        params_dashmap.insert(key.clone(), *value);
                    }

                    let tracks_clone = state.tracks.clone();
                    drop(state);

                    match crate::plugin_host::instantiate(uri) {
                        Ok(mut instance) => {
                            for entry in params_dashmap.iter() {
                                instance.set_parameter(entry.key(), *entry.value());
                            }
                            instance.set_params_arc(params_dashmap.clone());

                            let _ = realtime_tx.send(RealtimeCommand::AddPluginInstance {
                                track_id: *track_id,
                                plugin_idx,
                                instance,
                                descriptor: params_dashmap,
                                uri: uri.to_string(),
                                bypass: plugin_desc.bypass,
                            });

                            let snapshots =
                                crate::audio_snapshot::build_track_snapshots(&tracks_clone);
                            let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
                        }
                        Err(e) => {
                            eprintln!("Failed to instantiate plugin: {}", e);
                            let _ = ui_tx
                                .send(UIUpdate::Error(format!("Failed to load plugin: {}", e)));
                            let mut state = app_state.lock().unwrap();
                            if let Some(track) = state.tracks.get_mut(*track_id) {
                                track.plugin_chain.pop();
                            }
                        }
                    }
                }
            }
        }
        AudioCommand::RemovePlugin(track_id, plugin_idx) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if *plugin_idx < track.plugin_chain.len() {
                    track.plugin_chain.remove(*plugin_idx);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            let _ = realtime_tx.send(RealtimeCommand::RemovePluginInstance {
                track_id: *track_id,
                plugin_idx: *plugin_idx,
            });
            let snapshots = crate::audio_snapshot::build_track_snapshots(&state.tracks);
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

        AudioCommand::MovePlugin(_, _, _)
        | AudioCommand::LoadPluginPreset(_, _, _)
        | AudioCommand::SavePluginPreset(_, _, _) => {}

        AudioCommand::SetLoopEnabled(enabled) => {
            audio_state.loop_enabled.store(*enabled, Ordering::Relaxed);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopEnabled(*enabled));
        }
        AudioCommand::SetLoopRegion(start, end) => {
            audio_state.loop_start.store(*start);
            audio_state.loop_end.store(*end);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopRegion(*start, *end));
        }

        // ---------- MIDI CLIPS ----------
        AudioCommand::CreateMidiClip(track_id, start_beat, length_beats) => {
            // pre-allocate id before borrowing track
            let new_id = {
                let mut state = app_state.lock().unwrap();
                state.fresh_id()
            };
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                let clip = crate::model::clip::MidiClip {
                    id: new_id,
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
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::CreateMidiClipWithData(track_id, clip) => {
            // fix ids before borrowing track
            let mut new_clip = clip.clone();
            {
                let mut state = app_state.lock().unwrap();
                if new_clip.id == 0 {
                    new_clip.id = state.fresh_id();
                }
                for n in &mut new_clip.notes {
                    if n.id == 0 {
                        n.id = state.fresh_id();
                    }
                }
            }
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if track.is_midi {
                    track.midi_clips.push(new_clip);
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::UpdateMidiClip(track_id, clip_id, notes) => {
            // work on owned vector and assign ids before any track borrow
            let mut notes_owned = notes.clone();
            {
                let mut state = app_state.lock().unwrap();
                for n in &mut notes_owned {
                    if n.id == 0 {
                        n.id = state.fresh_id();
                    }
                }
            }
            let mut state = app_state.lock().unwrap();

            let pattern_id = state
                .tracks
                .get(*track_id)
                .and_then(|t| t.midi_clips.get(*clip_id))
                .and_then(|c| c.pattern_id);

            if let Some(pid) = pattern_id {
                // Mirror to all clips with the same pattern_id
                for t in &mut state.tracks {
                    for c in &mut t.midi_clips {
                        if c.pattern_id == Some(pid) {
                            c.notes = notes_owned.clone();
                            c.notes
                                .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                        }
                    }
                }
            } else {
                if let Some(track) = state.tracks.get_mut(*track_id) {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        clip.notes = notes_owned.clone();
                        clip.notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                    }
                }
            }

            // Send RT narrow update
            let notes_snapshot: Vec<MidiNoteSnapshot> = notes_owned
                .iter()
                .map(|n| MidiNoteSnapshot {
                    pitch: n.pitch,
                    velocity: n.velocity,
                    start: n.start,
                    duration: n.duration,
                })
                .collect();
            let _ = realtime_tx.send(RealtimeCommand::UpdateMidiClipNotes {
                track_id: *track_id,
                clip_id: *clip_id,
                notes: notes_snapshot,
            });
        }

        AudioCommand::DeleteMidiClip(track_id, clip_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if *clip_id < track.midi_clips.len() {
                    track.midi_clips.remove(*clip_id);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::MoveMidiClip(track_id, clip_id, new_start_beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.start_beat = *new_start_beat;
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::ResizeMidiClip(track_id, clip_id, new_start, new_length) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if track.is_midi {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        clip.start_beat = *new_start;
                        clip.length_beats = (*new_length).max(0.0);
                    }
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::DuplicateMidiClip(track_id, clip_id) => {
            // clone source first without holding &mut state borrow
            let source = {
                let state = app_state.lock().unwrap();
                state
                    .tracks
                    .get(*track_id)
                    .and_then(|t| t.midi_clips.get(*clip_id).cloned())
            };
            if let Some(mut clip) = source {
                // assign new ids
                {
                    let mut state = app_state.lock().unwrap();
                    clip.id = state.fresh_id();
                    for n in &mut clip.notes {
                        n.id = state.fresh_id();
                    }
                }
                clip.name = format!("{} (copy)", clip.name);
                clip.start_beat = clip.start_beat + clip.length_beats;

                let mut state = app_state.lock().unwrap();
                if let Some(track) = state.tracks.get_mut(*track_id) {
                    track.midi_clips.insert(*clip_id + 1, clip);
                }
                send_tracks_snapshot_locked(&state, realtime_tx);
            }
        }
        AudioCommand::SplitMidiClip(_, _, _) => {}

        // ---------- AUDIO CLIPS ----------
        AudioCommand::MoveAudioClip(track_id, clip_id, new_start) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if !track.is_midi {
                    if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                        clip.start_beat = *new_start;
                    }
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::ResizeAudioClip(track_id, clip_id, new_start, new_length) => {
            let mut state = app_state.lock().unwrap();
            let bpm = state.bpm;
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if !track.is_midi {
                    if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                        let spb = (clip.sample_rate as f64) * (60.0 / bpm as f64);
                        if *new_start > clip.start_beat {
                            let delta_beats = *new_start - clip.start_beat;
                            let drop_samples = (delta_beats * spb).round() as usize;
                            if drop_samples >= clip.samples.len() {
                                clip.samples.clear();
                            } else {
                                clip.samples.drain(0..drop_samples);
                            }
                        } else if *new_start < clip.start_beat {
                            let delta_beats = clip.start_beat - *new_start;
                            let pad_samples = (delta_beats * spb).round() as usize;
                            if pad_samples > 0 {
                                let mut padded =
                                    Vec::with_capacity(pad_samples + clip.samples.len());
                                padded.resize(pad_samples, 0.0);
                                padded.extend_from_slice(&clip.samples);
                                clip.samples = padded;
                            }
                        }
                        let target_samples = (*new_length * spb).round() as usize;
                        match target_samples.cmp(&clip.samples.len()) {
                            std::cmp::Ordering::Less => clip.samples.truncate(target_samples),
                            std::cmp::Ordering::Greater => {
                                let pad = target_samples - clip.samples.len();
                                clip.samples.extend(std::iter::repeat(0.0).take(pad));
                            }
                            _ => {}
                        }
                        clip.start_beat = *new_start;
                        clip.length_beats = new_length.max(0.0);
                    }
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::DuplicateAudioClip(track_id, clip_id) => {
            // clone source without holding &mut state borrow
            let source = {
                let state = app_state.lock().unwrap();
                state
                    .tracks
                    .get(*track_id)
                    .and_then(|t| t.audio_clips.get(*clip_id).cloned())
            };
            if let Some(mut clip) = source {
                // assign id before borrowing track
                {
                    let mut state = app_state.lock().unwrap();
                    clip.id = state.fresh_id();
                }
                clip.name = format!("{} (copy)", clip.name);
                clip.start_beat = clip.start_beat + clip.length_beats;

                let mut state = app_state.lock().unwrap();
                if let Some(track) = state.tracks.get_mut(*track_id) {
                    track.audio_clips.insert(*clip_id + 1, clip);
                }
                send_tracks_snapshot_locked(&state, realtime_tx);
            }
        }
        AudioCommand::SplitAudioClip(_, _, _)
        | AudioCommand::SetAudioClipGain(_, _, _)
        | AudioCommand::SetAudioClipFadeIn(_, _, _)
        | AudioCommand::SetAudioClipFadeOut(_, _, _) => {}
        AudioCommand::DeleteAudioClip(track_id, clip_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if !track.is_midi && *clip_id < track.audio_clips.len() {
                    track.audio_clips.remove(*clip_id);
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        // ---------- AUTOMATION ----------
        AudioCommand::AddAutomationPoint(track_id, target, beat, value) => {
            use crate::model::automation::{AutomationLane, AutomationMode, AutomationPoint};
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
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
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::RemoveAutomationPoint(track_id, lane_idx, beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(lane) = track.automation_lanes.get_mut(*lane_idx) {
                    lane.points.retain(|p| (p.beat - beat).abs() > 0.001);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
        AudioCommand::UpdateAutomationPoint(_, _, _, _, _)
        | AudioCommand::SetAutomationMode(_, _, _)
        | AudioCommand::ClearAutomationLane(_, _) => {}

        // ---------- PREVIEW ----------
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
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::AddSend(_, _, _)
        | AudioCommand::RemoveSend(_, _)
        | AudioCommand::SetSendAmount(_, _, _)
        | AudioCommand::SetSendPreFader(_, _, _)
        | AudioCommand::CreateGroup(_, _)
        | AudioCommand::RemoveGroup(_)
        | AudioCommand::AddTrackToGroup(_, _)
        | AudioCommand::RemoveTrackFromGroup(_) => {}

        // ---------- NOTES ----------
        AudioCommand::AddNote(track_id, clip_id, note_ref) => {
            // clone and assign id before borrowing track
            let mut note = note_ref.clone();
            if note.id == 0 {
                let mut state = app_state.lock().unwrap();
                note.id = state.fresh_id();
            }
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if track.is_midi {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        clip.notes.push(note);
                        clip.notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                        let notes_snapshot: Vec<MidiNoteSnapshot> = clip
                            .notes
                            .iter()
                            .map(|n| MidiNoteSnapshot {
                                pitch: n.pitch,
                                velocity: n.velocity,
                                start: n.start,
                                duration: n.duration,
                            })
                            .collect();
                        let _ = realtime_tx.send(RealtimeCommand::UpdateMidiClipNotes {
                            track_id: *track_id,
                            clip_id: *clip_id,
                            notes: notes_snapshot,
                        });
                        return;
                    }
                }
            }
        }
        AudioCommand::RemoveNote(track_id, clip_id, note_index) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if track.is_midi {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        if *note_index < clip.notes.len() {
                            clip.notes.remove(*note_index);
                            let notes_snapshot: Vec<MidiNoteSnapshot> = clip
                                .notes
                                .iter()
                                .map(|n| MidiNoteSnapshot {
                                    pitch: n.pitch,
                                    velocity: n.velocity,
                                    start: n.start,
                                    duration: n.duration,
                                })
                                .collect();
                            let _ = realtime_tx.send(RealtimeCommand::UpdateMidiClipNotes {
                                track_id: *track_id,
                                clip_id: *clip_id,
                                notes: notes_snapshot,
                            });
                            return;
                        }
                    }
                }
            }
        }
        AudioCommand::UpdateNote(track_id, clip_id, note_index, note_ref) => {
            // clone and assign id before borrowing track
            let mut new_note = note_ref.clone();
            if new_note.id == 0 {
                let mut state = app_state.lock().unwrap();
                new_note.id = state.fresh_id();
            }
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if track.is_midi {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        if *note_index < clip.notes.len() {
                            clip.notes[*note_index] = new_note;
                            clip.notes
                                .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                            let notes_snapshot: Vec<MidiNoteSnapshot> = clip
                                .notes
                                .iter()
                                .map(|n| MidiNoteSnapshot {
                                    pitch: n.pitch,
                                    velocity: n.velocity,
                                    start: n.start,
                                    duration: n.duration,
                                })
                                .collect();
                            let _ = realtime_tx.send(RealtimeCommand::UpdateMidiClipNotes {
                                track_id: *track_id,
                                clip_id: *clip_id,
                                notes: notes_snapshot,
                            });
                            return;
                        }
                    }
                }
            }
        }

        // ---------- MIDI EDIT SESSIONS ----------
        AudioCommand::BeginMidiEdit {
            track_id,
            clip_id,
            session_id,
            base_note_count,
        } => {
            let state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get(*track_id) {
                if let Some(clip) = track.midi_clips.get(*clip_id) {
                    sessions.insert(
                        (*track_id, *clip_id, *session_id),
                        MidiEditSession {
                            pending: clip.notes.clone(),
                            last_send: Instant::now()
                                .checked_sub(Duration::from_millis(MIDI_PREVIEW_THROTTLE_MS))
                                .unwrap_or_else(Instant::now),
                            base_count: *base_note_count,
                        },
                    );
                    let _ = realtime_tx.send(RealtimeCommand::BeginMidiClipEdit {
                        track_id: *track_id,
                        clip_id: *clip_id,
                        session_id: *session_id,
                    });
                }
            }
        }
        AudioCommand::ApplyMidiNoteDelta {
            track_id,
            clip_id,
            session_id,
            delta,
        } => {
            if let Some(sess) = sessions.get_mut(&(*track_id, *clip_id, *session_id)) {
                apply_delta_vec(&mut sess.pending, delta);
                if sess.last_send.elapsed() >= Duration::from_millis(MIDI_PREVIEW_THROTTLE_MS) {
                    let mut state = app_state.lock().unwrap();
                    if let Some(track) = state.tracks.get_mut(*track_id) {
                        if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                            clip.notes = sess.pending.clone();
                        }
                    }
                    drop(state);
                    let _ = realtime_tx.send(RealtimeCommand::PreviewMidiClipNotes {
                        track_id: *track_id,
                        clip_id: *clip_id,
                        session_id: *session_id,
                        notes: notes_to_snapshot(&sess.pending),
                    });
                    sess.last_send = Instant::now();
                }
            }
        }
        AudioCommand::CommitMidiEdit {
            track_id,
            clip_id,
            session_id,
            final_notes,
        } => {
            // assign ids to any new notes before committing
            let mut final_owned = final_notes.clone();
            {
                let mut state = app_state.lock().unwrap();
                for n in &mut final_owned {
                    if n.id == 0 {
                        n.id = state.fresh_id();
                    }
                }
            }
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.notes = final_owned.clone();
                    clip.notes
                        .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                }
            }
            let notes_snapshot: Vec<MidiNoteSnapshot> = final_owned
                .iter()
                .map(|n| MidiNoteSnapshot {
                    pitch: n.pitch,
                    velocity: n.velocity,
                    start: n.start,
                    duration: n.duration,
                })
                .collect();
            let _ = realtime_tx.send(RealtimeCommand::UpdateMidiClipNotes {
                track_id: *track_id,
                clip_id: *clip_id,
                notes: notes_snapshot,
            });
            sessions.remove(&(*track_id, *clip_id, *session_id));
        }

        // ---------- ID RESERVATION ----------
        AudioCommand::ReserveNoteIds(count) => {
            let mut ids = Vec::with_capacity(*count);
            {
                let mut state = app_state.lock().unwrap();
                for _ in 0..*count {
                    ids.push(state.fresh_id());
                }
            }
            let _ = ui_tx.send(UIUpdate::ReservedNoteIds(ids));
        }

        AudioCommand::ToggleClipLoop(track_id, clip_id, enabled) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.loop_enabled = *enabled;
                    if clip.content_len_beats <= 0.0 {
                        clip.content_len_beats = clip.length_beats.max(0.000001);
                    }
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::MakeClipAlias(track_id, clip_id) => {
            let needs_pid = {
                let state = app_state.lock().unwrap();
                state
                    .tracks
                    .get(*track_id)
                    .and_then(|t| t.midi_clips.get(*clip_id))
                    .map(|c| c.pattern_id.is_none())
                    .unwrap_or(false)
            };

            if !needs_pid {
                return;
            }

            // Assign a new pattern_id in a separate mutable scope
            let mut state = app_state.lock().unwrap();
            let new_id = state.fresh_id(); // safe (no field borrows yet)
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    // Double-check in case of races
                    if clip.pattern_id.is_none() {
                        clip.pattern_id = Some(new_id);
                    }
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::MakeClipUnique(track_id, clip_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    clip.pattern_id = None;
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::SetClipQuantize(tid, cid, grid, strength, swing, enabled) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*tid) {
                if let Some(clip) = track.midi_clips.get_mut(*cid) {
                    clip.quantize_grid = *grid;
                    clip.quantize_strength = strength.clamp(0.0, 1.0);
                    clip.swing = *swing;
                    clip.quantize_enabled = *enabled;
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::DuplicateMidiClipAsAlias(track_id, clip_id) => {
            // 0) Push a single undo snapshot (no mutation here)
            {
                let state = app_state.lock().unwrap();
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }

            // 1) Snapshot source details without holding &mut borrows
            let (src_clone, src_pid, src_len, src_name) = {
                let state = app_state.lock().unwrap();
                let track = match state.tracks.get(*track_id) {
                    Some(t) => t,
                    None => return,
                };
                let src = match track.midi_clips.get(*clip_id) {
                    Some(c) => c,
                    None => return,
                };
                (
                    src.clone(),
                    src.pattern_id,
                    src.length_beats,
                    src.name.clone(),
                )
            };

            // 2) Ensure the source has a pattern_id (do this in its own scope)
            let pid_final = if src_pid.is_none() {
                let mut state = app_state.lock().unwrap();
                let new_pid = state.fresh_id(); // safe: no clip borrowed now
                if let Some(track) = state.tracks.get_mut(*track_id) {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        if clip.pattern_id.is_none() {
                            clip.pattern_id = Some(new_pid);
                        }
                    }
                }
                new_pid
            } else {
                src_pid.unwrap()
            };

            // 3) Build duplicate from the cloned source (no outstanding borrows)
            let mut dup = src_clone;
            {
                let mut state = app_state.lock().unwrap();
                dup.id = state.fresh_id();
            }
            dup.start_beat = dup.start_beat + src_len;
            dup.pattern_id = Some(pid_final);
            dup.name = format!("{} (alias)", src_name);

            // Ensure note ids if you rely on global uniqueness
            {
                let mut state = app_state.lock().unwrap();
                for n in &mut dup.notes {
                    if n.id == 0 {
                        n.id = state.fresh_id();
                    }
                }
                if let Some(track) = state.tracks.get_mut(*track_id) {
                    let insert_at = (*clip_id + 1).min(track.midi_clips.len());
                    track.midi_clips.insert(insert_at, dup);
                }
                // 4) Refresh audio thread once
                send_tracks_snapshot_locked(&state, realtime_tx);
            }
        }

        AudioCommand::SetClipContentOffset(track_id, clip_id, new_off) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                    let len = clip.content_len_beats.max(0.000001);
                    // Wrap offset into [0, len)
                    clip.content_offset_beats = ((*new_off % len) + len) % len;
                }
            }
            send_tracks_snapshot_locked(&state, realtime_tx);
        }

        AudioCommand::UpdateMidiClipsSameNotes { targets, notes } => {
            let mut state = app_state.lock().unwrap();
            // Single undo for the whole batch
            let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));

            // Assign ids once if needed
            let mut notes_owned = notes.clone();
            for n in &mut notes_owned {
                if n.id == 0 {
                    n.id = state.fresh_id();
                }
            }

            for (t, c) in targets {
                if let Some(track) = state.tracks.get_mut(*t) {
                    if let Some(clip) = track.midi_clips.get_mut(*c) {
                        // If clip is pooled alias: changing any member will mirror via UpdateMidiClip; but here we set explicitly
                        clip.notes = notes_owned.clone();
                        clip.notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                    }
                }
            }
            // One refresh is enough
            send_tracks_snapshot_locked(&state, realtime_tx);
        }
    }
}

fn send_tracks_snapshot_locked(state: &AppState, realtime_tx: &Sender<RealtimeCommand>) {
    let snapshots = crate::audio_snapshot::build_track_snapshots(&state.tracks);
    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
}
