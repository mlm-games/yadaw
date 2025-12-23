use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::constants::DEFAULT_LOOP_LEN;
use crate::model::plugin_api::BackendKind;
use crate::model::track::TrackType;

pub struct AtomicF64 {
    storage: AtomicU64,
}

impl AtomicF64 {
    pub fn new(value: f64) -> Self {
        Self {
            storage: AtomicU64::new(value.to_bits()),
        }
    }
    pub fn load(&self) -> f64 {
        f64::from_bits(self.storage.load(Ordering::Relaxed))
    }
    pub fn store(&self, value: f64) {
        self.storage.store(value.to_bits(), Ordering::Relaxed);
    }
}

pub struct AtomicF32 {
    storage: AtomicU32,
}

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self {
            storage: AtomicU32::new(value.to_bits()),
        }
    }
    pub fn load(&self) -> f32 {
        f32::from_bits(self.storage.load(Ordering::Relaxed))
    }
    pub fn store(&self, value: f32) {
        self.storage.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Lock-free audio state that can be safely accessed from audio thread
pub struct AudioState {
    pub playing: Arc<AtomicBool>,
    pub recording: Arc<AtomicBool>,
    pub position: Arc<AtomicF64>,
    pub bpm: Arc<AtomicF32>,
    pub sample_rate: Arc<AtomicF32>,
    pub master_volume: Arc<AtomicF32>,
    pub loop_enabled: Arc<AtomicBool>,
    pub loop_start: Arc<AtomicF64>,
    pub loop_end: Arc<AtomicF64>,

    pub metronome_enabled: Arc<AtomicBool>,
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            playing: Arc::new(AtomicBool::new(false)),
            recording: Arc::new(AtomicBool::new(false)),
            position: Arc::new(AtomicF64::new(0.0)),
            bpm: Arc::new(AtomicF32::new(120.0)),
            sample_rate: Arc::new(AtomicF32::new(44100.0)),
            master_volume: Arc::new(AtomicF32::new(0.8)),
            loop_enabled: Arc::new(AtomicBool::new(true)),
            loop_start: Arc::new(AtomicF64::new(0.0)),
            loop_end: Arc::new(AtomicF64::new(DEFAULT_LOOP_LEN)),

            metronome_enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get_position(&self) -> f64 {
        self.position.load()
    }
    pub fn set_position(&self, pos: f64) {
        self.position.store(pos);
    }
}

/// Immutable snapshot of track data for audio processing
#[derive(Debug, Clone)]
pub struct TrackSnapshot {
    pub track_id: u64,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub monitor_enabled: bool,
    pub audio_clips: Vec<AudioClipSnapshot>,
    pub midi_clips: Vec<MidiClipSnapshot>,
    pub plugin_chain: Vec<PluginDescriptorSnapshot>,
    pub automation_lanes: Vec<RtAutomationLaneSnapshot>,
    pub sends: Vec<crate::model::track::Send>,
    pub track_type: TrackType,
}

#[derive(Debug, Clone)]
pub struct MidiClipSnapshot {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub content_len_beats: f64, // loop source length
    pub loop_enabled: bool,
    pub notes: Vec<MidiNoteSnapshot>,
    pub color: Option<(u8, u8, u8)>,

    pub transpose: i8,
    pub velocity_offset: i8,

    pub quantize_enabled: bool,
    pub quantize_grid: f32,
    pub quantize_strength: f32,
    pub swing: f32,
    pub humanize: f32,

    pub content_offset_beats: f64,
}

#[derive(Debug, Clone)]
pub struct MidiNoteSnapshot {
    pub pitch: u8,
    pub velocity: u8,
    pub start: f64,
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct PluginSnapshot {
    pub uri: String,
    pub bypass: bool,
    pub params: Arc<DashMap<String, f32>>,
}

#[derive(Clone, Debug)]
pub struct PatternSnapshot {
    pub length: f64,
    pub notes: Vec<MidiNoteSnapshot>,
}

/// Commands that require immediate audio state updates
#[derive(Debug)]
pub enum RealtimeCommand {
    UpdateTracks(Vec<TrackSnapshot>),
    UpdateTrackVolume(u64, f32),              // Track ID
    UpdateTrackPan(u64, f32),                 // Track ID
    UpdateTrackMute(u64, bool),               // Track ID
    UpdateTrackSolo(u64, bool),               // Track ID
    UpdatePluginBypass(u64, u64, bool),       // track_id, plugin_id, bypass
    UpdatePluginParam(u64, u64, String, f32), // track_id, plugin_id, param, value
    PreviewNote(u64, u8, f64),                // Track ID
    StopPreviewNote,
    SetLoopEnabled(bool),
    SetLoopRegion(f64, f64),
    AddUnifiedPlugin {
        track_id: u64,
        plugin_id: u64,
        backend: BackendKind,
        uri: String,
    },
    RemovePluginInstance {
        track_id: u64,
        plugin_id: u64,
    },
    UpdateMidiClipNotes {
        track_id: u64, // Track ID
        clip_id: u64,
        notes: Vec<MidiNoteSnapshot>,
    },
    BeginMidiClipEdit {
        track_id: u64, // Track ID
        clip_id: u64,
        session_id: u64,
    },
    PreviewMidiClipNotes {
        track_id: u64, // Track ID
        clip_id: u64,
        session_id: u64,
        notes: Vec<MidiNoteSnapshot>,
    },
    RebuildTrackChain {
        track_id: u64,
        chain: Vec<PluginDescriptorSnapshot>,
    },
    MidiMessage {
        track_id: u64,
        status: u8,
        data1: u8,
        data2: u8,
    },
}

#[derive(Debug, Clone)]
pub struct PluginDescriptorSnapshot {
    pub plugin_id: u64,
    pub uri: String,
    pub name: String,
    pub backend: BackendKind,
    pub bypass: bool,
    pub params: Arc<DashMap<String, f32>>,
}

#[derive(Debug, Clone)]
pub struct RtAutomationLaneSnapshot {
    pub parameter: RtAutomationTarget,
    pub points: Vec<RtAutomationPoint>,
    pub visible: bool,
    pub height: f32,
    pub color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct RtAutomationPoint {
    pub beat: f64,
    pub value: f32,
    pub curve_type: RtCurveType,
}

#[derive(Debug, Clone)]
pub enum RtCurveType {
    Linear,
    Exponential,
    Step,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RtAutomationTarget {
    TrackVolume,
    TrackPan,
    TrackSend(u64), // by id
    PluginParam { plugin_id: u64, param_name: String },
}

#[derive(Debug, Clone)]
pub struct AudioClipSnapshot {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub offset_beats: f64,
    pub samples: Vec<f32>,
    pub sample_rate: f32,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
    pub gain: f32,
}

#[derive(Debug, Clone, Default)]
pub struct AudioGraphSnapshot {
    pub tracks: Vec<TrackSnapshot>,
    pub track_order: Vec<u64>,
}
