use crossbeam::atomic::AtomicCell;
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Lock-free audio state that can be safely accessed from audio thread
pub struct AudioState {
    pub playing: AtomicBool,
    pub recording: AtomicBool,
    pub current_position: AtomicU64, // Store as u64, convert to f64
    pub master_volume: AtomicCell<f32>,
    pub bpm: AtomicCell<f32>,
    pub sample_rate: AtomicCell<f32>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            current_position: AtomicU64::new(0),
            master_volume: AtomicCell::new(0.8),
            bpm: AtomicCell::new(120.0),
            sample_rate: AtomicCell::new(44100.0),
        }
    }

    pub fn get_position(&self) -> f64 {
        f64::from_bits(self.current_position.load(Ordering::Relaxed))
    }

    pub fn set_position(&self, pos: f64) {
        self.current_position
            .store(pos.to_bits(), Ordering::Relaxed);
    }
}

/// Immutable snapshot of track data for audio processing
#[derive(Clone, Debug)]
pub struct TrackSnapshot {
    pub id: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub plugin_chain: Vec<PluginSnapshot>,
    pub patterns: Vec<PatternSnapshot>,
    pub is_midi: bool,
    pub audio_clips: Vec<Arc<AudioClipSnapshot>>,
    pub(crate) armed: bool,
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

#[derive(Clone, Copy, Debug)]
pub struct MidiNoteSnapshot {
    pub pitch: u8,
    pub velocity: u8,
    pub start: f64,
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct AudioClipSnapshot {
    pub start_beat: f64,
    pub length_beats: f64,
    pub samples: Arc<Vec<f32>>,
    pub sample_rate: f32,
}

/// Commands that require immediate audio state updates
#[derive(Debug, Clone)]
pub enum RealtimeCommand {
    UpdateTracks(Vec<TrackSnapshot>),
    UpdateTrackVolume(usize, f32),
    UpdateTrackPan(usize, f32),
    UpdateTrackMute(usize, bool),
    UpdateTrackSolo(usize, bool),
    UpdatePluginBypass(usize, usize, bool),
    UpdatePluginParam(usize, usize, String, f32),
    PreviewNote(usize, u8, f64), // track_id, pitch, start_position
    StopPreviewNote,
}
