use crate::constants::{
    DEFAULT_AUDIO_TRACK_PREFIX, DEFAULT_MIDI_TRACK_PREFIX, DEFAULT_MIN_PROJECT_BEATS,
    DEFAULT_TRACK_VOLUME,
};
use crate::messages::AudioCommand;
use crate::model::clip::{MidiClip, MidiNote};
use crate::model::track::Track;
use crossbeam_channel::Sender;
use eframe::egui;

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

#[derive(Debug, Clone)]
pub struct TrackBuilder {
    id_hint: usize,
    name: Option<String>,
    track_type: TrackType,
    volume: Option<f32>,
    pan: Option<f32>,
    midi_clips: Vec<MidiClip>,
}

impl TrackBuilder {
    pub fn new(id_hint: usize, track_type: TrackType) -> Self {
        Self {
            id_hint,
            name: None,
            track_type,
            volume: None,
            pan: None,
            midi_clips: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = Some(volume);
        self
    }

    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = Some(pan);
        self
    }

    pub fn with_default_pattern(mut self) -> Self {
        if self.track_type == TrackType::Midi {
            self.midi_clips.push(Self::create_default_pattern());
        }
        self
    }

    pub fn with_midi_clips(mut self, midi_clips: Vec<MidiClip>) -> Self {
        self.midi_clips = midi_clips;
        self
    }

    pub fn build(self) -> Track {
        let (default_name, is_midi) = match self.track_type {
            TrackType::Audio => (
                format!("{} {}", DEFAULT_AUDIO_TRACK_PREFIX, self.id_hint + 1),
                false,
            ),
            TrackType::Midi => (
                format!("{} {}", DEFAULT_MIDI_TRACK_PREFIX, self.id_hint + 1),
                true,
            ),
            TrackType::Bus => (format!("Bus {}", self.id_hint + 1), false),
            TrackType::Master => ("Master".to_string(), false),
        };

        Track {
            name: self.name.unwrap_or(default_name),
            volume: self.volume.unwrap_or(DEFAULT_TRACK_VOLUME),
            pan: self.pan.unwrap_or(0.0),
            muted: false,
            solo: false,
            armed: false,
            is_midi,
            input_device: None,
            output_device: None,
            midi_clips: self.midi_clips,
            audio_clips: vec![],
            plugin_chain: vec![],
            automation_lanes: vec![],
            sends: vec![],
            group_id: None,
            color: None,
            height: 80.0,
            minimized: false,
            record_enabled: false,
            monitor_enabled: false,
            input_gain: 1.0,
            phase_inverted: false,
            frozen: false,
            frozen_buffer: None,
        }
    }

    pub fn create_default_pattern() -> MidiClip {
        MidiClip {
            name: "Pattern 1".to_string(),
            notes: Self::create_default_notes(),
            start_beat: 0.0,
            length_beats: DEFAULT_MIN_PROJECT_BEATS,
            color: Some((1, 1, 1)),
            ..Default::default()
        }
    }

    pub fn create_default_notes() -> Vec<MidiNote> {
        vec![
            MidiNote {
                pitch: 60,
                velocity: 100,
                start: 0.0,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 62,
                velocity: 100,
                start: 0.5,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 64,
                velocity: 100,
                start: 1.0,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 65,
                velocity: 100,
                start: 1.5,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 67,
                velocity: 100,
                start: 2.0,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 69,
                velocity: 100,
                start: 2.5,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 71,
                velocity: 100,
                start: 3.0,
                duration: 0.5,
                id: 0,
            },
            MidiNote {
                pitch: 72,
                velocity: 100,
                start: 3.5,
                duration: 0.5,
                id: 0,
            },
        ]
    }
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

        let mut builder = TrackBuilder::new(id, track_type);

        if let Some(name) = name {
            builder = builder.with_name(name);
        }

        if track_type == TrackType::Midi {
            builder = builder.with_default_pattern();
        }

        builder.build()
    }

    pub fn duplicate_track(&self, src: &crate::model::track::Track) -> crate::model::track::Track {
        let mut t = src.clone();

        // Reset IDs for fresh ones
        for c in &mut t.audio_clips {
            c.id = 0;
        }

        for c in &mut t.midi_clips {
            c.id = 0;
            c.pattern_id = None;

            if !c.content_len_beats.is_finite() || c.content_len_beats <= 0.0 {
                c.content_len_beats = c.length_beats.max(0.000001);
            }
            if !c.content_offset_beats.is_finite() {
                c.content_offset_beats = 0.0;
            }
            c.content_offset_beats = c
                .content_offset_beats
                .rem_euclid(c.content_len_beats.max(0.000001));

            for n in &mut c.notes {
                n.id = 0;
            }
        }
        t
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

    pub fn sanitize(&mut self, tracks_len: usize) {
        for g in &mut self.groups {
            g.track_ids.retain(|&i| i < tracks_len);
            g.track_ids.sort_unstable();
            g.track_ids.dedup();
        }
    }
}

// Helper functions for track operations
pub fn solo_track(tracks: &mut Vec<Track>, track_id: usize, command_tx: &Sender<AudioCommand>) {
    if let Some(track) = tracks.get_mut(track_id) {
        let new_solo = !track.solo;
        track.solo = new_solo;

        if new_solo {
            for (i, t) in tracks.iter_mut().enumerate() {
                if i != track_id && t.solo {
                    t.solo = false;
                    let _ = command_tx.send(AudioCommand::SetTrackSolo(i, false));
                }
            }
        }

        let _ = command_tx.send(AudioCommand::SetTrackSolo(track_id, new_solo));
    }
}

pub fn create_default_audio_track(id: usize) -> Track {
    TrackBuilder::new(id, TrackType::Audio)
        .with_name(format!("Audio {}", id + 1))
        .build()
}

pub fn create_default_midi_track(id: usize) -> Track {
    TrackBuilder::new(id, TrackType::Midi)
        .with_name(format!("MIDI {}", id + 1))
        .with_default_pattern()
        .build()
}

pub fn create_master_track() -> Track {
    TrackBuilder::new(0, TrackType::Master)
        .with_volume(0.8)
        .build()
}

pub fn mute_track(tracks: &mut Vec<Track>, track_id: usize, command_tx: &Sender<AudioCommand>) {
    if let Some(track) = tracks.get_mut(track_id) {
        track.muted = !track.muted;
        let _ = command_tx.send(AudioCommand::SetTrackMute(track_id, track.muted));
    }
}

pub fn arm_track_exclusive(tracks: &mut Vec<Track>, track_id: usize) {
    for t in tracks.iter_mut() {
        t.armed = false;
    }
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
