use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: f64,
    pub value: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    pub parameter: AutomationTarget,
    pub points: BTreeMap<OrderedFloat<f64>, f32>,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AutomationTarget {
    TrackVolume,
    TrackPan,
    PluginParam {
        plugin_idx: usize,
        param_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct OrderedFloat<T>(pub T);

impl Eq for OrderedFloat<f64> {}
impl Ord for OrderedFloat<f64> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: usize,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub plugin_chain: Vec<PluginDescriptor>,
    pub patterns: Vec<Pattern>,
    pub is_midi: bool,
    pub audio_clips: Vec<AudioClip>,
    pub automation_lanes: Vec<AutomationLane>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub samples: Vec<f32>,
    pub sample_rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub uri: String,
    pub name: String,
    pub bypass: bool,
    pub params: HashMap<String, PluginParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginParam {
    pub index: usize,
    pub name: String,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MidiNote {
    pub pitch: u8,
    pub velocity: u8,
    pub start: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub length: f64,
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
    pub current_position: f64,
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
                    automation_lanes: vec![],
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
                    automation_lanes: vec![],
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

    pub fn load_project(&mut self, project: Project) {
        self.tracks = project.tracks;
        self.master_volume = project.master_volume;
        self.bpm = project.bpm;
        self.playing = false;
        self.current_position = 0.0;
    }

    pub fn position_to_beats(&self, position: f64) -> f64 {
        (position / self.sample_rate as f64) * (self.bpm as f64 / 60.0)
    }

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

// Audio commands
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    Stop,
    Record,
    UpdateTracks,
    StopRecording,
    SetTrackVolume(usize, f32),
    SetTrackPan(usize, f32),
    MuteTrack(usize, bool),
    SoloTrack(usize, bool),
    AddPlugin(usize, String),
    RemovePlugin(usize, usize),
    SetPluginParam(usize, usize, String, f32),
    SetPluginBypass(usize, usize, bool),
    AddNote(usize, usize, MidiNote),
    RemoveNote(usize, usize, usize),
    UpdateNote(usize, usize, usize, MidiNote),
    StartRecording(usize),
    SetRecordingInput(String),
    SaveProject(String),
    LoadProject(String),
    SplitClip(usize, usize, f64),
    DeleteClip(usize, usize),
    TrimClipStart(usize, usize, f64),
    TrimClipEnd(usize, usize, f64),
    PreviewNote(usize, u8),
    StopPreviewNote,
    AddAutomationPoint(usize, AutomationTarget, f64, f32), // track_id, target, beat, value
    RemoveAutomationPoint(usize, usize, f64),              // track_id, lane_idx, beat
    UpdateAutomationPoint(usize, usize, f64, f32),         // track_id, lane_idx, beat, new_value
    SetAutomationVisible(usize, usize, bool),              // track_id, lane_idx, visible
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    PeakLevel(usize, f32, f32),
    PluginAdded(usize, String),
    RecordingLevel(f32),
    RecordingFinished(usize, AudioClip),
    TrackLevels(Vec<(f32, f32)>),
    MasterLevel(f32, f32),
    PushUndo(AppStateSnapshot),
}
