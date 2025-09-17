use rayon::vec;
use serde::{Deserialize, Serialize};

use crate::model::{
    automation::{AutomationMode, AutomationTarget},
    clip::{AudioClip, MidiClip, MidiNote},
    plugin_api::BackendKind,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NoteDelta {
    Set { index: usize, note: MidiNote },
    Add { index: usize, note: MidiNote },
    Remove { index: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    AddPluginUnified {
        track_id: usize,
        backend: BackendKind,
        uri: String,
        display_name: String,
    },
    RemovePlugin(usize, usize),
    SetPluginBypass(usize, usize, bool),
    SetPluginParam(usize, usize, String, f32),
    MovePlugin(usize, usize, usize),
    LoadPluginPreset(usize, usize, String),
    SavePluginPreset(usize, usize, String),

    SetLoopEnabled(bool),
    SetLoopRegion(f64, f64),

    CreateMidiClip(usize, f64, f64),
    CreateMidiClipWithData(usize, MidiClip),
    UpdateMidiClip(usize, usize, Vec<MidiNote>),
    DeleteMidiClip(usize, usize),
    MoveMidiClip(usize, usize, f64),
    ResizeMidiClip(usize, usize, f64, f64),
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
    UpdateAutomationPoint(usize, usize, f64, f64, f32),
    SetAutomationMode(usize, usize, AutomationMode),
    ClearAutomationLane(usize, usize),

    AddNote(usize, usize, MidiNote),
    RemoveNote(usize, usize, usize),
    UpdateNote(usize, usize, usize, MidiNote),
    PreviewNote(usize, u8),
    StopPreviewNote,

    AddSend(usize, usize, f32),
    RemoveSend(usize, usize),
    SetSendAmount(usize, usize, f32),
    SetSendPreFader(usize, usize, bool),

    CreateGroup(String, Vec<usize>),
    RemoveGroup(usize),
    AddTrackToGroup(usize, usize),
    RemoveTrackFromGroup(usize),

    BeginMidiEdit {
        track_id: usize,
        clip_id: usize,
        session_id: u64,
        base_note_count: usize,
    },
    ApplyMidiNoteDelta {
        track_id: usize,
        clip_id: usize,
        session_id: u64,
        delta: NoteDelta,
    },
    CommitMidiEdit {
        track_id: usize,
        clip_id: usize,
        session_id: u64,
        final_notes: Vec<MidiNote>,
    },

    ReserveNoteIds(usize),
    ToggleClipLoop(usize, usize, bool), // track_id, clip_id, enabled
    MakeClipAlias(usize, usize),        // assign pattern_id, mirror edits
    MakeClipUnique(usize, usize),       // remove pattern_id
    SetClipQuantize(usize, usize, f32, f32, f32, bool), // grid, strength, swing, enabled
    DuplicateMidiClipAsAlias(usize, usize), // track_id, clip_id
    SetClipContentOffset(usize, usize, f64),
    UpdateMidiClipsSameNotes {
        targets: Vec<(usize, usize)>, // (track_id, clip_id)
        notes: Vec<MidiNote>,
    },
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    TrackLevels(Vec<(f32, f32)>),
    RecordingFinished(usize, AudioClip),
    RecordingLevel(f32),
    MasterLevel(f32, f32),
    PushUndo(crate::project::AppStateSnapshot),

    PerformanceMetric {
        cpu_usage: f32,
        buffer_fill: f32,
        xruns: u32,
        plugin_time_ms: f32,
        latency_ms: f32,
    },

    TrackAdded(usize),
    TrackRemoved(usize),
    TrackUpdated(usize),

    ClipAdded(usize, usize),
    ClipRemoved(usize, usize),
    ClipUpdated(usize, usize),

    AutomationUpdated(usize, usize),

    PluginAdded(usize, usize),
    PluginRemoved(usize, usize),
    PluginUpdated(usize, usize),

    Error(String),
    Warning(String),
    Info(String),

    ReservedNoteIds(Vec<u64>),

    PluginParamsDiscovered {
        track_id: usize,
        plugin_idx: usize,
        params: Vec<(String, f32, f32, f32)>,
    },
}
