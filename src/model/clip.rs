use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::constants::DEFAULT_MIN_PROJECT_BEATS;

#[inline]
fn zero_u64() -> u64 {
    0
}

#[inline]
fn default_zero_f64() -> f64 {
    0.0
}

#[inline]
fn default_quantize_grid() -> f32 {
    0.25
} // 1/4 beat = 16th notes

#[inline]
fn default_false() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiPattern {
    pub id: u64,
    pub notes: Vec<MidiNote>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MidiNote {
    #[serde(default = "zero_u64")]
    pub id: u64,
    pub pitch: u8,
    pub velocity: u8,
    pub start: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiClip {
    #[serde(default = "zero_u64")]
    pub id: u64,
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64, // instance length (can extend when looping)
    pub notes: Vec<MidiNote>,
    pub color: Option<(u8, u8, u8)>,
    pub velocity_offset: i8,
    pub transpose: i8,
    #[serde(default = "default_false")]
    pub loop_enabled: bool,

    #[serde(default = "default_zero_f64")]
    pub content_len_beats: f64,

    #[serde(default)]
    pub pattern_id: Option<u64>,

    #[serde(default = "default_quantize_grid")]
    pub quantize_grid: f32,
    #[serde(default)]
    pub quantize_strength: f32, // 0..1
    #[serde(default = "default_false")]
    pub quantize_enabled: bool,

    pub muted: bool,
    pub locked: bool,
    pub groove: Option<String>,
    pub swing: f32,
    pub humanize: f32,

    #[serde(default = "default_zero_f64")]
    pub content_offset_beats: f64,
}

impl Default for MidiClip {
    fn default() -> Self {
        let length = DEFAULT_MIN_PROJECT_BEATS;
        Self {
            id: 0,
            name: "MIDI Clip".to_string(),
            start_beat: 0.0,
            length_beats: length,
            notes: Vec::new(),
            color: None,
            velocity_offset: 0,
            transpose: 0,
            loop_enabled: false,
            content_len_beats: length,
            pattern_id: None,
            quantize_grid: default_quantize_grid(),
            quantize_strength: 1.0,
            quantize_enabled: false,
            muted: false,
            locked: false,
            groove: None,
            swing: 0.0,
            humanize: 0.0,
            content_offset_beats: 0.0,
        }
    }
}

fn default_opt_u64_none() -> Option<u64> {
    None
}

/// One anchor of an audio clip's beat-to-content time map. `beat` is a
/// clip-local beat, `content_seconds` is how far into the source material that
/// beat reads.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WarpPoint {
    pub beat: f64,
    pub content_seconds: f64,
}

/// A resolved clip-local beat -> content-seconds map. The two-point form is the
/// common case and stays a pair of scalars so the audio thread never allocates.
#[derive(Debug, Clone, Copy)]
pub enum WarpCurve<'a> {
    Linear { origin: f64, slope: f64 },
    Points(&'a [WarpPoint]),
}

impl WarpCurve<'_> {
    /// Content time to read at a clip-local beat, clamped to the curve's ends.
    #[inline]
    pub fn content_seconds_at(&self, beat: f64) -> f64 {
        match self {
            Self::Linear { origin, slope } => origin + beat * slope,
            Self::Points(points) => interp_warp_points(points, beat),
        }
    }
}

fn interp_warp_points(points: &[WarpPoint], beat: f64) -> f64 {
    let first = points[0];
    if beat <= first.beat {
        return first.content_seconds;
    }
    let last = points[points.len() - 1];
    if beat >= last.beat {
        return last.content_seconds;
    }
    let mut lo = 0usize;
    let mut hi = points.len() - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if points[mid].beat <= beat {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let a = points[lo];
    let b = points[hi];
    let span = b.beat - a.beat;
    if span <= f64::EPSILON {
        return b.content_seconds;
    }
    let t = (beat - a.beat) / span;
    a.content_seconds + (b.content_seconds - a.content_seconds) * t
}

/// Resolve a clip's warp map. `warp_mode` is the master switch: when it is off
/// the clip reads at natural speed and any stored points are ignored. When it is
/// on, two or more stored points are used verbatim, and otherwise the map is
/// synthesised so the whole source is stretched to fill `length_beats`.
pub fn resolve_warp<'a>(
    warps: &'a [WarpPoint],
    warp_mode: bool,
    length_beats: f64,
    source_seconds: f64,
    bpm: f64,
) -> WarpCurve<'a> {
    if !warp_mode {
        return WarpCurve::Linear {
            origin: 0.0,
            slope: 60.0 / bpm,
        };
    }
    if warps.len() >= 2 {
        return WarpCurve::Points(warps);
    }
    WarpCurve::Linear {
        origin: 0.0,
        slope: if length_beats > f64::EPSILON {
            source_seconds / length_beats
        } else {
            source_seconds
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClip {
    #[serde(default = "zero_u64")]
    pub id: u64,
    pub name: String,
    pub start_beat: f64,
    pub length_beats: f64,
    #[serde(default = "default_zero_f64")]
    pub offset_beats: f64,
    pub samples: Arc<Vec<f32>>,
    pub sample_rate: f32,
    #[serde(default = "default_opt_u64_none")]
    pub source_hash: Option<u64>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
    pub gain: f32,
    pub pitch_shift: f32,
    #[serde(default = "default_false")]
    pub warp_mode: bool,
    #[serde(default)]
    pub warps: Vec<WarpPoint>,
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
            id: 0,
            name: "Audio Clip".to_string(),
            start_beat: 0.0,
            length_beats: DEFAULT_MIN_PROJECT_BEATS,
            offset_beats: 0.0,
            samples: Arc::new(Vec::new()),
            sample_rate: 44100.0,
            source_hash: None,
            fade_in: None,
            fade_out: None,
            gain: 1.0,
            pitch_shift: 0.0,
            warp_mode: false,
            warps: Vec::new(),
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

impl AudioClip {
    #[inline]
    pub fn source_seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }

    pub fn warp_curve(&self, bpm: f64) -> WarpCurve<'_> {
        resolve_warp(
            &self.warps,
            self.warp_mode,
            self.length_beats,
            self.source_seconds(),
            bpm,
        )
    }
}
