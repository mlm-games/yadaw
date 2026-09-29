use serde::{Deserialize, Serialize};

use crate::constants::DEFAULT_BPM;

/// One anchor of the project's tempo curve. `beat` is a project beat and `bpm`
/// is the tempo in force from that beat until the next point. With `hold` the
/// tempo stays put and jumps at the next point, which is DAWproject's second
/// interpolation mode; the default ramps linearly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TempoPoint {
    pub beat: f64,
    pub bpm: f64,
    #[serde(default)]
    pub hold: bool,
}

impl TempoPoint {
    pub fn new(beat: f64, bpm: f64) -> Self {
        Self {
            beat,
            bpm,
            hold: false,
        }
    }

    pub fn held(beat: f64, bpm: f64) -> Self {
        Self {
            beat,
            bpm,
            hold: true,
        }
    }
}

/// One span of the tempo curve with the time at its start already accumulated,
/// so a lookup is a binary search plus a closed-form term.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoSegment {
    pub beat: f64,
    pub bpm: f64,
    pub seconds_at_beat: f64,
    /// The tempo holds at `bpm` for this whole span instead of ramping.
    pub hold: bool,
}

/// The project's beat <-> time mapping. Between two points the tempo ramps
/// linearly and the elapsed time is the exact integral of that ramp, so a map
/// with a single point reproduces a constant tempo exactly. The constant case
/// is its own variant so the audio thread can build one without allocating.
#[derive(Debug, Clone, PartialEq)]
pub enum TempoCurve {
    Constant { bpm: f64 },
    Mapped { segments: Vec<TempoSegment> },
}

impl Default for TempoCurve {
    fn default() -> Self {
        Self::Constant {
            bpm: f64::from(DEFAULT_BPM),
        }
    }
}

impl TempoCurve {
    /// Builds the curve from a tempo map. An empty map means a constant tempo,
    /// which is also how every project written before tempo maps existed reads.
    pub fn from_map(map: &[TempoPoint], fallback_bpm: f64) -> Self {
        let bpm = sanitise_bpm(fallback_bpm);
        let mut points: Vec<TempoPoint> = map
            .iter()
            .copied()
            .filter(|p| p.beat.is_finite() && p.bpm.is_finite() && p.bpm > 0.0)
            .collect();
        if points.is_empty() {
            return Self::constant(bpm);
        }
        points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        points.dedup_by(|a, b| a.beat == b.beat);
        if points.len() == 1 && points[0].beat.abs() <= f64::EPSILON {
            return Self::constant(points[0].bpm);
        }
        if points[0].beat > 0.0 {
            points.insert(0, TempoPoint::new(0.0, bpm));
        }

        let mut segments = Vec::with_capacity(points.len());
        let mut seconds = 0.0;
        segments.push(TempoSegment {
            beat: points[0].beat,
            bpm: points[0].bpm,
            seconds_at_beat: 0.0,
            hold: points[0].hold,
        });
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            seconds += span_seconds(a, b);
            segments.push(TempoSegment {
                beat: b.beat,
                bpm: b.bpm,
                seconds_at_beat: seconds,
                hold: b.hold,
            });
        }
        Self::Mapped { segments }
    }

    pub fn constant(bpm: f64) -> Self {
        Self::Constant {
            bpm: sanitise_bpm(bpm),
        }
    }

    pub fn is_constant(&self) -> bool {
        matches!(self, Self::Constant { .. })
    }

    pub fn constant_bpm(&self) -> f64 {
        match self {
            Self::Constant { bpm } => *bpm,
            Self::Mapped { segments } => segments.first().map_or(f64::from(DEFAULT_BPM), |s| s.bpm),
        }
    }

    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        match self {
            Self::Constant { bpm } => {
                if beats.is_finite() {
                    beats * 60.0 / bpm
                } else {
                    0.0
                }
            }
            Self::Mapped { segments } => map_beats_to_seconds(segments, beats),
        }
    }

    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        match self {
            Self::Constant { bpm } => {
                // Divided first, then multiplied, which is the association the
                // engine used before tempo maps existed. Keeping it means a
                // constant-tempo project converts bit for bit as it did.
                if seconds.is_finite() {
                    seconds * (bpm / 60.0)
                } else {
                    0.0
                }
            }
            Self::Mapped { segments } => map_seconds_to_beats(segments, seconds),
        }
    }

    /// Tempo in force at a beat, needed for tempo-synced plugin hosts.
    pub fn bpm_at(&self, beats: f64) -> f64 {
        match self {
            Self::Constant { bpm } => *bpm,
            Self::Mapped { segments } => {
                if !beats.is_finite() || segments.is_empty() {
                    return segments.first().map_or(f64::from(DEFAULT_BPM), |s| s.bpm);
                }
                map_bpm_at(segments, beats)
            }
        }
    }
}

fn map_beats_to_seconds(segments: &[TempoSegment], beats: f64) -> f64 {
    if !beats.is_finite() || segments.is_empty() {
        return 0.0;
    }
    let first = segments[0];
    if beats <= first.beat {
        return first.seconds_at_beat + (beats - first.beat) * 60.0 / first.bpm;
    }
    let last = segments[segments.len() - 1];
    if beats >= last.beat {
        return last.seconds_at_beat + (beats - last.beat) * 60.0 / last.bpm;
    }
    let index = segment_for(segments, beats);
    let segment = &segments[index];
    if segment.hold {
        return segment.seconds_at_beat + (beats - segment.beat) * 60.0 / segment.bpm;
    }
    let next = &segments[index + 1];
    let k = ramp_slope(segment.bpm, next.bpm, next.beat - segment.beat);
    segment.seconds_at_beat + ramp_seconds(k, segment.bpm, beats - segment.beat)
}

fn map_seconds_to_beats(segments: &[TempoSegment], seconds: f64) -> f64 {
    if !seconds.is_finite() || segments.is_empty() {
        return 0.0;
    }
    let first = segments[0];
    if seconds <= first.seconds_at_beat {
        return first.beat + (seconds - first.seconds_at_beat) * first.bpm / 60.0;
    }
    let last = segments[segments.len() - 1];
    if seconds >= last.seconds_at_beat {
        return last.beat + (seconds - last.seconds_at_beat) * last.bpm / 60.0;
    }
    let mut index = 0usize;
    while segments[index + 1].seconds_at_beat <= seconds {
        index += 1;
    }
    let segment = &segments[index];
    let next = &segments[index + 1];
    let span = next.beat - segment.beat;
    let elapsed = seconds - segment.seconds_at_beat;
    if segment.hold {
        return segment.beat + elapsed * segment.bpm / 60.0;
    }
    let k = ramp_slope(segment.bpm, next.bpm, span);
    segment.beat + ramp_beats(k, segment.bpm, span, elapsed)
}

fn map_bpm_at(segments: &[TempoSegment], beats: f64) -> f64 {
    let first = segments[0];
    if beats <= first.beat {
        return first.bpm;
    }
    let last = segments[segments.len() - 1];
    if beats >= last.beat {
        return last.bpm;
    }
    let index = segment_for(segments, beats);
    let segment = &segments[index];
    if segment.hold {
        return segment.bpm;
    }
    let next = &segments[index + 1];
    if (next.beat - segment.beat).abs() <= f64::EPSILON {
        return next.bpm;
    }
    let t = (beats - segment.beat) / (next.beat - segment.beat);
    segment.bpm + (next.bpm - segment.bpm) * t
}

/// Index of the segment whose span contains `beats`; never the last entry, so
/// the caller can always look at `index + 1`.
fn segment_for(segments: &[TempoSegment], beats: f64) -> usize {
    let last = segments.len() - 1;
    let mut lo = 0usize;
    let mut hi = last;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if segments[mid].beat <= beats {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

pub fn sanitise_bpm(bpm: f64) -> f64 {
    if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        f64::from(DEFAULT_BPM)
    }
}

/// Seconds to cross the span between two points, honouring whether it ramps or holds.
fn span_seconds(from: TempoPoint, to: TempoPoint) -> f64 {
    let span = to.beat - from.beat;
    if span <= 0.0 {
        return 0.0;
    }
    if from.hold {
        return span * 60.0 / from.bpm;
    }
    ramp_seconds(ramp_slope(from.bpm, to.bpm, span), from.bpm, span)
}

/// Tempo ramp slope in BPM per beat, taken over the segment's whole span.
fn ramp_slope(from_bpm: f64, to_bpm: f64, span: f64) -> f64 {
    if span.abs() <= f64::EPSILON {
        0.0
    } else {
        (to_bpm - from_bpm) / span
    }
}

/// Seconds to travel `along` beats into a ramp of slope `k` BPM per beat. This
/// is the exact integral of `60 / bpm(x)` for a linearly rising tempo.
fn ramp_seconds(k: f64, from_bpm: f64, along: f64) -> f64 {
    if k.abs() <= f64::EPSILON {
        return along * 60.0 / from_bpm;
    }
    let to_bpm = (from_bpm + k * along).max(f64::MIN_POSITIVE);
    (60.0 / k) * (to_bpm / from_bpm).ln()
}

/// The inverse of `ramp_seconds`: beats covered by `elapsed` seconds.
fn ramp_beats(k: f64, from_bpm: f64, span: f64, elapsed: f64) -> f64 {
    if k.abs() <= f64::EPSILON {
        return (elapsed * from_bpm / 60.0).clamp(0.0, span);
    }
    let bpm = from_bpm * (elapsed * k / 60.0).exp();
    ((bpm - from_bpm) / k).clamp(0.0, span.max(0.0))
}
