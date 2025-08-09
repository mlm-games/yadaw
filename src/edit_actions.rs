use crate::constants::{DEFAULT_GRID_SNAP, NORMALIZE_TARGET_LINEAR};
use crate::state::{AudioClip, MidiNote, Pattern, Track};

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

        let mut first_clip = clip.clone();
        first_clip.length_beats = split_offset;
        first_clip.samples = clip.samples[..split_sample].to_vec();

        let mut second_clip = clip.clone();
        second_clip.name = format!("{} (2)", clip.name);
        second_clip.start_beat = position_beats;
        second_clip.length_beats = clip.length_beats - split_offset;
        second_clip.samples = clip.samples[split_sample..].to_vec();

        Some((first_clip, second_clip))
    }

    pub fn join_clips(clips: Vec<&AudioClip>) -> Option<AudioClip> {
        if clips.is_empty() {
            return None;
        }

        // Sort clips by start position
        let mut sorted_clips = clips.clone();
        sorted_clips.sort_by(|a, b| a.start_beat.partial_cmp(&b.start_beat).unwrap());

        // Check if all clips have the same sample rate
        let sample_rate = sorted_clips[0].sample_rate;
        if !sorted_clips.iter().all(|c| c.sample_rate == sample_rate) {
            return None;
        }

        // Calculate total length and create combined samples
        let start_beat = sorted_clips[0].start_beat;
        let end_beat =
            sorted_clips.last().unwrap().start_beat + sorted_clips.last().unwrap().length_beats;
        let total_beats = end_beat - start_beat;

        let total_samples = ((total_beats * 60.0 / 120.0) * sample_rate as f64) as usize; // Assuming 120 BPM
        let mut combined_samples = vec![0.0; total_samples];

        // Mix clips into combined buffer
        for clip in sorted_clips {
            let offset_beats = clip.start_beat - start_beat;
            let offset_samples = ((offset_beats * 60.0 / 120.0) * sample_rate as f64) as usize;

            for (i, &sample) in clip.samples.iter().enumerate() {
                if offset_samples + i < combined_samples.len() {
                    combined_samples[offset_samples + i] += sample;
                }
            }
        }

        Some(AudioClip {
            name: format!("{} (joined)", clips[0].name),
            start_beat,
            length_beats: total_beats,
            samples: combined_samples,
            sample_rate,
        })
    }

    pub fn apply_fade_in(clip: &mut AudioClip, duration_beats: f64) {
        let fade_samples = ((duration_beats * 60.0 / 120.0) * clip.sample_rate as f64) as usize;
        let fade_samples = fade_samples.min(clip.samples.len());

        for i in 0..fade_samples {
            let fade_factor = i as f32 / fade_samples as f32;
            clip.samples[i] *= fade_factor;
        }
    }

    pub fn apply_fade_out(clip: &mut AudioClip, duration_beats: f64) {
        let fade_samples = ((duration_beats * 60.0 / 120.0) * clip.sample_rate as f64) as usize;
        let fade_samples = fade_samples.min(clip.samples.len());
        let start_idx = clip.samples.len() - fade_samples;

        for i in 0..fade_samples {
            let fade_factor = 1.0 - (i as f32 / fade_samples as f32);
            clip.samples[start_idx + i] *= fade_factor;
        }
    }

    pub fn quantize_notes(notes: &mut Vec<MidiNote>, grid: f64, strength: f32) {
        for note in notes.iter_mut() {
            let quantized_start = (note.start / grid).round() * grid;
            note.start = note.start + (quantized_start - note.start) * strength as f64;
        }

        // Re-sort notes after quantization
        notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    }

    pub fn transpose_notes(notes: &mut Vec<MidiNote>, semitones: i32) {
        for note in notes {
            let new_pitch = (note.pitch as i32 + semitones).clamp(0, 127);
            note.pitch = new_pitch as u8;
        }
    }

    pub fn scale_velocity(notes: &mut Vec<MidiNote>, factor: f32) {
        for note in notes {
            let new_velocity = (note.velocity as f32 * factor).clamp(1.0, 127.0);
            note.velocity = new_velocity as u8;
        }
    }

    pub fn humanize_notes(notes: &mut Vec<MidiNote>, amount: f32) {
        use rand::Rng;
        let mut rng = rand::rng();

        for note in notes {
            // Add random timing variation
            let timing_variation = (rng.random::<f64>() - 0.5) * amount as f64 * 0.1;
            note.start = (note.start + timing_variation).max(0.0);

            // Add random velocity variation
            let velocity_variation = (rng.random::<f32>() - 0.5) * amount * 20.0;
            let new_velocity = (note.velocity as f32 + velocity_variation).clamp(1.0, 127.0);
            note.velocity = new_velocity as u8;
        }
    }

    pub fn duplicate_pattern(pattern: &Pattern) -> Pattern {
        let mut new_pattern = pattern.clone();
        new_pattern.name = format!("{} (copy)", pattern.name);
        new_pattern
    }

    pub fn merge_patterns(patterns: Vec<&Pattern>) -> Pattern {
        let mut merged = Pattern {
            name: "Merged Pattern".to_string(),
            length: patterns.iter().map(|p| p.length).fold(0.0, f64::max),
            notes: Vec::new(),
        };

        for pattern in patterns {
            merged.notes.extend_from_slice(&pattern.notes);
        }

        // Sort and remove duplicate notes
        merged.notes.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap()
                .then(a.pitch.cmp(&b.pitch))
        });

        merged
            .notes
            .dedup_by(|a, b| a.start == b.start && a.pitch == b.pitch);

        merged
    }

    pub fn slice_to_grid(clip: &AudioClip, grid_size: f64, bpm: f32) -> Vec<AudioClip> {
        let mut slices = Vec::new();
        let num_slices = (clip.length_beats / grid_size).ceil() as usize;
        let samples_per_slice =
            ((grid_size * 60.0 / bpm as f64) * clip.sample_rate as f64) as usize;

        for i in 0..num_slices {
            let start_sample = i * samples_per_slice;
            let end_sample = ((i + 1) * samples_per_slice).min(clip.samples.len());

            if start_sample < clip.samples.len() {
                let slice = AudioClip {
                    name: format!("{} (slice {})", clip.name, i + 1),
                    start_beat: clip.start_beat + (i as f64 * grid_size),
                    length_beats: grid_size,
                    samples: clip.samples[start_sample..end_sample].to_vec(),
                    sample_rate: clip.sample_rate,
                };
                slices.push(slice);
            }
        }

        slices
    }

    pub fn time_stretch(clip: &AudioClip, factor: f32) -> AudioClip {
        // Simple linear interpolation time stretch
        // For production, you'd want to use a proper algorithm like WSOLA or phase vocoder
        let new_length = (clip.samples.len() as f32 * factor) as usize;
        let mut stretched = Vec::with_capacity(new_length);

        for i in 0..new_length {
            let source_pos = i as f32 / factor;
            let idx = source_pos.floor() as usize;
            let frac = source_pos.fract();

            if idx + 1 < clip.samples.len() {
                let sample = clip.samples[idx] * (1.0 - frac) + clip.samples[idx + 1] * frac;
                stretched.push(sample);
            } else if idx < clip.samples.len() {
                stretched.push(clip.samples[idx]);
            } else {
                stretched.push(0.0);
            }
        }

        AudioClip {
            name: format!("{} (stretched)", clip.name),
            start_beat: clip.start_beat,
            length_beats: clip.length_beats * factor as f64,
            samples: stretched,
            sample_rate: clip.sample_rate,
        }
    }
}
