use crate::constants::{
    DEFAULT_AUDIO_TRACK_PREFIX, DEFAULT_MIDI_TRACK_PREFIX, DEFAULT_TRACK_VOLUME,
};
use crate::state::{AudioCommand, MidiNote, Pattern, Track};
use crossbeam_channel::Sender;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackType {
    Audio,
    Midi,
    Bus,
    Master,
}

#[derive(Debug, Clone)]
pub struct TrackGroup {
    pub name: String,
    pub track_ids: Vec<usize>,
    pub color: egui::Color32,
    pub collapsed: bool,
}

pub struct TrackManager {
    next_track_id: usize,
    groups: Vec<TrackGroup>,
}

impl TrackManager {
    pub fn new() -> Self {
        Self {
            next_track_id: 0,
            groups: Vec::new(),
        }
    }

    pub fn create_track(&mut self, track_type: TrackType, name: Option<String>) -> Track {
        let id = self.next_track_id;
        self.next_track_id += 1;

        let (default_name, is_midi) = match track_type {
            TrackType::Audio => (format!("{} {}", DEFAULT_AUDIO_TRACK_PREFIX, id + 1), false),
            TrackType::Midi => (format!("{} {}", DEFAULT_MIDI_TRACK_PREFIX, id + 1), true),
            TrackType::Bus => (format!("Bus {}", id + 1), false),
            TrackType::Master => ("Master".to_string(), false),
        };

        let mut track = Track {
            id,
            name: name.unwrap_or(default_name),
            volume: DEFAULT_TRACK_VOLUME,
            pan: 0.0,
            muted: false,
            solo: false,
            armed: false,
            plugin_chain: vec![],
            patterns: vec![],
            is_midi,
            audio_clips: vec![],
            automation_lanes: vec![],
        };

        // Add default pattern for MIDI tracks
        if is_midi {
            track.patterns.push(Pattern {
                name: "Pattern 1".to_string(),
                length: 4.0,
                notes: Self::create_default_notes(),
            });
        }

        track
    }

    fn create_default_notes() -> Vec<MidiNote> {
        vec![
            MidiNote {
                pitch: 60,
                velocity: 100,
                start: 0.0,
                duration: 0.5,
            },
            MidiNote {
                pitch: 62,
                velocity: 100,
                start: 0.5,
                duration: 0.5,
            },
            MidiNote {
                pitch: 64,
                velocity: 100,
                start: 1.0,
                duration: 0.5,
            },
            MidiNote {
                pitch: 65,
                velocity: 100,
                start: 1.5,
                duration: 0.5,
            },
            MidiNote {
                pitch: 67,
                velocity: 100,
                start: 2.0,
                duration: 0.5,
            },
            MidiNote {
                pitch: 69,
                velocity: 100,
                start: 2.5,
                duration: 0.5,
            },
            MidiNote {
                pitch: 71,
                velocity: 100,
                start: 3.0,
                duration: 0.5,
            },
            MidiNote {
                pitch: 72,
                velocity: 100,
                start: 3.5,
                duration: 0.5,
            },
        ]
    }

    pub fn duplicate_track(&mut self, source_track: &Track) -> Track {
        let mut new_track = source_track.clone();
        new_track.id = self.next_track_id;
        self.next_track_id += 1;
        new_track.name = format!("{} (Copy)", source_track.name);
        new_track
    }

    pub fn create_group(&mut self, name: String, track_ids: Vec<usize>) -> usize {
        let group = TrackGroup {
            name,
            track_ids,
            color: egui::Color32::from_rgb(100, 150, 200),
            collapsed: false,
        };
        self.groups.push(group);
        self.groups.len() - 1
    }

    pub fn add_to_group(&mut self, group_id: usize, track_id: usize) {
        if let Some(group) = self.groups.get_mut(group_id) {
            if !group.track_ids.contains(&track_id) {
                group.track_ids.push(track_id);
            }
        }
    }

    pub fn remove_from_group(&mut self, group_id: usize, track_id: usize) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.track_ids.retain(|&id| id != track_id);
        }
    }

    pub fn get_groups(&self) -> &[TrackGroup] {
        &self.groups
    }

    pub fn toggle_group(&mut self, group_id: usize) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.collapsed = !group.collapsed;
        }
    }
}

// Helper functions for track operations
pub fn solo_track(tracks: &mut Vec<Track>, track_id: usize, command_tx: &Sender<AudioCommand>) {
    if let Some(track) = tracks.get_mut(track_id) {
        let new_solo = !track.solo;
        track.solo = new_solo;

        // If soloing this track, unsolo all others
        if new_solo {
            for (i, t) in tracks.iter_mut().enumerate() {
                if i != track_id && t.solo {
                    t.solo = false;
                    let _ = command_tx.send(AudioCommand::SoloTrack(i, false));
                }
            }
        }

        let _ = command_tx.send(AudioCommand::SoloTrack(track_id, new_solo));
    }
}

pub fn mute_track(tracks: &mut Vec<Track>, track_id: usize, command_tx: &Sender<AudioCommand>) {
    if let Some(track) = tracks.get_mut(track_id) {
        track.muted = !track.muted;
        let _ = command_tx.send(AudioCommand::MuteTrack(track_id, track.muted));
    }
}

pub fn arm_track_exclusive(tracks: &mut Vec<Track>, track_id: usize) {
    // Disarm all tracks first
    for t in tracks.iter_mut() {
        t.armed = false;
    }

    // Arm selected track
    if let Some(track) = tracks.get_mut(track_id) {
        track.armed = true;
    }
}

pub fn delete_track(tracks: &mut Vec<Track>, track_id: usize) -> Option<Track> {
    if track_id < tracks.len() {
        return Some(tracks.remove(track_id));
    }
    None
}

pub fn move_track(tracks: &mut Vec<Track>, from: usize, to: usize) {
    if from < tracks.len() && to < tracks.len() && from != to {
        let track = tracks.remove(from);
        let insert_pos = if from < to { to - 1 } else { to };
        tracks.insert(insert_pos, track);
    }
}
