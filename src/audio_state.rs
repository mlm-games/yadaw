use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::constants::DEFAULT_MIN_PROJECT_BEATS;
use crate::lv2_plugin_host::LV2PluginInstance;
use crate::model::PluginDescriptor;

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
            loop_end: Arc::new(AtomicF64::new(4.0)),
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
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub is_midi: bool,
    pub monitor_enabled: bool,
    pub audio_clips: Vec<AudioClipSnapshot>,
    pub midi_clips: Vec<MidiClipSnapshot>,
    pub plugin_chain: Vec<PluginDescriptorSnapshot>,
    pub automation_lanes: Vec<RtAutomationLaneSnapshot>,
}

#[derive(Debug, Clone)]
pub struct MidiClipSnapshot {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub notes: Vec<MidiNoteSnapshot>,
    pub color: Option<(u8, u8, u8)>,
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
    UpdateTrackVolume(usize, f32),
    UpdateTrackPan(usize, f32),
    UpdateTrackMute(usize, bool),
    UpdateTrackSolo(usize, bool),
    UpdatePluginBypass(usize, usize, bool),
    UpdatePluginParam(usize, usize, String, f32),
    PreviewNote(usize, u8, f64),
    StopPreviewNote,
    SetLoopEnabled(bool),
    SetLoopRegion(f64, f64),
    AddPluginInstance {
        track_id: usize,
        plugin_idx: usize,
        instance: LV2PluginInstance,
        descriptor: Arc<DashMap<String, f32>>, // Plugin params
        uri: String,
        bypass: bool,
    },
    RemovePluginInstance {
        track_id: usize,
        plugin_idx: usize,
    },
    UpdateMidiClipNotes {
        track_id: usize,
        clip_id: usize,
        notes: Vec<MidiNoteSnapshot>,
    },

    BeginMidiClipEdit {
        track_id: usize,
        clip_id: usize,
        session_id: u64,
    },

    PreviewMidiClipNotes {
        track_id: usize,
        clip_id: usize,
        session_id: u64,
        notes: Vec<MidiNoteSnapshot>,
    },
}

#[derive(Debug, Clone)]
pub struct PluginDescriptorSnapshot {
    pub uri: String,
    pub name: String,
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
    TrackSend(usize),
    PluginParam {
        plugin_idx: usize,
        param_name: String,
    },
}

#[derive(Debug, Clone)]
pub struct AudioClipSnapshot {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub samples: Vec<f32>,
    pub sample_rate: f32,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
    pub gain: f32,
}
