use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: usize,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub plugin_chain: Vec<PluginInstance>,
    pub patterns: Vec<Pattern>,
    pub is_midi: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInstance {
    pub uri: String,
    pub name: String,
    pub bypass: bool,
    pub params: HashMap<String, f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MidiNote {
    pub pitch: u8,     // MIDI note number (0-127)
    pub velocity: u8,  // Note velocity (0-127)
    pub start: f64,    // Start time in beats
    pub duration: f64, // Duration in beats
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub length: f64, // Length in beats
    pub notes: Vec<MidiNote>,
}

#[derive(Debug)]
pub struct AppState {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub sample_rate: f32,
    pub buffer_size: usize,
    pub current_position: f64, // in samples
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tracks: vec![
                Track {
                    id: 0,
                    name: "Audio 1".to_string(),
                    volume: 0.7,
                    pan: 0.0,
                    muted: false,
                    solo: false,
                    armed: false,
                    plugin_chain: vec![],
                    patterns: vec![],
                    is_midi: false,
                },
                Track {
                    id: 1,
                    name: "MIDI 1".to_string(),
                    volume: 0.7,
                    pan: 0.0,
                    muted: false,
                    solo: false,
                    armed: false,
                    plugin_chain: vec![],
                    patterns: vec![Pattern {
                        name: "Pattern 1".to_string(),
                        length: 4.0,
                        notes: vec![
                            // C major scale
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
                        ],
                    }],
                    is_midi: true,
                },
            ],
            master_volume: 0.8,
            playing: false,
            recording: false,
            bpm: 120.0,
            sample_rate: 44100.0,
            buffer_size: 512,
            current_position: 0.0,
        }
    }

    // Add helper to convert position in samples to beats
    pub fn position_to_beats(&self, position: f64) -> f64 {
        (position / self.sample_rate as f64) * ((self.bpm / 60.0) as f64)
    }

    // Add helper to convert beats to samples
    pub fn beats_to_samples(&self, beats: f64) -> f64 {
        (beats * 60.0 / self.bpm as f64) * self.sample_rate as f64
    }
}

// Add new commands for MIDI editing
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    Stop,
    Record,
    StopRecording,
    SetTrackVolume(usize, f32),
    SetTrackPan(usize, f32),
    MuteTrack(usize, bool),
    SoloTrack(usize, bool),
    AddPlugin(usize, String),
    RemovePlugin(usize, usize),
    SetPluginParam(usize, usize, String, f32),
    // Add MIDI commands
    AddNote(usize, usize, MidiNote), // track_id, pattern_id, note
    RemoveNote(usize, usize, usize), // track_id, pattern_id, note_index
    UpdateNote(usize, usize, usize, MidiNote), // track_id, pattern_id, note_index, new_note
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    PeakLevel(usize, f32, f32), // track_id, left, right
    PluginAdded(usize, String),
}
