use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    audio_export::ExportConfig,
    model::{
        MidiNote,
        automation::{AutomationMode, AutomationTarget},
        clip::{AudioClip, MidiClip},
        plugin_api::BackendKind,
    },
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

    SetTrackVolume(u64, f32),
    SetTrackPan(u64, f32),
    SetTrackMute(u64, bool),
    SetTrackSolo(u64, bool),
    SetTrackArmed(u64, bool),
    SetTrackInput(u64, Option<String>),
    SetTrackOutput(u64, Option<String>),
    SetTrackMonitor(u64, bool),
    FreezeTrack(u64),
    UnfreezeTrack(u64),

    AddPluginUnified {
        track_id: u64,
        plugin_idx: usize, // for ordering
        backend: BackendKind,
        uri: String,
        display_name: String,
    },

    RemovePlugin(u64, u64),
    SetPluginBypass(u64, u64, bool),
    SetPluginParam(u64, u64, String, f32),
    MovePlugin(u64, usize, usize),
    LoadPluginPreset(u64, usize, String),
    SavePluginPreset(u64, usize, String),

    SetLoopEnabled(bool),
    SetLoopRegion(f64, f64),

    CreateMidiClip {
        track_id: u64,
        start_beat: f64,
        length_beats: f64,
    },
    CreateMidiClipWithData {
        track_id: u64,
        clip: MidiClip,
    },
    DeleteMidiClip {
        clip_id: u64,
    },
    MoveMidiClip {
        clip_id: u64,
        new_start: f64,
    },
    ResizeMidiClip {
        clip_id: u64,
        new_start: f64,
        new_length: f64,
    },
    DuplicateMidiClip {
        clip_id: u64,
    },
    SplitMidiClip {
        clip_id: u64,
        position: f64,
    },

    MoveAudioClip {
        clip_id: u64,
        new_start: f64,
    },
    ResizeAudioClip {
        clip_id: u64,
        new_start: f64,
        new_length: f64,
    },
    DuplicateAudioClip {
        clip_id: u64,
    },
    SplitAudioClip {
        clip_id: u64,
        position: f64,
    },
    DeleteAudioClip {
        clip_id: u64,
    },
    SetAudioClipGain(u64, f32),
    SetAudioClipFadeIn(u64, Option<f64>),
    SetAudioClipFadeOut(u64, Option<f64>),

    // Automation (track ID + lane index)
    AddAutomationPoint(u64, AutomationTarget, f64, f32),
    RemoveAutomationPoint(u64, usize, f64),
    UpdateAutomationPoint {
        track_id: u64,
        lane_idx: usize,
        old_beat: f64,
        new_beat: f64,
        new_value: f32,
    },
    SetAutomationMode(u64, usize, AutomationMode),
    ClearAutomationLane(u64, usize),

    // Preview (track ID)
    PreviewNote(u64, u8),
    StopPreviewNote,

    // Sends/Groups (track IDs)
    AddSend(u64, u64, f32), // source, destination, amount
    RemoveSend(u64, usize),
    SetSendAmount(u64, usize, f32),
    SetSendPreFader(u64, usize, bool),
    CreateGroup(String, Vec<u64>),
    RemoveGroup(usize),
    AddTrackToGroup(u64, usize),
    RemoveTrackFromGroup(u64),

    ToggleClipLoop {
        clip_id: u64,
        enabled: bool,
    },
    MakeClipAlias {
        clip_id: u64,
    },
    MakeClipUnique {
        clip_id: u64,
    },
    SetClipQuantize {
        clip_id: u64,
        grid: f32,
        strength: f32,
        swing: f32,
        enabled: bool,
    },
    DuplicateMidiClipAsAlias {
        clip_id: u64,
    },
    SetClipContentOffset {
        clip_id: u64,
        new_offset: f64,
    },
    CutSelectedNotes {
        clip_id: u64,
        note_ids: Vec<u64>,
    },
    PasteNotes {
        clip_id: u64,
        notes: Vec<MidiNote>,
    },
    DeleteSelectedNotes {
        clip_id: u64,
        note_ids: Vec<u64>,
    },
    ExportAudio(ExportConfig),
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    TrackLevels(HashMap<u64, (f32, f32)>), // indexed for meters
    RecordingFinished(u64, AudioClip),     // Track ID
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

    TrackAdded(u64),
    TrackRemoved(u64),
    TrackUpdated(u64),

    ClipAdded(u64),   // Clip ID
    ClipRemoved(u64), // Clip ID
    ClipUpdated(u64), // Clip ID

    AutomationUpdated(u64, usize), // Track ID, lane index

    PluginAdded(u64, usize),   // Track ID, plugin index
    PluginRemoved(u64, usize), // Track ID, plugin index
    PluginUpdated(u64, usize), // Track ID, plugin index

    Error(String),
    Warning(String),
    Info(String),

    ReservedNoteIds(Vec<u64>),

    PluginParamsDiscovered {
        track_id: u64,
        plugin_idx: usize,
        params: Vec<(String, f32, f32, f32)>,
    },
    NotesCutToClipboard(Vec<MidiNote>),
    ExportProgress(f32),
}
