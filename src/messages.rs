use serde::{Deserialize, Serialize};

use crate::model::{
    automation::{AutomationMode, AutomationTarget},
    clip::{AudioClip, MidiClip},
    plugin_api::BackendKind,
};

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

    // MIDI clip management (structure, not content)
    CreateMidiClip(usize, f64, f64),
    CreateMidiClipWithData(usize, MidiClip),
    DeleteMidiClip(usize, usize),
    MoveMidiClip(usize, usize, f64),
    ResizeMidiClip(usize, usize, f64, f64),
    DuplicateMidiClip(usize, usize),
    SplitMidiClip(usize, usize, f64),

    // Audio clips
    MoveAudioClip(usize, usize, f64),
    ResizeAudioClip(usize, usize, f64, f64),
    DuplicateAudioClip(usize, usize),
    SplitAudioClip(usize, usize, f64),
    DeleteAudioClip(usize, usize),
    SetAudioClipGain(usize, usize, f32),
    SetAudioClipFadeIn(usize, usize, Option<f64>),
    SetAudioClipFadeOut(usize, usize, Option<f64>),

    // Automation
    AddAutomationPoint(usize, AutomationTarget, f64, f32),
    RemoveAutomationPoint(usize, usize, f64),
    UpdateAutomationPoint(usize, usize, f64, f64, f32),
    SetAutomationMode(usize, usize, AutomationMode),
    ClearAutomationLane(usize, usize),

    // Preview
    PreviewNote(usize, u8),
    StopPreviewNote,

    // Sends/Groups
    AddSend(usize, usize, f32),
    RemoveSend(usize, usize),
    SetSendAmount(usize, usize, f32),
    SetSendPreFader(usize, usize, bool),
    CreateGroup(String, Vec<usize>),
    RemoveGroup(usize),
    AddTrackToGroup(usize, usize),
    RemoveTrackFromGroup(usize),

    // Clip operations
    ToggleClipLoop(usize, usize, bool), // track_id, clip_id, enabled
    MakeClipAlias(usize, usize),        // assign pattern_id, mirror edits
    MakeClipUnique(usize, usize),       // remove pattern_id
    SetClipQuantize(usize, usize, f32, f32, f32, bool), // grid, strength, swing, enabled
    DuplicateMidiClipAsAlias(usize, usize), // track_id, clip_id
    SetClipContentOffset(usize, usize, f64),
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
