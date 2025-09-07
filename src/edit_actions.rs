use crate::constants::{DEFAULT_GRID_SNAP, NORMALIZE_TARGET_LINEAR};
use crate::model::{AudioClip, MidiNote, Track};

#[derive(Debug, Clone)]
pub enum EditAction {
    // Clip operations
    SplitClip {
        track_id: usize,
        clip_id: usize,
        position: f64,
    },
    JoinClips {
        track_id: usize,
        clip_ids: Vec<usize>,
    },
    DuplicateClip {
        track_id: usize,
        clip_id: usize,
    },
    TrimClipStart {
        track_id: usize,
        clip_id: usize,
        amount: f64,
    },
    TrimClipEnd {
        track_id: usize,
        clip_id: usize,
        amount: f64,
    },
    FadeIn {
        track_id: usize,
        clip_id: usize,
        duration: f64,
    },
    FadeOut {
        track_id: usize,
        clip_id: usize,
        duration: f64,
    },
    Crossfade {
        track_id: usize,
        clip1_id: usize,
        clip2_id: usize,
        duration: f64,
    },

    // MIDI operations
    Quantize {
        track_id: usize,
        pattern_id: usize,
        strength: f32,
    },
    Transpose {
        track_id: usize,
        pattern_id: usize,
        semitones: i32,
    },
    ScaleVelocity {
        track_id: usize,
        pattern_id: usize,
        factor: f32,
    },
    Humanize {
        track_id: usize,
        pattern_id: usize,
        amount: f32,
    },

    // Track operations
    DuplicateTrack {
        track_id: usize,
        include_content: bool,
    },
    MergeTrack {
        source_id: usize,
        dest_id: usize,
    },
    BounceTrack {
        track_id: usize,
        include_effects: bool,
    },
    FreezeTrack {
        track_id: usize,
    },

    // Range operations
    SelectRange {
        start: f64,
        end: f64,
        track_ids: Vec<usize>,
    },
    CutRange {
        start: f64,
        end: f64,
    },
    CopyRange {
        start: f64,
        end: f64,
    },
    PasteRange {
        position: f64,
    },
    DeleteRange {
        start: f64,
        end: f64,
    },
    InsertTime {
        position: f64,
        duration: f64,
    },
    RemoveTime {
        start: f64,
        end: f64,
    },
}

pub struct EditProcessor;

impl EditProcessor {
    pub fn split_clip(
        clip: &AudioClip,
        position_beats: f64,
        bpm: f32,
    ) -> Option<(AudioClip, AudioClip)> {
        if position_beats <= clip.start_beat
            || position_beats >= clip.start_beat + clip.length_beats
        {
            return None;
        }
        let split_offset = position_beats - clip.start_beat;
        let split_sample = ((split_offset * 60.0 / bpm as f64) * clip.sample_rate as f64) as usize;
        if split_sample >= clip.samples.len() {
            return None;
        }
        let mut first = clip.clone();
        first.length_beats = split_offset;
        first.samples = clip.samples[..split_sample].to_vec();

        let mut second = clip.clone();
        second.name = format!("{} (2)", clip.name);
        second.start_beat = position_beats;
        second.length_beats = clip.length_beats - split_offset;
        second.samples = clip.samples[split_sample..].to_vec();
        Some((first, second))
    }

    pub fn apply_fade_in(clip: &mut AudioClip, duration_beats: f64, bpm: f32) {
        let fade_samples = ((duration_beats * 60.0 / bpm as f64) * clip.sample_rate as f64)
            .round()
            .clamp(0.0, clip.samples.len() as f64) as usize;
        for i in 0..fade_samples {
            let f = i as f32 / fade_samples.max(1) as f32;
            clip.samples[i] *= f;
        }
    }

    pub fn apply_fade_out(clip: &mut AudioClip, duration_beats: f64, bpm: f32) {
        let fade_samples = ((duration_beats * 60.0 / bpm as f64) * clip.sample_rate as f64)
            .round()
            .clamp(0.0, clip.samples.len() as f64) as usize;
        let start = clip.samples.len().saturating_sub(fade_samples);
        for i in 0..fade_samples {
            let f = 1.0 - (i as f32 / fade_samples.max(1) as f32);
            clip.samples[start + i] *= f;
        }
    }

    pub fn quantize_notes(notes: &mut Vec<MidiNote>, grid: f64, strength: f32) {
        for n in notes.iter_mut() {
            let q = (n.start / grid).round() * grid;
            n.start = n.start + (q - n.start) * strength as f64;
        }
        notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    }

    pub fn transpose_notes(notes: &mut Vec<MidiNote>, semitones: i32) {
        for n in notes.iter_mut() {
            n.pitch = (n.pitch as i32 + semitones).clamp(0, 127) as u8;
        }
    }

    pub fn humanize_notes(notes: &mut Vec<MidiNote>, amount: f32) {
        use rand::Rng;
        let mut rng = rand::rng();
        for n in notes.iter_mut() {
            let dt = (rng.random::<f64>() - 0.5) * amount as f64 * 0.1;
            n.start = (n.start + dt).max(0.0);
            let dv = (rng.random::<f32>() - 0.5) * amount * 20.0;
            let v = (n.velocity as f32 + dv).clamp(1.0, 127.0);
            n.velocity = v as u8;
        }
    }
}
