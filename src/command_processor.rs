use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam_channel::{Receiver, Sender};

use crate::audio_export::AudioExporter;
use crate::audio_state::{AudioGraphSnapshot, AudioState, MidiNoteSnapshot, RealtimeCommand};
use crate::messages::{AudioCommand, UIUpdate};
use crate::midi_input::MidiInputHandler;
use crate::model::{AutomationPoint, MidiClip, PluginDescriptor};
use crate::plugin::{create_plugin_instance, get_control_port_info};
use crate::project::AppState;

pub fn run_command_processor(
    app_state: Arc<std::sync::Mutex<AppState>>,
    audio_state: Arc<AudioState>,
    command_rx: Receiver<AudioCommand>,
    realtime_tx: Sender<RealtimeCommand>,
    ui_tx: Sender<UIUpdate>,
    snapshot_tx: Sender<AudioGraphSnapshot>,
    midi_input_handler: Option<Arc<MidiInputHandler>>,
) {
    let mut midi_recording_state: Option<MidiRecordingState> = None;
    while let Ok(command) = command_rx.recv() {
        process_command(
            &command,
            &mut midi_recording_state,
            &app_state,
            &audio_state,
            &realtime_tx,
            &ui_tx,
            &snapshot_tx,
            &midi_input_handler,
        );
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
    midi_recording_state: &mut Option<MidiRecordingState>,
    app_state: &Arc<std::sync::Mutex<AppState>>,
    audio_state: &Arc<AudioState>,
    realtime_tx: &Sender<RealtimeCommand>,
    ui_tx: &Sender<UIUpdate>,
    snapshot_tx: &Sender<AudioGraphSnapshot>,
    midi_input_handler: &Option<Arc<MidiInputHandler>>,
) {
    match command {
        AudioCommand::Play => {
            audio_state.playing.store(true, Ordering::Relaxed);
        }
        AudioCommand::Stop => {
            audio_state.playing.store(false, Ordering::Relaxed);
            audio_state.recording.store(false, Ordering::Relaxed);
            if midi_recording_state.is_some() {
                log::info!("Stopping MIDI recording due to transport stop.");
                *midi_recording_state = None;
                send_graph_snapshot_locked(&app_state.lock().unwrap(), snapshot_tx);
            }
        }
        AudioCommand::Pause => {
            audio_state.playing.store(false, Ordering::Relaxed);
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
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        // Track commands using IDs
        AudioCommand::SetTrackVolume(track_id, volume) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                track.volume = *volume;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackVolume(*track_id, *volume));
        }
        AudioCommand::SetTrackPan(track_id, pan) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                track.pan = *pan;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackPan(*track_id, *pan));
        }
        AudioCommand::SetTrackMute(track_id, mute) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                track.muted = *mute;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackMute(*track_id, *mute));
        }
        AudioCommand::SetTrackSolo(track_id, solo) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                track.solo = *solo;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackSolo(*track_id, *solo));
        }
        AudioCommand::ArmForRecording(track_id, armed) => {
            let mut state = app_state.lock().unwrap();

            let target_is_midi = state.tracks.get(track_id).map_or(false, |t| t.is_midi);

            if *armed {
                for (id, track) in state.tracks.iter_mut() {
                    if *id == *track_id {
                        track.armed = true;
                    } else if track.is_midi == target_is_midi {
                        // Disarm other tracks of the same type
                        track.armed = false;
                    }
                }
            } else {
                // If we are disarming, just do it for the target track.
                if let Some(track) = state.tracks.get_mut(track_id) {
                    track.armed = false;
                }
            }

            drop(state);
            send_graph_snapshot_locked(&app_state.lock().unwrap(), snapshot_tx);
        }

        AudioCommand::FinalizeRecording => {
            //HACK?, this is just a no-op for the processor.
            // Only for pending state checks for the end of recording. The acual logic is in the audio callback.
            log::info!("FinalizeRecording command received.");
        }

        AudioCommand::StartRecording => {
            audio_state.recording.store(true, Ordering::Relaxed);
            audio_state.playing.store(true, Ordering::Relaxed);

            let (armed_midi_track_id, bpm, sr) = {
                let state = app_state.lock().unwrap();
                (
                    state
                        .tracks
                        .values()
                        .find(|t| t.is_midi && t.armed)
                        .map(|t| t.id),
                    state.bpm,
                    state.sample_rate,
                )
            };

            if let Some(track_id) = armed_midi_track_id {
                if midi_recording_state.is_none() {
                    let start_beat = crate::time_utils::quick::samples_to_beats(
                        audio_state.get_position(),
                        sr,
                        bpm,
                    );

                    let mut state = app_state.lock().unwrap();
                    let track_exists = state.tracks.contains_key(&track_id);

                    if track_exists {
                        let clip_exists = state.tracks[&track_id].midi_clips.iter().any(|c| {
                            start_beat >= c.start_beat
                                && start_beat < (c.start_beat + c.length_beats)
                        });

                        if !clip_exists {
                            // Generate the ID *before* mutably borrowing the track
                            let new_clip_id = state.fresh_id();

                            // Now we can safely get the mutable track reference
                            let track = state.tracks.get_mut(&track_id).unwrap();

                            log::info!(
                                "No existing clip at start beat {}. Creating a new one.",
                                start_beat
                            );
                            let new_clip = MidiClip {
                                id: new_clip_id,
                                name: format!("Rec @ Beat {:.1}", start_beat),
                                start_beat: start_beat.floor(),
                                length_beats: 64.0,
                                ..Default::default()
                            };
                            track.midi_clips.push(new_clip);
                            track
                                .midi_clips
                                .sort_by(|a, b| a.start_beat.partial_cmp(&b.start_beat).unwrap());
                        }
                    }
                    drop(state);

                    log::info!("Starting MIDI recording on track {}", track_id);
                    *midi_recording_state = Some(MidiRecordingState {
                        track_id,
                        active_notes: HashMap::new(),
                    });
                }
            }
        }

        AudioCommand::StopRecording => {
            audio_state.recording.store(false, Ordering::Relaxed);
            if midi_recording_state.is_some() {
                log::info!("Stopping MIDI recording.");
                *midi_recording_state = None;
                send_graph_snapshot_locked(&app_state.lock().unwrap(), snapshot_tx);
            }
        }

        AudioCommand::MidiInput(raw_message) => {
            if let Some(state) = midi_recording_state {
                let status = raw_message.message[0];
                let data1 = raw_message.message[1];
                let data2 = raw_message.message[2];
                let channel = status & 0x0F;
                let message_type = status & 0xF0;

                let current_beat = {
                    let pos_samples = audio_state.get_position();
                    let sr = audio_state.sample_rate.load();
                    let bpm = audio_state.bpm.load();
                    crate::time_utils::quick::samples_to_beats(pos_samples, sr, bpm)
                };

                match message_type {
                    0x90 if data2 > 0 => {
                        // Note On
                        state
                            .active_notes
                            .insert((data1, channel), (current_beat, data2));
                    }
                    0x80 | 0x90 => {
                        // Note Off
                        if let Some((start_beat, velocity)) =
                            state.active_notes.remove(&(data1, channel))
                        {
                            let duration = (current_beat - start_beat).max(0.01);

                            let mut app_state_guard = app_state.lock().unwrap();

                            // Find the clip first without holding a mutable borrow on it
                            let clip_exists = app_state_guard.tracks.get(&state.track_id).map_or(
                                false,
                                |track| {
                                    track.midi_clips.iter().any(|c| {
                                        start_beat >= c.start_beat
                                            && start_beat < (c.start_beat + c.length_beats)
                                    })
                                },
                            );

                            if clip_exists {
                                // Generate ID before getting mutable references
                                let new_note_id = app_state_guard.fresh_id();

                                // Now get the mutable references safely
                                let track =
                                    app_state_guard.tracks.get_mut(&state.track_id).unwrap();
                                let clip = track
                                    .midi_clips
                                    .iter_mut()
                                    .find(|c| {
                                        start_beat >= c.start_beat
                                            && start_beat < (c.start_beat + c.length_beats)
                                    })
                                    .unwrap();

                                let note = crate::model::MidiNote {
                                    id: new_note_id,
                                    pitch: data1,
                                    velocity,
                                    start: start_beat - clip.start_beat,
                                    duration,
                                };
                                clip.notes.push(note);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        AudioCommand::SetTrackMidiInput(track_id, port_name) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                track.midi_input_port = port_name.clone();

                // Logic to ensure only one track is connected at a time
                if let Some(p_name) = port_name {
                    if let Some(handler) = midi_input_handler {
                        if let Err(e) = handler.connect(p_name) {
                            log::error!("Failed to connect to MIDI port {}: {}", p_name, e);
                        }
                    }
                    // Disconnect other tracks
                    for (id, t) in state.tracks.iter_mut() {
                        if *id != *track_id {
                            t.midi_input_port = None;
                        }
                    }
                } else {
                    // Disconnect if "None" was selected
                    if let Some(handler) = midi_input_handler {
                        handler.disconnect();
                    }
                }
            }
        }

        // Plugin commands
        AudioCommand::RemovePlugin(track_id, plugin_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                if let Some(idx) = track.plugin_chain.iter().position(|p| p.id == *plugin_id) {
                    track.plugin_chain.remove(idx);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            drop(state);

            let _ = realtime_tx.send(RealtimeCommand::RemovePluginInstance {
                track_id: *track_id,
                plugin_id: *plugin_id,
            });

            send_graph_snapshot_locked(&app_state.lock().unwrap(), snapshot_tx);
        }

        AudioCommand::SetPluginBypass(track_id, plugin_id, bypass) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                if let Some(plugin) = track.plugin_chain.iter_mut().find(|p| p.id == *plugin_id) {
                    plugin.bypass = *bypass;
                }
            }
            drop(state);

            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginBypass(
                *track_id, *plugin_id, *bypass,
            ));
        }

        AudioCommand::SetPluginParam(track_id, plugin_id, param_name, value) => {
            let (uri, min_v, max_v) = {
                let state = app_state.lock().unwrap();
                if let Some(plugin) = state
                    .tracks
                    .get(track_id)
                    .and_then(|t| t.plugin_chain.iter().find(|p| p.id == *plugin_id))
                {
                    let (min, max) = get_control_port_info(&plugin.uri, param_name)
                        .map(|m| (m.min, m.max))
                        .unwrap_or((0.0, 1.0));
                    (Some(plugin.uri.clone()), min, max)
                } else {
                    (None, 0.0, 1.0)
                }
            };

            if uri.is_some() {
                let v = value.clamp(min_v, max_v);

                let mut state = app_state.lock().unwrap();
                if let Some(track) = state.tracks.get_mut(track_id) {
                    if let Some(plugin) = track.plugin_chain.iter_mut().find(|p| p.id == *plugin_id)
                    {
                        plugin.params.insert(param_name.clone(), v);
                    }
                }
                drop(state);

                let _ = realtime_tx.send(RealtimeCommand::UpdatePluginParam(
                    *track_id,
                    *plugin_id,
                    param_name.clone(),
                    v,
                ));
            }
        }

        AudioCommand::MovePlugin(track_id, from_idx, to_idx) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                if *from_idx < track.plugin_chain.len() && *to_idx < track.plugin_chain.len() {
                    let plugin = track.plugin_chain.remove(*from_idx);
                    let insert_pos = if from_idx < to_idx {
                        to_idx - 1
                    } else {
                        *to_idx
                    };
                    track.plugin_chain.insert(insert_pos, plugin);
                }
            }
            drop(state);

            send_graph_snapshot_locked(&app_state.lock().unwrap(), snapshot_tx);
        }

        AudioCommand::LoadPluginPreset(_, _, _) | AudioCommand::SavePluginPreset(_, _, _) => {}

        AudioCommand::SetLoopEnabled(enabled) => {
            audio_state.loop_enabled.store(*enabled, Ordering::Relaxed);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopEnabled(*enabled));
        }
        AudioCommand::SetLoopRegion(start, end) => {
            audio_state.loop_start.store(*start);
            audio_state.loop_end.store(*end);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopRegion(*start, *end));
        }

        AudioCommand::AddPluginUnified {
            track_id,
            plugin_idx,
            backend,
            uri,
            display_name,
        } => {
            let plugin_id_opt = {
                let mut state = app_state.lock().unwrap();
                let plugin_id = state.fresh_id();

                let mut desc = create_plugin_instance(uri, audio_state.sample_rate.load())
                    .unwrap_or_else(|_| PluginDescriptor {
                        id: 0,
                        uri: uri.clone(),
                        name: display_name.clone(),
                        backend: *backend,
                        bypass: false,
                        params: std::collections::HashMap::new(),
                        preset_name: None,
                        custom_name: None,
                    });
                desc.backend = *backend;
                desc.id = plugin_id;
                desc.name = display_name.clone();

                let inserted = if let Some(track) = state.tracks.get_mut(track_id) {
                    let insert_at = (*plugin_idx).min(track.plugin_chain.len());
                    track.plugin_chain.insert(insert_at, desc);
                    true
                } else {
                    false
                };

                if inserted {
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                    Some(plugin_id)
                } else {
                    None
                }
            };

            if let Some(plugin_id) = plugin_id_opt {
                let _ = realtime_tx.send(RealtimeCommand::AddUnifiedPlugin {
                    track_id: *track_id,
                    plugin_id,
                    backend: *backend,
                    uri: uri.clone(),
                });

                let state = app_state.lock().unwrap();
                send_graph_snapshot_locked(&state, snapshot_tx);
            }
        }

        // MIDI Clip commands using clip IDs
        AudioCommand::CreateMidiClip {
            track_id,
            start_beat,
            length_beats,
        } => {
            let new_clip_id = {
                let mut state = app_state.lock().unwrap();
                state.fresh_id()
            };

            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
                let clip = crate::model::clip::MidiClip {
                    id: new_clip_id,
                    name: format!("MIDI Clip {}", track.midi_clips.len() + 1),
                    start_beat: *start_beat,
                    length_beats: *length_beats,
                    notes: Vec::new(),
                    color: Some((100, 150, 200)),
                    ..Default::default()
                };
                track.midi_clips.push(clip);

                // Update clip index
                state.clips_by_id.insert(
                    new_clip_id,
                    crate::project::ClipRef {
                        track_id: *track_id,
                        is_midi: true,
                    },
                );

                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::DeleteMidiClip { clip_id } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    track.midi_clips.remove(idx);
                    state.clips_by_id.remove(clip_id);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::MoveMidiClip { clip_id, new_start } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.start_beat = *new_start;
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::ResizeMidiClip {
            clip_id,
            new_start,
            new_length,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.start_beat = *new_start;
                        clip.length_beats = (*new_length).max(0.0);
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::DuplicateMidiClip { clip_id } => {
            let source = {
                let state = app_state.lock().unwrap();
                state.find_clip(*clip_id).and_then(|(track, loc)| {
                    if let crate::project::ClipLocation::Midi(idx) = loc {
                        track.midi_clips.get(idx).cloned()
                    } else {
                        None
                    }
                })
            };

            if let Some(mut clip) = source {
                let mut state = app_state.lock().unwrap();
                let new_clip_id = state.fresh_id();
                clip.id = new_clip_id;
                for n in &mut clip.notes {
                    n.id = state.fresh_id();
                }
                clip.name = format!("{} (copy)", clip.name);
                clip.start_beat += clip.length_beats;

                if let Some(clip_ref) = state.clips_by_id.get(clip_id) {
                    let track_id = clip_ref.track_id;
                    if let Some(track) = state.tracks.get_mut(&track_id) {
                        track.midi_clips.push(clip);
                        state.clips_by_id.insert(
                            new_clip_id,
                            crate::project::ClipRef {
                                track_id,
                                is_midi: true,
                            },
                        );
                    }
                }
                send_graph_snapshot_locked(&state, snapshot_tx);
            }
        }

        // ---------- AUDIO CLIPS ----------
        AudioCommand::MoveAudioClip { clip_id, new_start } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        clip.start_beat = *new_start;
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::ResizeAudioClip {
            clip_id,
            new_start,
            new_length,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        let old_start = clip.start_beat;
                        let delta_beats = *new_start - old_start;

                        clip.offset_beats = (clip.offset_beats + delta_beats).max(0.0);
                        clip.start_beat = *new_start;
                        clip.length_beats = new_length.max(0.0);
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::DuplicateAudioClip { clip_id } => {
            // Clone source without holding &mut borrow
            let source = {
                let state = app_state.lock().unwrap();
                state.find_clip(*clip_id).and_then(|(track, loc)| {
                    if let crate::project::ClipLocation::Audio(idx) = loc {
                        track.audio_clips.get(idx).cloned()
                    } else {
                        None
                    }
                })
            };

            if let Some(mut clip) = source {
                let mut state = app_state.lock().unwrap();
                let new_clip_id = state.fresh_id();
                clip.id = new_clip_id;
                clip.name = format!("{} (copy)", clip.name);
                clip.start_beat += clip.length_beats;

                // Find the track again and insert
                if let Some(clip_ref) = state.clips_by_id.get(clip_id) {
                    let track_id = clip_ref.track_id;
                    if let Some(track) = state.tracks.get_mut(&track_id) {
                        track.audio_clips.push(clip.clone());
                        state.clips_by_id.insert(
                            new_clip_id,
                            crate::project::ClipRef {
                                track_id,
                                is_midi: false,
                            },
                        );
                    }
                }
            }
            send_graph_snapshot_locked(&app_state.lock().unwrap(), snapshot_tx);
        }

        AudioCommand::DeleteAudioClip { clip_id } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    track.audio_clips.remove(idx);
                    state.clips_by_id.remove(clip_id);
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        // ---------- AUTOMATION ----------
        AudioCommand::AddAutomationPoint(track_id, target, beat, value) => {
            use crate::model::automation::{AutomationLane, AutomationMode, AutomationPoint};
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id) {
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
            send_graph_snapshot_locked(&state, snapshot_tx);
        }
        AudioCommand::RemoveAutomationPoint(track_id, lane_idx, beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id)
                && let Some(lane) = track.automation_lanes.get_mut(*lane_idx)
            {
                lane.points.retain(|p| (p.beat - beat).abs() > 0.001);
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }
        AudioCommand::UpdateAutomationPoint {
            track_id,
            lane_idx,
            old_beat,
            new_beat,
            new_value,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(track_id)
                && let Some(lane) = track.automation_lanes.get_mut(*lane_idx)
            {
                // Remove old point
                lane.points.retain(|p| (p.beat - old_beat).abs() > 0.001);

                // Add new point
                lane.points.push(AutomationPoint {
                    beat: *new_beat,
                    value: *new_value,
                });
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }
        AudioCommand::SetAutomationMode(_, _, _) | AudioCommand::ClearAutomationLane(_, _) => {}

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
            if let Some(track) = state.tracks.get_mut(track_id) {
                track.monitor_enabled = *enabled;
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::AddSend(_, _, _)
        | AudioCommand::RemoveSend(_, _)
        | AudioCommand::SetSendAmount(_, _, _)
        | AudioCommand::SetSendPreFader(_, _, _)
        | AudioCommand::CreateGroup(_, _)
        | AudioCommand::RemoveGroup(_)
        | AudioCommand::AddTrackToGroup(_, _)
        | AudioCommand::RemoveTrackFromGroup(_) => {}

        AudioCommand::ToggleClipLoop { clip_id, enabled } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.loop_enabled = *enabled;
                        if clip.content_len_beats <= 0.0 {
                            clip.content_len_beats = clip.length_beats.max(0.000001);
                        }
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::MakeClipAlias { clip_id } => {
            let needs_pid = {
                let state = app_state.lock().unwrap();
                state
                    .find_clip(*clip_id)
                    .map(|(track, loc)| {
                        if let crate::project::ClipLocation::Midi(idx) = loc {
                            track
                                .midi_clips
                                .get(idx)
                                .map(|c| c.pattern_id.is_none())
                                .unwrap_or(false)
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
            };

            if !needs_pid {
                return;
            }

            // Assign a new pattern_id in a separate mutable scope
            let mut state = app_state.lock().unwrap();
            let new_id = state.fresh_id(); // safe (no field borrows yet)
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        // Double-check in case of races
                        if clip.pattern_id.is_none() {
                            clip.pattern_id = Some(new_id);
                        }
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::MakeClipUnique { clip_id } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.pattern_id = None;
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::SetClipQuantize {
            clip_id,
            grid,
            strength,
            swing,
            enabled,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.quantize_grid = *grid;
                        clip.quantize_strength = strength.clamp(0.0, 1.0);
                        clip.swing = *swing;
                        clip.quantize_enabled = *enabled;
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::DuplicateMidiClipAsAlias { clip_id } => {
            let mut state = app_state.lock().unwrap();

            // 1) Snapshot undo ONCE at start
            let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));

            // 2) Get source clip and its track ID
            let (src_clip, src_pid, track_id) = {
                let (track, loc) = match state.find_clip(*clip_id) {
                    Some(t) => t,
                    None => return,
                };
                let clip = match loc {
                    crate::project::ClipLocation::Midi(idx) => track.midi_clips.get(idx),
                    _ => None,
                };
                (
                    clip.cloned(),
                    clip.and_then(|c| c.pattern_id),
                    Some(track.id),
                )
            };

            let src_clip = match src_clip {
                Some(c) => c,
                None => return,
            };
            let track_id = track_id.unwrap();

            // 3) Assign pattern_id to source if needed
            let final_pid = if src_pid.is_none() {
                let new_pid = state.fresh_id();
                // Safe: we still hold state lock
                if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                    if let crate::project::ClipLocation::Midi(idx) = loc {
                        if let Some(clip) = track.midi_clips.get_mut(idx) {
                            clip.pattern_id = Some(new_pid);
                        }
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
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.midi_clips.push(dup.clone());
                state.clips_by_id.insert(
                    dup.id,
                    crate::project::ClipRef {
                        track_id,
                        is_midi: true,
                    },
                );
            }

            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::SetClipContentOffset {
            clip_id,
            new_offset,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        let len = clip.content_len_beats.max(0.000001);
                        // Wrap offset into [0, len)
                        clip.content_offset_beats = ((*new_offset % len) + len) % len;
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }
        AudioCommand::CutSelectedNotes { clip_id, note_ids } => {
            let mut clipboard_notes = Vec::new();
            let mut state = app_state.lock().unwrap();

            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        // Partition notes into 'kept' and 'cut'
                        let original_notes = std::mem::take(&mut clip.notes);
                        let (kept_notes, cut_notes): (Vec<_>, Vec<_>) = original_notes
                            .into_iter()
                            .partition(|n| !note_ids.contains(&n.id));

                        clip.notes = kept_notes;
                        clipboard_notes = cut_notes;
                    }
                }
            }

            // Send the cut notes back to the UI thread for its clipboard
            if !clipboard_notes.is_empty() {
                let _ = ui_tx.send(UIUpdate::NotesCutToClipboard(clipboard_notes));
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::DeleteSelectedNotes { clip_id, note_ids } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.notes.retain(|n| !note_ids.contains(&n.id));
                    }
                }
            }
            send_graph_snapshot_locked(&state, snapshot_tx);
        }

        AudioCommand::PasteNotes { clip_id, notes } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(*clip_id) {
                if let crate::project::ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.notes.extend_from_slice(notes);
                        clip.notes
                            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                    }
                }
            }
            state.ensure_ids(); // Assign IDs to newly pasted notes
            send_graph_snapshot_locked(&state, snapshot_tx);
        }
        AudioCommand::ExportAudio(config) => {
            let app_state_clone = app_state.lock().unwrap().clone();
            let audio_state_clone = audio_state.clone();
            let ui_tx_clone = ui_tx.clone();

            let _ = AudioExporter::export_to_wav(
                app_state_clone,
                audio_state_clone,
                config.clone(),
                ui_tx_clone,
            );
        }
        AudioCommand::RebuildAllRtChains => {
            let state = app_state.lock().unwrap();
            let track_snapshots = crate::audio_snapshot::build_track_snapshots(&state);
            drop(state);

            for ts in track_snapshots {
                let _ = realtime_tx.send(RealtimeCommand::RebuildTrackChain {
                    track_id: ts.track_id,
                    chain: ts.plugin_chain,
                });
            }
        }
        _ => {
            // Stub for unhandled commands
        }
    }
}

fn send_graph_snapshot_locked(state: &AppState, snapshot_tx: &Sender<AudioGraphSnapshot>) {
    if snapshot_tx.is_full() {
        log::trace!("Skipping snapshot send, audio thread is busy.");
        return;
    }

    let snapshot = AudioGraphSnapshot {
        tracks: crate::audio_snapshot::build_track_snapshots(state),
        track_order: state.track_order.clone(),
    };

    if let Err(e) = snapshot_tx.try_send(snapshot) {
        if e.is_full() {
            // This is okay, another thread sent a snapshot just before we did.
        } else {
            log::error!("Failed to send audio graph snapshot: audio thread may have crashed.");
        }
    }
}

/// State for an in-progress MIDI recording.
struct MidiRecordingState {
    /// The track ID we are recording to.
    track_id: u64,
    active_notes: HashMap<(u8, u8), (f64, u8)>,
}
