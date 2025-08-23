use crate::audio_state::AutomationTarget;
use crate::time_utils::TimeConverter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: f64,
    pub value: f32,
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
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub is_midi: bool,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub midi_clips: Vec<MidiClip>,
    pub audio_clips: Vec<AudioClip>,
    pub plugin_chain: Vec<PluginDescriptor>,
    pub automation_lanes: Vec<AutomationLane>,
    pub sends: Vec<Send>,
    pub group_id: Option<usize>,
    pub color: Option<(u8, u8, u8)>,
    pub height: f32,
    pub minimized: bool,
    pub record_enabled: bool,
    pub monitor_enabled: bool,
    pub input_gain: f32,
    pub phase_inverted: bool,
    pub frozen: bool,
    pub frozen_buffer: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Send {
    pub destination_track: usize,
    pub amount: f32,
    pub pre_fader: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub uri: String,
    pub name: String,
    pub bypass: bool,
    pub params: HashMap<String, f32>,
    pub preset_name: Option<String>,
    pub custom_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    pub parameter: AutomationTarget,
    pub points: Vec<AutomationPoint>,
    pub visible: bool,
    pub height: f32,
    pub color: Option<(u8, u8, u8)>,
    pub write_mode: AutomationMode,
    pub read_enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AutomationMode {
    Off,
    Read,
    Write,
    Touch,
    Latch,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            name: "New Track".to_string(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
            armed: false,
            is_midi: false,
            input_device: None,
            output_device: None,
            midi_clips: Vec::new(),
            audio_clips: Vec::new(),
            plugin_chain: Vec::new(),
            automation_lanes: Vec::new(),
            sends: Vec::new(),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub samples: Vec<f32>,
    pub sample_rate: f32,
    pub fade_in: Option<f64>,  // Fade in duration in beats
    pub fade_out: Option<f64>, // Fade out duration in beats
    pub gain: f32,             // Clip gain
    pub pitch_shift: f32,      // Semitones
    pub time_stretch: f32,     // Stretch factor
    pub reverse: bool,         // Play in reverse
    pub loop_enabled: bool,    // Loop this clip
    pub color: Option<(u8, u8, u8)>,
    pub muted: bool,
    pub locked: bool,               // Prevent editing
    pub crossfade_in: Option<f64>,  // Crossfade with previous clip
    pub crossfade_out: Option<f64>, // Crossfade with next clip
}

impl Default for AudioClip {
    fn default() -> Self {
        Self {
            name: "Audio Clip".to_string(),
            start_beat: 0.0,
            length_beats: 4.0,
            samples: Vec::new(),
            sample_rate: 44100.0,
            fade_in: None,
            fade_out: None,
            gain: 1.0,
            pitch_shift: 0.0,
            time_stretch: 1.0,
            reverse: false,
            loop_enabled: false,
            color: None,
            muted: false,
            locked: false,
            crossfade_in: None,
            crossfade_out: None,
        }
    }
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
pub struct MidiClip {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub notes: Vec<MidiNote>,
    pub color: Option<(u8, u8, u8)>,
    pub velocity_offset: i8, // Add/subtract from all velocities
    pub transpose: i8,       // Transpose all notes
    pub loop_enabled: bool,  // Loop this clip
    pub muted: bool,
    pub locked: bool,           // Prevent editing
    pub groove: Option<String>, // Apply groove template
    pub swing: f32,             // Swing amount
    pub humanize: f32,          // Humanization amount
}

impl Default for MidiClip {
    fn default() -> Self {
        Self {
            name: "MIDI Clip".to_string(),
            start_beat: 0.0,
            length_beats: 4.0,
            notes: Vec::new(),
            color: Some((100, 150, 200)),
            velocity_offset: 0,
            transpose: 0,
            loop_enabled: false,
            muted: false,
            locked: false,
            groove: None,
            swing: 0.0,
            humanize: 0.0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct AppState {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub sample_rate: f32,
    pub buffer_size: usize,
    pub current_position: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    time_signature: (i32, i32),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub version: String,
    pub name: String,
    pub tracks: Vec<Track>,
    pub bpm: f32,
    pub time_signature: (i32, i32),
    pub sample_rate: f32,
    pub master_volume: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

impl From<&AppState> for Project {
    fn from(state: &AppState) -> Self {
        Project {
            version: "1.0.0".to_string(),
            name: "Untitled Project".to_string(),
            tracks: state.tracks.clone(),
            bpm: state.bpm,
            time_signature: state.time_signature,
            sample_rate: state.sample_rate,
            master_volume: state.master_volume,
            loop_start: state.loop_start,
            loop_end: state.loop_end,
            loop_enabled: state.loop_enabled,
            created_at: chrono::Utc::now(),
            modified_at: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub length: f64,
    pub notes: Vec<MidiNote>,
}

impl From<MidiClip> for Pattern {
    fn from(clip: MidiClip) -> Self {
        Pattern {
            name: clip.name,
            length: clip.length_beats,
            notes: clip.notes,
        }
    }
}

impl From<Pattern> for MidiClip {
    fn from(pattern: Pattern) -> Self {
        MidiClip {
            name: pattern.name,
            start_beat: 0.0,
            length_beats: pattern.length,
            notes: pattern.notes,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub bpm: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    sample_rate: f32,
    time_signature: (i32, i32),
    playing: bool,
    recording: bool,
}

impl AppState {
    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            tracks: self.tracks.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            playing: self.playing,
            recording: self.recording,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
        }
    }

    pub fn restore(&mut self, snapshot: AppStateSnapshot) {
        self.tracks = snapshot.tracks;
        self.bpm = snapshot.bpm;
        self.time_signature = snapshot.time_signature;
        self.sample_rate = snapshot.sample_rate;
        self.playing = snapshot.playing;
        self.recording = snapshot.recording;
        self.master_volume = snapshot.master_volume;
        self.loop_start = snapshot.loop_start;
        self.loop_end = snapshot.loop_end;
        self.loop_enabled = snapshot.loop_enabled;
    }

    pub fn position_to_beats(&self, position: f64) -> f64 {
        let converter = TimeConverter::new(self.sample_rate, self.bpm);
        converter.samples_to_beats(position)
    }

    pub fn beats_to_samples(&self, beats: f64) -> f64 {
        let converter = TimeConverter::new(self.sample_rate, self.bpm);
        converter.beats_to_samples(beats)
    }

    pub fn load_project(&mut self, project: Project) {
        self.tracks = project.tracks;
        self.bpm = project.bpm;
        self.time_signature = project.time_signature;
        self.sample_rate = project.sample_rate;
        self.master_volume = project.master_volume;
        self.loop_start = project.loop_start;
        self.loop_end = project.loop_end;
        self.loop_enabled = project.loop_enabled;
    }

    pub fn to_project(&self) -> Project {
        Project {
            version: "1.0.0".to_string(),
            name: "Untitled Project".to_string(),
            tracks: self.tracks.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
            created_at: chrono::Utc::now(),
            modified_at: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    Stop,
    Pause,
    Record,
    SetPosition(f64),
    SetBPM(f32),
    SetMasterVolume(f32),

    UpdateTracks,
    SetTrackVolume(usize, f32),
    SetTrackPan(usize, f32),
    SetTrackMute(usize, bool),
    SetTrackSolo(usize, bool),
    SetTrackArmed(usize, bool),
    SetTrackInput(usize, Option<String>),
    SetTrackOutput(usize, Option<String>),
    SetTrackMonitor(usize, bool),
    FreezeTrack(usize),
    UnfreezeTrack(usize),

    AddPlugin(usize, String),
    RemovePlugin(usize, usize),
    SetPluginBypass(usize, usize, bool),
    SetPluginParam(usize, usize, String, f32),
    MovePlugin(usize, usize, usize), // track_id, from_index, to_index
    LoadPluginPreset(usize, usize, String),
    SavePluginPreset(usize, usize, String),

    SetLoopEnabled(bool),
    SetLoopRegion(f64, f64),

    CreateMidiClip(usize, f64, f64),
    CreateMidiClipWithData(usize, MidiClip),
    UpdateMidiClip(usize, usize, Vec<MidiNote>),
    DeleteMidiClip(usize, usize),
    MoveMidiClip(usize, usize, f64),
    ResizeMidiClip(usize, usize, f64, f64), // track_id, clip_id, start, length
    DuplicateMidiClip(usize, usize),
    SplitMidiClip(usize, usize, f64),

    MoveAudioClip(usize, usize, f64),
    ResizeAudioClip(usize, usize, f64, f64),
    DuplicateAudioClip(usize, usize),
    SplitAudioClip(usize, usize, f64),
    DeleteAudioClip(usize, usize),
    SetAudioClipGain(usize, usize, f32),
    SetAudioClipFadeIn(usize, usize, Option<f64>),
    SetAudioClipFadeOut(usize, usize, Option<f64>),

    AddAutomationPoint(usize, AutomationTarget, f64, f32),
    RemoveAutomationPoint(usize, usize, f64),
    UpdateAutomationPoint(usize, usize, f64, f64, f32), // track, lane, old_beat, new_beat, value
    SetAutomationMode(usize, usize, AutomationMode),
    ClearAutomationLane(usize, usize),

    AddNote(usize, usize, MidiNote),
    RemoveNote(usize, usize, usize),
    UpdateNote(usize, usize, usize, MidiNote),
    PreviewNote(usize, u8),
    StopPreviewNote,

    AddSend(usize, usize, f32), // from_track, to_track, amount
    RemoveSend(usize, usize),
    SetSendAmount(usize, usize, f32),
    SetSendPreFader(usize, usize, bool),

    CreateGroup(String, Vec<usize>),
    RemoveGroup(usize),
    AddTrackToGroup(usize, usize),
    RemoveTrackFromGroup(usize),
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    TrackLevels(Vec<(f32, f32)>),
    RecordingFinished(usize, AudioClip),
    RecordingLevel(f32),
    MasterLevel(f32, f32),
    PushUndo(AppStateSnapshot),

    PerformanceMetric {
        cpu_usage: f32,
        buffer_fill: f32,
        xruns: u32,
    },

    TrackAdded(usize),
    TrackRemoved(usize),
    TrackUpdated(usize),

    ClipAdded(usize, usize), // track_id, clip_id
    ClipRemoved(usize, usize),
    ClipUpdated(usize, usize),

    AutomationUpdated(usize, usize), // track_id, lane_id

    PluginAdded(usize, usize), // track_id, plugin_id
    PluginRemoved(usize, usize),
    PluginUpdated(usize, usize),

    Error(String),
    Warning(String),
    Info(String),
}
