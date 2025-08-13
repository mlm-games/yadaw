/// MIDI note utilities and conversions
pub struct MidiNoteUtils;

impl MidiNoteUtils {
    /// Convert MIDI note number to frequency in Hz
    /// A4 (MIDI note 69) = 440 Hz
    #[inline]
    pub fn to_frequency(pitch: u8) -> f32 {
        440.0 * 2.0_f32.powf((pitch as f32 - 69.0) / 12.0)
    }

    /// Convert frequency in Hz to nearest MIDI note number
    #[inline]
    pub fn from_frequency(freq: f32) -> u8 {
        (69.0 + 12.0 * (freq / 440.0).log2())
            .round()
            .clamp(0.0, 127.0) as u8
    }

    /// Get note name from MIDI note number (e.g., 60 -> "C4")
    pub fn to_name(pitch: u8) -> String {
        const NOTE_NAMES: &[&str] = &[
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let octave = (pitch / 12) as i32 - 1;
        let note = NOTE_NAMES[(pitch % 12) as usize];
        format!("{}{}", note, octave)
    }

    /// Parse note name to MIDI note number (e.g., "C4" -> 60)
    pub fn from_name(name: &str) -> Option<u8> {
        let name = name.trim().to_uppercase();

        // Extract note and octave
        let (note_part, octave_part) = if name.contains('#') {
            let idx = name.find('#')? + 1;
            (&name[..idx], &name[idx..])
        } else {
            (&name[..1], &name[1..])
        };

        let note_offset = match note_part {
            "C" => 0,
            "C#" | "CS" => 1,
            "D" => 2,
            "D#" | "DS" => 3,
            "E" => 4,
            "F" => 5,
            "F#" | "FS" => 6,
            "G" => 7,
            "G#" | "GS" => 8,
            "A" => 9,
            "A#" | "AS" => 10,
            "B" => 11,
            _ => return None,
        };

        let octave: i32 = octave_part.parse().ok()?;
        let midi_note = (octave + 1) * 12 + note_offset;

        if midi_note >= 0 && midi_note <= 127 {
            Some(midi_note as u8)
        } else {
            None
        }
    }

    /// Check if a MIDI note is a black key on piano
    pub fn is_black_key(pitch: u8) -> bool {
        matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
    }

    /// Check if a MIDI note is a white key on piano
    pub fn is_white_key(pitch: u8) -> bool {
        !Self::is_black_key(pitch)
    }

    /// Get the cents deviation from equal temperament for a frequency
    pub fn cents_from_pitch(freq: f32, pitch: u8) -> f32 {
        let expected_freq = Self::to_frequency(pitch);
        1200.0 * (freq / expected_freq).log2()
    }
}

/// MIDI velocity utilities
pub struct MidiVelocity;

impl MidiVelocity {
    /// Convert velocity to linear amplitude (0.0 to 1.0)
    #[inline]
    pub fn to_amplitude(velocity: u8) -> f32 {
        (velocity as f32 / 127.0).powi(2) // Quadratic curve for more natural dynamics
    }

    /// Convert velocity to decibels
    #[inline]
    pub fn to_db(velocity: u8) -> f32 {
        if velocity == 0 {
            -96.0 // Effectively silent
        } else {
            20.0 * (velocity as f32 / 127.0).log10()
        }
    }

    /// Apply velocity curve (for different keyboard response)
    pub fn apply_curve(velocity: u8, curve_type: VelocityCurve) -> u8 {
        let normalized = velocity as f32 / 127.0;

        let curved = match curve_type {
            VelocityCurve::Linear => normalized,
            VelocityCurve::Soft => normalized.sqrt(), // More sensitive to soft playing
            VelocityCurve::Hard => normalized.powi(2), // Less sensitive to soft playing
            VelocityCurve::Exponential => normalized.exp() / std::f32::consts::E,
        };

        (curved * 127.0).round().clamp(0.0, 127.0) as u8
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VelocityCurve {
    Linear,
    Soft,
    Hard,
    Exponential,
}

/// Simple oscillator for MIDI note synthesis
pub struct SimpleOscillator {
    phase: f64,
    frequency: f32,
    sample_rate: f64,
}

impl SimpleOscillator {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            phase: 0.0,
            frequency: 0.0,
            sample_rate,
        }
    }

    pub fn set_note(&mut self, pitch: u8) {
        self.frequency = MidiNoteUtils::to_frequency(pitch);
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
    }

    /// Generate next sample of sine wave
    #[inline]
    pub fn next_sine(&mut self) -> f32 {
        let sample = (self.phase * 2.0 * std::f64::consts::PI).sin() as f32;
        self.phase += self.frequency as f64 / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        sample
    }

    /// Reset phase for note retrigger
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// Generate a simple sine wave for a MIDI note
#[inline]
pub fn generate_sine_for_note(
    pitch: u8,
    velocity: u8,
    sample_position: f64,
    sample_rate: f64,
) -> f32 {
    let freq = MidiNoteUtils::to_frequency(pitch);
    let amplitude = MidiVelocity::to_amplitude(velocity);
    let phase = (sample_position * freq as f64 / sample_rate) % 1.0;
    (phase * 2.0 * std::f64::consts::PI).sin() as f32 * amplitude * 0.1
}
