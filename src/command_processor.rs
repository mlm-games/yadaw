use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam_channel::{Receiver, Sender};

use crate::audio_state::{AudioState, MidiNoteSnapshot, RealtimeCommand};
use crate::messages::{AudioCommand, UIUpdate};
use crate::model::PluginDescriptor;
use crate::model::plugin_api::BackendKind;
use crate::plugin::{create_plugin_instance, get_control_port_info};
use crate::project::AppState;

pub fn run_command_processor(
    app_state: Arc<std::sync::Mutex<AppState>>,
    audio_state: Arc<AudioState>,
    command_rx: Receiver<AudioCommand>,
    realtime_tx: Sender<RealtimeCommand>,
    ui_tx: Sender<UIUpdate>,
) {
    while let Ok(command) = command_rx.recv() {
        process_command(&command, &app_state, &audio_state, &realtime_tx, &ui_tx);
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

        AudioCommand::AddPluginUnified {
            track_id,
            backend,
            uri,
            display_name,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                let plugin_idx = track.plugin_chain.len();
                // seed descriptor
                let desc = if *backend == BackendKind::Lv2 {
                    create_plugin_instance(uri, audio_state.sample_rate.load()).unwrap_or_else(
                        |_| PluginDescriptor {
                            uri: uri.clone(),
                            name: display_name.clone(),
                            bypass: false,
                            params: HashMap::new(),
                            preset_name: None,
                            custom_name: None,
                        },
                    )
                } else {
                    PluginDescriptor {
                        uri: uri.clone(),
                        name: display_name.clone(),
                        bypass: false,
                        params: HashMap::new(),
                        preset_name: None,
                        custom_name: None,
                    }
                };
                track.plugin_chain.push(desc);
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                let tracks_clone = state.tracks.clone();
                drop(state);

                // ask audio thread to instantiate
                let _ = realtime_tx.send(RealtimeCommand::AddUnifiedPlugin {
                    track_id: *track_id,
                    plugin_idx,
                    backend: *backend,
                    uri: uri.clone(),
                });

                // refresh tracks snapshot
                let snapshots = crate::audio_snapshot::build_track_snapshots(&tracks_clone);
                let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
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
            // Look up min/max/default (fall back to 0..1)
            let (min_v, max_v) = get_control_port_info(
                {
                    // we need the plugin URI; read it without holding the lock too long
                    let state = app_state.lock().unwrap();
                    state
                        .tracks
                        .get(*track_id)
                        .and_then(|t| t.plugin_chain.get(*plugin_idx))
                        .map(|p| p.uri.clone())
                }
                .as_deref()
                .unwrap_or(""),
                param_name,
            )
            .map(|m| (m.min, m.max))
            .unwrap_or((0.0, 1.0));

            let v = value.clamp(min_v, max_v);

            // Update model
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(plugin) = track.plugin_chain.get_mut(*plugin_idx) {
                    plugin.params.insert(param_name.clone(), v);
                }
            }

            // RT update
            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginParam(
                *track_id,
                *plugin_idx,
                param_name.clone(),
                v,
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
            let mut state = app_state.lock().unwrap();

            // 1) Snapshot undo ONCE at start
            let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));

            // 2) Get source clip (borrow checker safe - we own state)
            let (src_clip, src_pid) = {
                let track = match state.tracks.get(*track_id) {
                    Some(t) => t,
                    None => return,
                };
                let clip = match track.midi_clips.get(*clip_id) {
                    Some(c) => c,
                    None => return,
                };
                (clip.clone(), clip.pattern_id)
            };

            // 3) Assign pattern_id to source if needed
            let final_pid = if src_pid.is_none() {
                let new_pid = state.fresh_id();
                // Safe: we still hold state lock
                if let Some(track) = state.tracks.get_mut(*track_id) {
                    if let Some(clip) = track.midi_clips.get_mut(*clip_id) {
                        clip.pattern_id = Some(new_pid);
                    }
                }
                new_pid
            } else {
                src_pid.unwrap()
            };

            // 4) Build duplicate
            let mut dup = src_clip;
            dup.id = state.fresh_id();
            dup.start_beat += dup.length_beats;
            dup.pattern_id = Some(final_pid);
            dup.name = format!("{} (alias)", dup.name);

            // Assign note IDs
            for n in &mut dup.notes {
                if n.id == 0 {
                    n.id = state.fresh_id();
                }
            }

            // 5) Insert duplicate
            if let Some(track) = state.tracks.get_mut(*track_id) {
                let insert_at = (*clip_id + 1).min(track.midi_clips.len());
                track.midi_clips.insert(insert_at, dup);
            }

            send_tracks_snapshot_locked(&state, realtime_tx);
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
    }
}

fn send_tracks_snapshot_locked(state: &AppState, realtime_tx: &Sender<RealtimeCommand>) {
    let snapshots = crate::audio_snapshot::build_track_snapshots(&state.tracks);
    let _ = realtime_tx.send(RealtimeCommand::UpdateTracks(snapshots));
}
