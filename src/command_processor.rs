use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam_channel::{Receiver, Sender};

use crate::audio_export::AudioExporter;
use crate::audio_state::{AudioGraphSnapshot, AudioState, MidiNoteSnapshot, RealtimeCommand};
use crate::edit_actions::EditProcessor;
use crate::idgen;
use crate::messages::{AudioCommand, UIUpdate};
use crate::midi_input::MidiInputHandler;
use crate::model::clip::MidiPattern;
use crate::model::track::TrackType;
use crate::model::{AutomationPoint, MidiClip, MidiNote, PluginDescriptor};
use crate::plugin::{create_plugin_instance, get_control_port_info};
use crate::project::{AppState, ClipLocation, ClipRef};
use crate::time_utils::quick::samples_to_beats;

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
            command, // pass by value so we can move owned fields
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
    command: AudioCommand, // by value
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
                send_graph_snapshot(&app_state.lock().unwrap(), snapshot_tx);
            }
        }
        AudioCommand::Pause => {
            audio_state.playing.store(false, Ordering::Relaxed);
        }
        AudioCommand::SetPosition(position) => {
            audio_state.set_position(position);
        }
        AudioCommand::SetBPM(bpm) => {
            audio_state.bpm.store(bpm);
            let mut state = app_state.lock().unwrap();
            state.bpm = bpm;
        }
        AudioCommand::SetMasterVolume(volume) => {
            audio_state.master_volume.store(volume);
        }
        AudioCommand::UpdateTracks => {
            send_graph_snapshot(&app_state.lock().unwrap(), snapshot_tx);
        }
        AudioCommand::SetTrackVolume(track_id, volume) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.volume = volume;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackVolume(track_id, volume));
        }
        AudioCommand::SetTrackPan(track_id, pan) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.pan = pan;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackPan(track_id, pan));
        }
        AudioCommand::SetTrackMute(track_id, mute) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.muted = mute;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackMute(track_id, mute));
        }
        AudioCommand::SetTrackSolo(track_id, solo) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.solo = solo;
            }
            let _ = realtime_tx.send(RealtimeCommand::UpdateTrackSolo(track_id, solo));
        }
        AudioCommand::ArmForRecording(track_id, armed) => {
            let mut state = app_state.lock().unwrap();

            let target_is_midi = state
                .tracks
                .get(&track_id)
                .map_or(false, |t| matches!(t.track_type, TrackType::Midi));

            if armed {
                for (id, track) in state.tracks.iter_mut() {
                    if *id == track_id {
                        track.armed = true;
                    } else if matches!(track.track_type, TrackType::Midi) == target_is_midi {
                        track.armed = false;
                    }
                }
            } else {
                if let Some(track) = state.tracks.get_mut(&track_id) {
                    track.armed = false;
                }
            }

            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::FinalizeRecording => {
            log::info!("FinalizeRecording command received.");
        }
        AudioCommand::StartRecording => {
            audio_state.playing.store(true, Ordering::Relaxed);

            // Resolve armed MIDI track (read-only)
            let armed_midi_track_id = {
                let state = app_state.lock().unwrap();
                state
                    .tracks
                    .values()
                    .find(|t| matches!(t.track_type, TrackType::Midi) && t.armed)
                    .map(|t| t.id)
            };

            let sr = audio_state.sample_rate.load();
            let bpm = audio_state.bpm.load();
            let start_beat = samples_to_beats(audio_state.get_position(), sr, bpm);

            if let Some(track_id) = armed_midi_track_id {
                insert_recording_clip_if_missing(app_state, track_id, start_beat);
                *midi_recording_state = Some(MidiRecordingState {
                    track_id,
                    active_notes: HashMap::new(),
                });
            }

            audio_state.recording.store(true, Ordering::Relaxed);
        }
        AudioCommand::StopRecording => {
            audio_state.recording.store(false, Ordering::Relaxed);
            if midi_recording_state.is_some() {
                *midi_recording_state = None;
                send_graph_snapshot(&app_state.lock().unwrap(), snapshot_tx);
            }
        }
        AudioCommand::SetMetronome(on) => {
            audio_state.metronome_enabled.store(on, Ordering::Relaxed);
        }
        AudioCommand::SetSendDestination(track_id, index, dest_track_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(t) = state.tracks.get_mut(&track_id) {
                if index < t.sends.len() {
                    t.sends[index].destination_track = dest_track_id;
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::MidiInput(raw_message) => {
            if let Some(rec) = midi_recording_state {
                let status = raw_message.message[0];
                let data1 = raw_message.message[1];
                let data2 = raw_message.message[2];
                let channel = status & 0x0F;
                let message_type = status & 0xF0;

                let current_beat = {
                    let pos_samples = audio_state.get_position();
                    let sr = audio_state.sample_rate.load();
                    let bpm = audio_state.bpm.load();
                    samples_to_beats(pos_samples, sr, bpm)
                };

                match message_type {
                    0x90 if data2 > 0 => {
                        rec.active_notes
                            .insert((data1, channel), (current_beat, data2));
                    }
                    0x80 | 0x90 => {
                        if let Some((start_beat, velocity)) =
                            rec.active_notes.remove(&(data1, channel))
                        {
                            let duration = (current_beat - start_beat).max(0.01);

                            // 1) Find the target clip/pattern immutably
                            let (clip_idx, pid_opt, clip_start) = {
                                let st = app_state.lock().unwrap();
                                let Some(track) = st.tracks.get(&rec.track_id) else {
                                    return;
                                };
                                if let Some((idx, clip)) =
                                    track.midi_clips.iter().enumerate().find(|(_, c)| {
                                        start_beat >= c.start_beat
                                            && start_beat < c.start_beat + c.length_beats
                                    })
                                {
                                    (idx, clip.pattern_id, clip.start_beat)
                                } else {
                                    (usize::MAX, None, 0.0)
                                }
                            };
                            if clip_idx == usize::MAX {
                                return;
                            }

                            // 2) Prepare the note with a global ID
                            let note = crate::model::MidiNote {
                                id: idgen::next(),
                                pitch: data1,
                                velocity,
                                start: (start_beat - clip_start).max(0.0),
                                duration,
                            };

                            // 3) Insert into pattern in a short mutable scope
                            if let Some(pid) = pid_opt {
                                let mut st = app_state.lock().unwrap();
                                if let Some(p) = st.patterns.get_mut(&pid) {
                                    p.notes.push(note);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        AudioCommand::SetTrackMidiInput(track_id, port_name) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.midi_input_port = port_name.clone();

                if let Some(p_name) = port_name {
                    if let Some(handler) = midi_input_handler {
                        if let Err(e) = handler.connect(&p_name) {
                            log::error!("Failed to connect to MIDI port {}: {}", p_name, e);
                        }
                    }
                    for (id, t) in state.tracks.iter_mut() {
                        if *id != track_id {
                            t.midi_input_port = None;
                        }
                    }
                } else {
                    if let Some(handler) = midi_input_handler {
                        handler.disconnect();
                    }
                }
            }
        }
        AudioCommand::RemovePlugin(track_id, plugin_id) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                if let Some(idx) = track.plugin_chain.iter().position(|p| p.id == plugin_id) {
                    track.plugin_chain.remove(idx);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            drop(state);

            let _ = realtime_tx.send(RealtimeCommand::RemovePluginInstance {
                track_id,
                plugin_id,
            });

            send_graph_snapshot(&app_state.lock().unwrap(), snapshot_tx);
        }
        AudioCommand::SetPluginBypass(track_id, plugin_id, bypass) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                if let Some(plugin) = track.plugin_chain.iter_mut().find(|p| p.id == plugin_id) {
                    plugin.bypass = bypass;
                }
            }
            drop(state);

            let _ = realtime_tx.send(RealtimeCommand::UpdatePluginBypass(
                track_id, plugin_id, bypass,
            ));
        }
        AudioCommand::SetPluginParam(track_id, plugin_id, param_name, value) => {
            let (uri, min_v, max_v) = {
                let state = app_state.lock().unwrap();
                if let Some(plugin) = state
                    .tracks
                    .get(&track_id)
                    .and_then(|t| t.plugin_chain.iter().find(|p| p.id == plugin_id))
                {
                    let (min, max) = get_control_port_info(&plugin.uri, &param_name)
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
                if let Some(track) = state.tracks.get_mut(&track_id) {
                    if let Some(plugin) = track.plugin_chain.iter_mut().find(|p| p.id == plugin_id)
                    {
                        plugin.params.insert(param_name.clone(), v);
                    }
                }
                drop(state);

                let _ = realtime_tx.send(RealtimeCommand::UpdatePluginParam(
                    track_id,
                    plugin_id,
                    param_name.clone(),
                    v,
                ));
            }
        }
        AudioCommand::MovePlugin(track_id, from_idx, to_idx) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                if from_idx < track.plugin_chain.len() && to_idx < track.plugin_chain.len() {
                    let plugin = track.plugin_chain.remove(from_idx);
                    let insert_pos = if from_idx < to_idx {
                        to_idx - 1
                    } else {
                        to_idx
                    };
                    track.plugin_chain.insert(insert_pos, plugin);
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::SavePluginPreset(track_id, plugin_idx, name) => {
            use crate::presets::{PluginPreset, save_preset};
            let state = app_state.lock().unwrap();

            let (uri, backend, params_map) = if let Some(track) = state.tracks.get(&track_id) {
                if plugin_idx < track.plugin_chain.len() {
                    let desc = &track.plugin_chain[plugin_idx];
                    (desc.uri.clone(), desc.backend, desc.params.clone())
                } else {
                    drop(state);
                    let _ = ui_tx.send(UIUpdate::Warning(format!(
                        "Invalid plugin index {} on track {}",
                        plugin_idx, track_id
                    )));
                    return;
                }
            } else {
                drop(state);
                let _ = ui_tx.send(UIUpdate::Warning(format!("Track {} not found", track_id)));
                return;
            };

            let preset = PluginPreset {
                uri: uri.clone(),
                backend,
                name: name.clone(),
                params: params_map,
            };

            match save_preset(&preset) {
                Ok(_) => {
                    let _ = ui_tx.send(UIUpdate::Info(format!(
                        "Saved preset '{}' for {}",
                        name, uri
                    )));
                }
                Err(e) => {
                    let _ = ui_tx.send(UIUpdate::Error(format!(
                        "Failed to save preset '{}': {}",
                        name, e
                    )));
                }
            }
        }
        AudioCommand::LoadPluginPreset(track_id, plugin_idx, name) => {
            use crate::presets::load_preset;

            {
                let snapshot = app_state.lock().unwrap().snapshot();
                let _ = ui_tx.send(UIUpdate::PushUndo(snapshot));
            }

            let (uri, plugin_id, params_to_update) = {
                let mut state = app_state.lock().unwrap();
                let (uri, plugin_id) = if let Some(track) = state.tracks.get_mut(&track_id) {
                    if plugin_idx < track.plugin_chain.len() {
                        let desc = &track.plugin_chain[plugin_idx];
                        (desc.uri.clone(), desc.id)
                    } else {
                        let _ = ui_tx.send(UIUpdate::Warning(format!(
                            "Invalid plugin index {} on track {}",
                            plugin_idx, track_id
                        )));
                        return;
                    }
                } else {
                    let _ = ui_tx.send(UIUpdate::Warning(format!("Track {} not found", track_id)));
                    return;
                };

                let preset = match load_preset(&uri, &name) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ui_tx.send(UIUpdate::Error(format!(
                            "Failed to load preset '{}': {}",
                            name, e
                        )));
                        return;
                    }
                };

                if let Some(track) = state.tracks.get_mut(&track_id) {
                    if let Some(desc) = track.plugin_chain.get_mut(plugin_idx) {
                        for (k, v) in &preset.params {
                            desc.params.insert(k.clone(), *v);
                        }
                        desc.preset_name = Some(name.clone());
                    }
                }

                let params_to_update = preset.params.clone();
                (uri, plugin_id, params_to_update)
            };

            for (param_name, value) in params_to_update {
                let _ = realtime_tx.send(RealtimeCommand::UpdatePluginParam(
                    track_id, plugin_id, param_name, value,
                ));
            }

            send_graph_snapshot(&app_state.lock().unwrap(), snapshot_tx);
        }
        AudioCommand::SetLoopEnabled(enabled) => {
            audio_state.loop_enabled.store(enabled, Ordering::Relaxed);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopEnabled(enabled));
        }
        AudioCommand::SetLoopRegion(start, end) => {
            audio_state.loop_start.store(start);
            audio_state.loop_end.store(end);
            let _ = realtime_tx.send(RealtimeCommand::SetLoopRegion(start, end));
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
                let plugin_id = idgen::next();

                let mut desc = create_plugin_instance(&uri, audio_state.sample_rate.load())
                    .unwrap_or_else(|_| PluginDescriptor {
                        id: 0,
                        uri: uri.clone(),
                        name: display_name.clone(),
                        backend,
                        bypass: false,
                        params: std::collections::HashMap::new(),
                        preset_name: None,
                        custom_name: None,
                    });
                desc.backend = backend;
                desc.id = plugin_id;
                desc.name = display_name.clone();

                let inserted = if let Some(track) = state.tracks.get_mut(&track_id) {
                    let insert_at = (plugin_idx).min(track.plugin_chain.len());
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
                    track_id,
                    plugin_id,
                    backend,
                    uri: uri.clone(),
                });

                let state = app_state.lock().unwrap();
                send_graph_snapshot(&state, snapshot_tx);
            }
        }
        AudioCommand::CreateMidiClip {
            track_id,
            start_beat,
            length_beats,
        } => {
            let mut state = app_state.lock().unwrap();
            let new_clip_id = idgen::next();
            let new_pid = idgen::next();
            state.patterns.insert(
                new_pid,
                MidiPattern {
                    id: new_pid,
                    notes: Vec::new(),
                },
            );
            if let Some(track) = state.tracks.get_mut(&track_id) {
                let clip = MidiClip {
                    id: new_clip_id,
                    name: format!("MIDI Clip {}", track.midi_clips.len() + 1),
                    start_beat,
                    length_beats,
                    notes: Vec::new(),
                    color: Some((100, 150, 200)),
                    pattern_id: Some(new_pid),
                    ..Default::default()
                };
                track.midi_clips.push(clip);

                state.clips_by_id.insert(
                    new_clip_id,
                    ClipRef {
                        track_id,
                        is_midi: true,
                    },
                );

                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::DeleteMidiClip { clip_id } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    track.midi_clips.remove(idx);
                    state.clips_by_id.remove(&clip_id);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::MoveMidiClip { clip_id, new_start } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.start_beat = new_start;
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::ResizeMidiClip {
            clip_id,
            new_start,
            new_length,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.start_beat = new_start;
                        clip.length_beats = (new_length).max(0.0);
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::DuplicateMidiClip { clip_id } => {
            let source = {
                let state = app_state.lock().unwrap();
                state.find_clip(clip_id).and_then(|(track, loc)| {
                    if let ClipLocation::Midi(idx) = loc {
                        track.midi_clips.get(idx).cloned()
                    } else {
                        None
                    }
                })
            };

            if let Some(mut clip) = source {
                let mut state = app_state.lock().unwrap();
                let new_clip_id = idgen::next();
                let new_pid = idgen::next();
                let base_notes = if let Some(pid) = clip.pattern_id {
                    state
                        .patterns
                        .get(&pid)
                        .map(|p| p.notes.clone())
                        .unwrap_or_else(|| clip.notes.clone())
                } else {
                    clip.notes.clone()
                };
                state.patterns.insert(
                    new_pid,
                    MidiPattern {
                        id: new_pid,
                        notes: base_notes,
                    },
                );
                clip.id = new_clip_id;
                for n in &mut clip.notes {
                    n.id = idgen::next();
                }
                clip.name = format!("{} (copy)", clip.name);
                clip.start_beat += clip.length_beats;
                clip.pattern_id = Some(new_pid);

                if let Some(clip_ref) = state.clips_by_id.get(&clip_id) {
                    let track_id = clip_ref.track_id;
                    if let Some(track) = state.tracks.get_mut(&track_id) {
                        track.midi_clips.push(clip);
                        state.clips_by_id.insert(
                            new_clip_id,
                            ClipRef {
                                track_id,
                                is_midi: true,
                            },
                        );
                    }
                }
                send_graph_snapshot(&state, snapshot_tx);
            }
        }
        AudioCommand::MoveAudioClip { clip_id, new_start } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        clip.start_beat = new_start;
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::ResizeAudioClip {
            clip_id,
            new_start,
            new_length,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        let old_start = clip.start_beat;
                        let delta_beats = new_start - old_start;

                        clip.offset_beats = (clip.offset_beats + delta_beats).max(0.0);
                        clip.start_beat = new_start;
                        clip.length_beats = new_length.max(0.0);
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::DuplicateAudioClip { clip_id } => {
            let source = {
                let state = app_state.lock().unwrap();
                state.find_clip(clip_id).and_then(|(track, loc)| {
                    if let ClipLocation::Audio(idx) = loc {
                        track.audio_clips.get(idx).cloned()
                    } else {
                        None
                    }
                })
            };

            if let Some(mut clip) = source {
                let mut state = app_state.lock().unwrap();
                let new_clip_id = idgen::next();
                clip.id = new_clip_id;
                clip.name = format!("{} (copy)", clip.name);
                clip.start_beat += clip.length_beats;

                if let Some(clip_ref) = state.clips_by_id.get(&clip_id) {
                    let track_id = clip_ref.track_id;
                    if let Some(track) = state.tracks.get_mut(&track_id) {
                        track.audio_clips.push(clip.clone());
                        state.clips_by_id.insert(
                            new_clip_id,
                            ClipRef {
                                track_id,
                                is_midi: false,
                            },
                        );
                    }
                }
            }
            send_graph_snapshot(&app_state.lock().unwrap(), snapshot_tx);
        }
        AudioCommand::DeleteAudioClip { clip_id } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    track.audio_clips.remove(idx);
                    state.clips_by_id.remove(&clip_id);
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::AddAutomationPoint(track_id, target, beat, value) => {
            use crate::model::automation::{AutomationLane, AutomationMode, AutomationPoint};
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                let lane_idx = if let Some(idx) = track
                    .automation_lanes
                    .iter()
                    .position(|l| l.parameter == target)
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
                    lane.points.push(AutomationPoint { beat, value });
                    lane.points
                        .sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
                }
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::RemoveAutomationPoint(track_id, lane_idx, beat) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id)
                && let Some(lane) = track.automation_lanes.get_mut(lane_idx)
            {
                lane.points.retain(|p| (p.beat - beat).abs() > 0.001);
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::UpdateAutomationPoint {
            track_id,
            lane_idx,
            old_beat,
            new_beat,
            new_value,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id)
                && let Some(lane) = track.automation_lanes.get_mut(lane_idx)
            {
                lane.points.retain(|p| (p.beat - old_beat).abs() > 0.001);
                lane.points.push(AutomationPoint {
                    beat: new_beat,
                    value: new_value,
                });
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::PreviewNote(track_id, pitch) => {
            let current_position = audio_state.get_position();
            let _ = realtime_tx.send(RealtimeCommand::PreviewNote(
                track_id,
                pitch,
                current_position,
            ));
        }
        AudioCommand::StopPreviewNote => {
            let _ = realtime_tx.send(RealtimeCommand::StopPreviewNote);
        }
        AudioCommand::SetTrackMonitor(track_id, enabled) => {
            let mut state = app_state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.monitor_enabled = enabled;
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::AddSend(track_id, dest_track_id, amount) => {
            let mut state = app_state.lock().unwrap();
            if let Some(t) = state.tracks.get_mut(&track_id) {
                t.sends.push(crate::model::track::Send {
                    destination_track: dest_track_id,
                    amount,
                    pre_fader: false,
                    muted: false,
                });
                let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::RemoveSend(track_id, index) => {
            let mut state = app_state.lock().unwrap();
            if let Some(t) = state.tracks.get_mut(&track_id) {
                if index < t.sends.len() {
                    t.sends.remove(index);
                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::SetSendAmount(track_id, index, value) => {
            let mut state = app_state.lock().unwrap();
            if let Some(t) = state.tracks.get_mut(&track_id) {
                if index < t.sends.len() {
                    t.sends[index].amount = value;
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::SetSendPreFader(track_id, index, pref) => {
            let mut state = app_state.lock().unwrap();
            if let Some(t) = state.tracks.get_mut(&track_id) {
                if index < t.sends.len() {
                    t.sends[index].pre_fader = pref;
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::DuplicateAndMoveMidiClip {
            clip_id,
            dest_track_id,
            new_start,
        } => {
            let mut state = app_state.lock().unwrap();

            if let Some((src_track, loc)) = state.find_clip(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    let original = src_track.midi_clips[idx].clone();

                    let mut new_clip = original.clone();
                    new_clip.id = idgen::next();
                    new_clip.start_beat = new_start;

                    let base_notes = if let Some(pid) = original.pattern_id {
                        state
                            .patterns
                            .get(&pid)
                            .map(|p| p.notes.clone())
                            .unwrap_or_else(|| original.notes.clone())
                    } else {
                        original.notes.clone()
                    };
                    let new_pid = idgen::next();
                    state.patterns.insert(
                        new_pid,
                        MidiPattern {
                            id: new_pid,
                            notes: base_notes,
                        },
                    );
                    new_clip.pattern_id = Some(new_pid);

                    if let Some(dest_track) = state.tracks.get_mut(&dest_track_id) {
                        dest_track.midi_clips.push(new_clip.clone());
                        state.clips_by_id.insert(
                            new_clip.id,
                            ClipRef {
                                track_id: dest_track_id,
                                is_midi: true,
                            },
                        );
                    }

                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::DuplicateAndMoveAudioClip {
            clip_id,
            dest_track_id,
            new_start,
        } => {
            let mut state = app_state.lock().unwrap();

            if let Some((src_track, loc)) = state.find_clip(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    let original = src_track.audio_clips[idx].clone();

                    let mut new_clip = original.clone();
                    new_clip.id = idgen::next();
                    new_clip.start_beat = new_start;

                    if let Some(dest_track) = state.tracks.get_mut(&dest_track_id) {
                        dest_track.audio_clips.push(new_clip.clone());
                        state.clips_by_id.insert(
                            new_clip.id,
                            ClipRef {
                                track_id: dest_track_id,
                                is_midi: false,
                            },
                        );
                    }

                    let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::CreateGroup(_name, track_ids) => {
            // Assign a compact new group id (max+1).
            let mut st = app_state.lock().unwrap();
            let next_gid = st
                .tracks
                .values()
                .filter_map(|t| t.group_id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            for tid in track_ids {
                if let Some(t) = st.tracks.get_mut(&tid) {
                    t.group_id = Some(next_gid);
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::RemoveGroup(group_id) => {
            let mut st = app_state.lock().unwrap();
            for t in st.tracks.values_mut() {
                if t.group_id == Some(group_id) {
                    t.group_id = None;
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::AddTrackToGroup(track_id, group_id) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.group_id = Some(group_id);
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::RemoveTrackFromGroup(track_id) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.group_id = None;
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::ToggleClipLoop { clip_id, enabled } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.loop_enabled = enabled;
                        if clip.content_len_beats <= 0.0 {
                            clip.content_len_beats = clip.length_beats.max(0.000001);
                        }
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::MakeClipAlias { clip_id } => {
            let needs_pid = {
                let state = app_state.lock().unwrap();
                state
                    .find_clip(clip_id)
                    .map(|(track, loc)| {
                        if let ClipLocation::Midi(idx) = loc {
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

            let mut state = app_state.lock().unwrap();
            let new_id = idgen::next();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        if clip.pattern_id.is_none() {
                            clip.pattern_id = Some(new_id);
                        }
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::MakeClipUnique { clip_id } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.pattern_id = None;
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::SetClipQuantize {
            clip_id,
            grid,
            strength,
            swing,
            enabled,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        clip.quantize_grid = grid;
                        clip.quantize_strength = strength.clamp(0.0, 1.0);
                        clip.swing = swing;
                        clip.quantize_enabled = enabled;
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::DuplicateMidiClipAsAlias { clip_id } => {
            let mut state = app_state.lock().unwrap();

            let _ = ui_tx.send(UIUpdate::PushUndo(state.snapshot()));

            let (src_clip, src_pid, track_id) = {
                let (track, loc) = match state.find_clip(clip_id) {
                    Some(t) => t,
                    None => return,
                };
                let clip = match loc {
                    ClipLocation::Midi(idx) => track.midi_clips.get(idx),
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

            let final_pid = match src_pid {
                Some(pid) => pid,
                None => {
                    let new_pid = idgen::next();
                    state.patterns.insert(
                        new_pid,
                        MidiPattern {
                            id: new_pid,
                            notes: src_clip.notes.clone(),
                        },
                    );
                    if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                        if let ClipLocation::Midi(idx) = loc {
                            if let Some(clip) = track.midi_clips.get_mut(idx) {
                                clip.pattern_id = Some(new_pid);
                            }
                        }
                    }
                    new_pid
                }
            };

            let mut dup = src_clip;
            dup.id = idgen::next();
            dup.start_beat += dup.length_beats;
            dup.pattern_id = Some(final_pid);
            dup.name = format!("{} (alias)", dup.name);

            for n in &mut dup.notes {
                if n.id == 0 {
                    n.id = idgen::next();
                }
            }

            if let Some(track) = state.tracks.get_mut(&track_id) {
                track.midi_clips.push(dup.clone());
                state.clips_by_id.insert(
                    dup.id,
                    ClipRef {
                        track_id,
                        is_midi: true,
                    },
                );
            }

            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::SetClipContentOffset {
            clip_id,
            new_offset,
        } => {
            let mut state = app_state.lock().unwrap();
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let ClipLocation::Midi(idx) = loc {
                    if let Some(clip) = track.midi_clips.get_mut(idx) {
                        let len = clip.content_len_beats.max(0.000001);
                        clip.content_offset_beats = ((new_offset % len) + len) % len;
                    }
                }
            }
            send_graph_snapshot(&state, snapshot_tx);
        }
        AudioCommand::CutSelectedNotes { clip_id, note_ids } => {
            let cut = with_pattern_mut(app_state, clip_id, |pat, _len| {
                let orig = std::mem::take(&mut pat.notes);
                let (kept, removed): (Vec<_>, Vec<_>) =
                    orig.into_iter().partition(|n| !note_ids.contains(&n.id));
                pat.notes = kept;
                removed
            })
            .unwrap_or_default();

            if !cut.is_empty() {
                let _ = ui_tx.send(UIUpdate::NotesCutToClipboard(cut));
            }
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::DeleteSelectedNotes { clip_id, note_ids } => {
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                pat.notes.retain(|n| !note_ids.contains(&n.id));
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::PasteNotes { clip_id, mut notes } => {
            for n in &mut notes {
                if n.id == 0 {
                    n.id = idgen::next();
                }
                if !n.duration.is_finite() || n.duration <= 0.0 {
                    n.duration = 1e-6;
                }
                if !n.start.is_finite() || n.start < 0.0 {
                    n.start = 0.0;
                }
            }
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                pat.notes.extend(notes);
                pat.notes
                    .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::TransposeSelectedNotes {
            clip_id,
            note_ids,
            semitones,
        } => {
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                for note in &mut pat.notes {
                    if note_ids.contains(&note.id) {
                        note.pitch = (note.pitch as i32 + semitones).clamp(0, 127) as u8;
                    }
                }
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::NudgeSelectedNotes {
            clip_id,
            note_ids,
            delta_beats,
        } => {
            with_pattern_mut(app_state, clip_id, |pat, clip_len| {
                for note in &mut pat.notes {
                    if note_ids.contains(&note.id) {
                        let new_start = (note.start + delta_beats).max(0.0);
                        let max_start = (clip_len - note.duration).max(0.0);
                        note.start = new_start.min(max_start);
                    }
                }
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::QuantizeSelectedNotes {
            clip_id,
            note_ids,
            strength,
            grid,
        } => {
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                let mut sub: Vec<_> = pat
                    .notes
                    .iter()
                    .filter(|n| note_ids.contains(&n.id))
                    .cloned()
                    .collect();
                EditProcessor::quantize_notes(&mut sub, grid as f64, strength);
                let mut sub_iter = sub.into_iter();
                for note in &mut pat.notes {
                    if note_ids.contains(&note.id) {
                        if let Some(quantized) = sub_iter.next() {
                            *note = quantized;
                        }
                    }
                }
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::HumanizeSelectedNotes {
            clip_id,
            note_ids,
            amount,
        } => {
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                let mut sub: Vec<_> = pat
                    .notes
                    .iter()
                    .filter(|n| note_ids.contains(&n.id))
                    .cloned()
                    .collect();
                EditProcessor::humanize_notes(&mut sub, amount);
                let mut sub_iter = sub.into_iter();
                for note in &mut pat.notes {
                    if note_ids.contains(&note.id) {
                        if let Some(humanized) = sub_iter.next() {
                            *note = humanized;
                        }
                    }
                }
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
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
        AudioCommand::AddNotesToClip { clip_id, mut notes } => {
            for n in &mut notes {
                if n.id == 0 {
                    n.id = idgen::next();
                }
                if !n.duration.is_finite() || n.duration <= 0.0 {
                    n.duration = 1e-6;
                }
                if !n.start.is_finite() || n.start < 0.0 {
                    n.start = 0.0;
                }
            }
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                pat.notes.extend(notes);
                pat.notes
                    .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::RemoveNotesById { clip_id, note_ids } => {
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                pat.notes.retain(|n| !note_ids.contains(&n.id));
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::UpdateNotesById { clip_id, notes } => {
            with_pattern_mut(app_state, clip_id, |pat, _len| {
                for up in notes {
                    if let Some(n) = pat.notes.iter_mut().find(|n| n.id == up.id) {
                        n.start = up.start.max(0.0);
                        n.duration = up.duration.max(1e-6);
                        n.pitch = up.pitch.clamp(0, 127);
                        n.velocity = up.velocity.clamp(1, 127);
                    }
                }
                pat.notes
                    .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            });
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::DuplicateNotesWithOffset {
            clip_id,
            source_note_ids,
            delta_beats,
            delta_semitones,
        } => {
            let sources: Vec<MidiNote> = {
                let st = app_state.lock().unwrap();
                match st.find_clip(clip_id) {
                    Some((track, ClipLocation::Midi(idx))) => {
                        let pid = track.midi_clips[idx].pattern_id;
                        pid.and_then(|p| {
                            st.patterns.get(&p).map(|pat| {
                                pat.notes
                                    .iter()
                                    .filter(|n| source_note_ids.contains(&n.id))
                                    .cloned()
                                    .collect::<Vec<_>>()
                            })
                        })
                        .unwrap_or_default()
                    }
                    _ => Vec::new(),
                }
            };

            if sources.is_empty() {
                let st = app_state.lock().unwrap();
                send_graph_snapshot(&st, snapshot_tx);
                return;
            }

            let clip_len = {
                let st = app_state.lock().unwrap();
                st.find_clip(clip_id)
                    .and_then(|(t, l)| {
                        if let ClipLocation::Midi(i) = l {
                            Some(t.midi_clips[i].length_beats)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0)
            };

            let mut new_ids = Vec::with_capacity(sources.len());
            let mut clones = Vec::with_capacity(sources.len());
            for mut n in sources.into_iter() {
                let id = idgen::next();
                n.id = id;
                new_ids.push(id);

                n.pitch = (n.pitch as i32 + delta_semitones).clamp(0, 127) as u8;
                let new_start = (n.start + delta_beats).max(0.0);
                let max_start = (clip_len - n.duration).max(0.0);
                n.start = new_start.min(max_start);

                clones.push(n);
            }

            with_pattern_mut(app_state, clip_id, |pat, _len| {
                pat.notes.extend(clones);
                pat.notes
                    .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            });

            if !new_ids.is_empty() {
                let _ = ui_tx.send(UIUpdate::ReservedNoteIds(new_ids));
            }
            let st = app_state.lock().unwrap();
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::MoveMidiClipToTrack {
            clip_id,
            dest_track_id,
            new_start,
        } => {
            // 1) Snapshot the clip to move (immutable read)
            let clip_opt = {
                let st = app_state.lock().unwrap();
                st.find_clip(clip_id).and_then(|(track, loc)| {
                    if let ClipLocation::Midi(idx) = loc {
                        track.midi_clips.get(idx).cloned()
                    } else {
                        None
                    }
                })
            };

            if let Some(mut clip) = clip_opt {
                // 2) Mutate: remove from source, insert in destination
                let mut st = app_state.lock().unwrap();

                // Type guard: destination track must be MIDI
                if !st
                    .tracks
                    .get(&dest_track_id)
                    .map_or(false, |t| matches!(t.track_type, TrackType::Midi))
                {
                    return;
                }

                // Remove from source
                if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                    if let ClipLocation::Midi(idx) = loc {
                        track.midi_clips.remove(idx);
                    }
                }
                // Update mapping
                st.clips_by_id.remove(&clip_id);

                // Insert into destination
                clip.start_beat = new_start;
                if let Some(dest) = st.tracks.get_mut(&dest_track_id) {
                    dest.midi_clips.push(clip.clone());
                }

                // Re-index
                st.clips_by_id.insert(
                    clip_id,
                    ClipRef {
                        track_id: dest_track_id,
                        is_midi: true,
                    },
                );

                send_graph_snapshot(&st, snapshot_tx);
            }
        }
        AudioCommand::MoveAudioClipToTrack {
            clip_id,
            dest_track_id,
            new_start,
        } => {
            let clip_opt = {
                let st = app_state.lock().unwrap();
                st.find_clip(clip_id).and_then(|(track, loc)| {
                    if let ClipLocation::Audio(idx) = loc {
                        track.audio_clips.get(idx).cloned()
                    } else {
                        None
                    }
                })
            };

            if let Some(mut clip) = clip_opt {
                let mut st = app_state.lock().unwrap();

                // Destination must be non-MIDI
                if !st
                    .tracks
                    .get(&dest_track_id)
                    .map_or(false, |t| !matches!(t.track_type, TrackType::Midi))
                {
                    return;
                }

                if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                    if let ClipLocation::Audio(idx) = loc {
                        track.audio_clips.remove(idx);
                    }
                }
                st.clips_by_id.remove(&clip_id);

                clip.start_beat = new_start;
                if let Some(dest) = st.tracks.get_mut(&dest_track_id) {
                    dest.audio_clips.push(clip.clone());
                }
                st.clips_by_id.insert(
                    clip_id,
                    ClipRef {
                        track_id: dest_track_id,
                        is_midi: false,
                    },
                );

                send_graph_snapshot(&st, snapshot_tx);
            }
        }
        AudioCommand::SetAudioClipGain(clip_id, gain) => {
            let mut st = app_state.lock().unwrap();
            if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    if let Some(ac) = track.audio_clips.get_mut(idx) {
                        ac.gain = gain;
                    }
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::SetAudioClipFadeIn(clip_id, dur) => {
            let mut st = app_state.lock().unwrap();
            if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    if let Some(ac) = track.audio_clips.get_mut(idx) {
                        ac.fade_in = dur;
                    }
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::SetAudioClipFadeOut(clip_id, dur) => {
            let mut st = app_state.lock().unwrap();
            if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                if let ClipLocation::Audio(idx) = loc {
                    if let Some(ac) = track.audio_clips.get_mut(idx) {
                        ac.fade_out = dur;
                    }
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::CreateMidiClipWithData { track_id, mut clip } => {
            let mut st = app_state.lock().unwrap();

            if !st
                .tracks
                .get(&track_id)
                .map_or(false, |t| matches!(t.track_type, TrackType::Midi))
            {
                return;
            }

            if clip.id == 0 {
                clip.id = idgen::next();
            }
            for n in &mut clip.notes {
                if n.id == 0 {
                    n.id = idgen::next();
                }
                if !n.duration.is_finite() || n.duration <= 0.0 {
                    n.duration = 1e-6;
                }
                if !n.start.is_finite() || n.start < 0.0 {
                    n.start = 0.0;
                }
            }

            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.midi_clips.push(clip.clone());
                st.clips_by_id.insert(
                    clip.id,
                    ClipRef {
                        track_id,
                        is_midi: true,
                    },
                );
            }

            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::SplitMidiClip { clip_id, position } => {
            // Snapshot original
            let src = {
                let st = app_state.lock().unwrap();
                st.find_clip(clip_id).and_then(|(track, loc)| {
                    if let ClipLocation::Midi(idx) = loc {
                        Some((track.id, track.midi_clips[idx].clone()))
                    } else {
                        None
                    }
                })
            };
            let Some((track_id, clip)) = src else {
                return;
            };

            let split_rel = position - clip.start_beat;
            if split_rel <= 0.0 || split_rel >= clip.length_beats {
                return;
            }

            // Resolve source notes (pattern-aware)
            let (notes, use_pattern) = {
                let st = app_state.lock().unwrap();
                if let Some(pid) = clip.pattern_id {
                    let base = st
                        .patterns
                        .get(&pid)
                        .map(|p| p.notes.clone())
                        .unwrap_or_default();
                    (base, true)
                } else {
                    (clip.notes.clone(), false)
                }
            };

            // Distribute notes across halves (split overlapping)
            let mut left_notes: Vec<MidiNote> = Vec::new();
            let mut right_notes: Vec<MidiNote> = Vec::new();

            for n in notes {
                let s = n.start;
                let e = n.start + n.duration;
                if e <= split_rel {
                    left_notes.push(n);
                } else if s >= split_rel {
                    let mut nn = n;
                    nn.start = (s - split_rel).max(0.0);
                    right_notes.push(nn);
                } else {
                    // Spans the cut: split into two
                    let mut l = n;
                    l.duration = (split_rel - s).max(1e-6);
                    left_notes.push(l);

                    let mut r = n;
                    r.start = 0.0;
                    r.duration = (e - split_rel).max(1e-6);
                    r.id = 0; // force new id below
                    right_notes.push(r);
                }
            }

            // Assign new IDs to right half duplicates
            for n in &mut right_notes {
                if n.id == 0 {
                    n.id = idgen::next();
                }
            }

            // Build two new clips; ensure pattern isolation to avoid alias bleed
            let mut left = clip.clone();
            left.length_beats = split_rel;

            let mut right = clip.clone();
            right.id = idgen::next();
            right.start_beat = position;
            right.length_beats = (clip.length_beats - split_rel).max(0.0);

            // Create fresh patterns for both halves
            let left_pid = idgen::next();
            let right_pid = idgen::next();

            left.pattern_id = Some(left_pid);
            left.notes.clear(); // use pattern
            right.pattern_id = Some(right_pid);
            right.notes.clear();

            {
                let mut st = app_state.lock().unwrap();

                // Install patterns
                st.patterns.insert(
                    left_pid,
                    crate::model::clip::MidiPattern {
                        id: left_pid,
                        notes: left_notes,
                    },
                );
                st.patterns.insert(
                    right_pid,
                    crate::model::clip::MidiPattern {
                        id: right_pid,
                        notes: right_notes,
                    },
                );

                // Replace original with left, insert right at next position
                if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                    if let ClipLocation::Midi(idx) = loc {
                        track.midi_clips[idx] = left.clone();
                        track.midi_clips.insert(idx + 1, right.clone());
                    }
                }

                // Update mapping
                st.clips_by_id.insert(
                    left.id,
                    ClipRef {
                        track_id,
                        is_midi: true,
                    },
                );
                st.clips_by_id.insert(
                    right.id,
                    ClipRef {
                        track_id,
                        is_midi: true,
                    },
                );

                send_graph_snapshot(&st, snapshot_tx);
            }
        }
        AudioCommand::SplitAudioClip { clip_id, position } => {
            // Immutable stage: get (track_id, clip clone, bpm)
            let (track_id, clip, bpm) = {
                let st = app_state.lock().unwrap();
                match st.find_clip(clip_id) {
                    Some((track, ClipLocation::Audio(idx))) => {
                        (track.id, track.audio_clips[idx].clone(), st.bpm)
                    }
                    _ => return,
                }
            };

            if let Some((mut first, mut second)) = EditProcessor::split_clip(&clip, position, bpm) {
                // Keep original id in the left part; assign a fresh id to the right part
                first.id = clip.id;
                second.id = idgen::next();
                second.start_beat = position;

                // Mutating stage: replace original and insert second right after
                let mut st = app_state.lock().unwrap();
                if let Some((track, loc)) = st.find_clip_mut(clip_id) {
                    if let ClipLocation::Audio(idx) = loc {
                        track.audio_clips[idx] = first.clone();
                        track.audio_clips.insert(idx + 1, second.clone());
                    }
                }

                // Refresh registry
                st.clips_by_id.insert(
                    first.id,
                    ClipRef {
                        track_id,
                        is_midi: false,
                    },
                );
                st.clips_by_id.insert(
                    second.id,
                    ClipRef {
                        track_id,
                        is_midi: false,
                    },
                );

                send_graph_snapshot(&st, snapshot_tx);
            }
        }
        AudioCommand::SetTrackInput(track_id, input) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.input_device = input;
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::SetTrackOutput(track_id, output) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.output_device = output;
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::FreezeTrack(track_id) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.frozen = true;
                t.frozen_buffer = None;
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::UnfreezeTrack(track_id) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                t.frozen = false;
                t.frozen_buffer = None;
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::SetAutomationMode(track_id, lane_idx, automation_mode) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                if let Some(lane) = t.automation_lanes.get_mut(lane_idx) {
                    lane.write_mode = automation_mode;
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::ClearAutomationLane(track_id, lane_idx) => {
            let mut st = app_state.lock().unwrap();
            if let Some(t) = st.tracks.get_mut(&track_id) {
                if let Some(lane) = t.automation_lanes.get_mut(lane_idx) {
                    lane.points.clear();
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::PunchOutAudioClip {
            clip_id,
            start_beat,
            end_beat,
        } => {
            let mut st = app_state.lock().unwrap();
            let (track_id, original_clip_opt) =
                if let Some((track, ClipLocation::Audio(idx))) = st.find_clip(clip_id) {
                    (track.id, track.audio_clips.get(idx).cloned())
                } else {
                    (0, None)
                };

            if let Some(original_clip) = original_clip_opt {
                let clip_start = original_clip.start_beat;
                let clip_end = original_clip.start_beat + original_clip.length_beats;

                // Ensure punch-out is fully within the clip
                if start_beat > clip_start && end_beat < clip_end {
                    // 1. Create the right-hand part as a new clip
                    let mut right_part = original_clip.clone();
                    right_part.id = idgen::next();
                    right_part.start_beat = end_beat;
                    let right_len = clip_end - end_beat;
                    right_part.length_beats = right_len;
                    // Adjust audio sample offset for the new right-hand clip
                    let converter = crate::time_utils::TimeConverter::new(st.sample_rate, st.bpm);
                    let right_offset_beats = converter
                        .samples_to_beats(converter.beats_to_samples(
                            original_clip.offset_beats + (end_beat - clip_start),
                        ));
                    right_part.offset_beats = right_offset_beats;

                    // 2. Modify the original clip to become the left-hand part
                    if let Some((track, ClipLocation::Audio(idx))) = st.find_clip_mut(clip_id) {
                        if let Some(left_part) = track.audio_clips.get_mut(idx) {
                            left_part.length_beats = start_beat - clip_start;
                        }
                        // 3. Insert the new right-hand part
                        track.audio_clips.push(right_part.clone());
                        st.clips_by_id.insert(
                            right_part.id,
                            ClipRef {
                                track_id,
                                is_midi: false,
                            },
                        );
                    }
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
        AudioCommand::PunchOutMidiClip {
            clip_id,
            start_beat,
            end_beat,
        } => {
            // This is more complex due to patterns. We will split the pattern's notes.
            let mut st = app_state.lock().unwrap();
            let (track_id, original_clip_opt) =
                if let Some((track, ClipLocation::Midi(idx))) = st.find_clip(clip_id) {
                    (track.id, track.midi_clips.get(idx).cloned())
                } else {
                    (0, None)
                };

            if let Some(original_clip) = original_clip_opt {
                let clip_start = original_clip.start_beat;
                let clip_end = original_clip.start_beat + original_clip.length_beats;

                if start_beat > clip_start && end_beat < clip_end {
                    // For MIDI, we can simply create two new clips that reference the same pattern.
                    // The audio engine's MIDI processing logic will correctly play only the notes
                    // within each clip's time window.

                    // 1. Create the right-hand part
                    let mut right_part = original_clip.clone();
                    right_part.id = idgen::next();
                    right_part.start_beat = end_beat;
                    right_part.length_beats = clip_end - end_beat;

                    // 2. Modify the original to become the left-hand part
                    if let Some((track, ClipLocation::Midi(idx))) = st.find_clip_mut(clip_id) {
                        if let Some(left_part) = track.midi_clips.get_mut(idx) {
                            left_part.length_beats = start_beat - clip_start;
                        }
                        // 3. Insert the new right-hand part
                        track.midi_clips.push(right_part.clone());
                        st.clips_by_id.insert(
                            right_part.id,
                            ClipRef {
                                track_id,
                                is_midi: true,
                            },
                        );
                    }
                }
            }
            send_graph_snapshot(&st, snapshot_tx);
        }
    }
}

pub fn send_graph_snapshot(state: &AppState, snapshot_tx: &Sender<AudioGraphSnapshot>) {
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
            // benign
        } else {
            log::error!("Failed to send audio graph snapshot: audio thread may have crashed.");
        }
    }
}

// Create a recording MIDI clip at start_beat if none spans that beat.
fn insert_recording_clip_if_missing(
    app_state: &Arc<std::sync::Mutex<AppState>>,
    track_id: u64,
    start_beat: f64,
) {
    let need_insert = {
        let st = app_state.lock().unwrap();
        match st.tracks.get(&track_id) {
            Some(t) => !t
                .midi_clips
                .iter()
                .any(|c| start_beat >= c.start_beat && start_beat < c.start_beat + c.length_beats),
            None => false,
        }
    };
    if !need_insert {
        return;
    }

    let mut st = app_state.lock().unwrap();
    let new_clip_id = idgen::next();
    let new_pid = idgen::next();
    st.patterns.insert(
        new_pid,
        MidiPattern {
            id: new_pid,
            notes: Vec::new(),
        },
    );
    if let Some(t) = st.tracks.get_mut(&track_id) {
        t.midi_clips.push(MidiClip {
            id: new_clip_id,
            name: format!("Rec @ Beat {:.1}", start_beat),
            start_beat: start_beat.floor(),
            length_beats: 64.0,
            pattern_id: Some(new_pid),
            ..Default::default()
        });
        t.midi_clips
            .sort_by(|a, b| a.start_beat.partial_cmp(&b.start_beat).unwrap());
    }
    st.ensure_ids();
}

// Borrow-safe helper: resolve a clip's pattern and length, then mutate in place.
fn with_pattern_mut<T>(
    app_state: &Arc<std::sync::Mutex<AppState>>,
    clip_id: u64,
    f: impl FnOnce(&mut MidiPattern, f64) -> T,
) -> Option<T> {
    // Stage 1: resolve pid and clip length immutably
    let (pid_opt, clip_len) = {
        let st = app_state.lock().unwrap();
        match st.find_clip(clip_id) {
            Some((track, ClipLocation::Midi(idx))) => {
                let c = &track.midi_clips[idx];
                (c.pattern_id, c.length_beats)
            }
            _ => (None, 0.0),
        }
    };
    let pid = pid_opt?;

    // Stage 2: short &mut borrow for pattern only
    let mut st = app_state.lock().unwrap();
    let pat = st.patterns.get_mut(&pid)?;
    Some(f(pat, clip_len))
}

/// State for an in-progress MIDI recording.
struct MidiRecordingState {
    /// The track ID we are recording to.
    track_id: u64,
    active_notes: HashMap<(u8, u8), (f64, u8)>,
}
