/// Time conversion utilities for the DAW
pub struct TimeConverter {
    sample_rate: f32,
    bpm: f32,
}

impl TimeConverter {
    pub fn new(sample_rate: f32, bpm: f32) -> Self {
        Self { sample_rate, bpm }
    }

    /// Convert sample position to beats
    #[inline]
    pub fn samples_to_beats(&self, samples: f64) -> f64 {
        (samples / self.sample_rate as f64) * (self.bpm as f64 / 60.0)
    }

    /// Convert beats to sample position
    #[inline]
    pub fn beats_to_samples(&self, beats: f64) -> f64 {
        (beats * 60.0 / self.bpm as f64) * self.sample_rate as f64
    }

    /// Convert seconds to beats
    #[inline]
    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        seconds * (self.bpm as f64 / 60.0)
    }

    /// Convert beats to seconds
    #[inline]
    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        beats * 60.0 / self.bpm as f64
    }

    /// Convert samples to seconds
    #[inline]
    pub fn samples_to_seconds(&self, samples: f64) -> f64 {
        samples / self.sample_rate as f64
    }

    /// Convert seconds to samples
    #[inline]
    pub fn seconds_to_samples(&self, seconds: f64) -> f64 {
        seconds * self.sample_rate as f64
    }

    /// Convert microseconds to beats (for MIDI timing)
    #[inline]
    pub fn microseconds_to_beats(&self, microseconds: u64) -> f64 {
        let seconds = microseconds as f64 / 1_000_000.0;
        self.seconds_to_beats(seconds)
    }

    /// Update BPM (for tempo changes)
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
    }

    /// Update sample rate (rarely needed)
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }
}

/// Format time in bars:beats:sixteenths
pub fn format_bars_beats_sixteenths(beats: f64, beats_per_bar: u32) -> String {
    let bars = (beats / beats_per_bar as f64) as i32 + 1;
    let beat = (beats % beats_per_bar as f64) as i32 + 1;
    let sixteenth = ((beats % 1.0) * 4.0) as i32 + 1;
    format!("{:03}:{:02}:{:02}", bars, beat, sixteenth)
}

/// Format time in minutes:seconds.milliseconds
pub fn format_minutes_seconds(seconds: f64) -> String {
    let minutes = (seconds / 60.0) as i32;
    let secs = seconds % 60.0;
    format!("{:02}:{:06.3}", minutes, secs)
}

/// Quantize a beat position to the nearest grid point
#[inline]
pub fn quantize_to_grid(beat: f64, grid_size: f64) -> f64 {
    if grid_size > 0.0 {
        (beat / grid_size).round() * grid_size
    } else {
        beat
    }
}

/// Get the pattern position for a looping pattern
#[inline]
pub fn get_pattern_position(global_beat: f64, pattern_length: f64) -> f64 {
    if pattern_length > 0.0 {
        global_beat % pattern_length
    } else {
        global_beat
    }
}

/// Static convenience functions for common conversions
pub mod quick {
    /// Quick conversion without creating a converter
    #[inline]
    pub fn samples_to_beats(samples: f64, sample_rate: f32, bpm: f32) -> f64 {
        (samples / sample_rate as f64) * (bpm as f64 / 60.0)
    }

    #[inline]
    pub fn beats_to_samples(beats: f64, sample_rate: f32, bpm: f32) -> f64 {
        (beats * 60.0 / bpm as f64) * sample_rate as f64
    }
}
