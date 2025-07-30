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
    pub audio_clips: Vec<AudioClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub samples: Vec<f32>, // Mono samples, we'll convert from stereo
    pub sample_rate: f32,
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

#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub bpm: f32,
}

impl From<&AppState> for Project {
    fn from(state: &AppState) -> Self {
        Project {
            name: "Untitled Project".to_string(),
            tracks: state.tracks.clone(),
            master_volume: state.master_volume,
            bpm: state.bpm,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub bpm: f32,
}

impl AppState {
    pub fn load_project(&mut self, project: Project) {
        self.tracks = project.tracks;
        self.master_volume = project.master_volume;
        self.bpm = project.bpm;
        self.playing = false;
        self.current_position = 0.0;
    }

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
                    audio_clips: Vec::new(),
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
                    audio_clips: Vec::new(),
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

    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            tracks: self.tracks.clone(),
            master_volume: self.master_volume,
            bpm: self.bpm,
        }
    }

    pub fn restore(&mut self, snapshot: AppStateSnapshot) {
        self.tracks = snapshot.tracks;
        self.master_volume = snapshot.master_volume;
        self.bpm = snapshot.bpm;
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
    RemoveNote(usize, usize, usize), // ||, || , note_index
    UpdateNote(usize, usize, usize, MidiNote), // ||, || , note_index, new_note
    StartRecording(usize),
    SetRecordingInput(String),
    SaveProject(String), // filepath
    LoadProject(String),
    SplitClip(usize, usize, f64), // track_id, clip_id, beat_position
    DeleteClip(usize, usize),
    TrimClipStart(usize, usize, f64), // ||, ||, new_start_beat
    TrimClipEnd(usize, usize, f64),
    PreviewNote(usize, u8), // track_id, pitch
    StopPreviewNote,
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    PeakLevel(usize, f32, f32), // track_id, left, right
    PluginAdded(usize, String),
    RecordingLevel(f32), // peak level
    RecordingFinished(usize, AudioClip),
    TrackLevels(Vec<(f32, f32)>),
}
