use serde::{Deserialize, Serialize};

use crate::constants::DEFAULT_MIN_PROJECT_BEATS;

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
    pub velocity_offset: i8,
    pub transpose: i8,
    pub loop_enabled: bool,
    pub muted: bool,
    pub locked: bool,
    pub groove: Option<String>,
    pub swing: f32,
    pub humanize: f32,
}

impl Default for MidiClip {
    fn default() -> Self {
        Self {
            name: "MIDI Clip".to_string(),
            start_beat: 0.0,
            length_beats: DEFAULT_MIN_PROJECT_BEATS,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    pub samples: Vec<f32>,
    pub sample_rate: f32,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
    pub gain: f32,
    pub pitch_shift: f32,
    pub time_stretch: f32,
    pub reverse: bool,
    pub loop_enabled: bool,
    pub color: Option<(u8, u8, u8)>,
    pub muted: bool,
    pub locked: bool,
    pub crossfade_in: Option<f64>,
    pub crossfade_out: Option<f64>,
}

impl Default for AudioClip {
    fn default() -> Self {
        Self {
            name: "Audio Clip".to_string(),
            start_beat: 0.0,
            length_beats: DEFAULT_MIN_PROJECT_BEATS,
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
