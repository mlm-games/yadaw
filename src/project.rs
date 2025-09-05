use serde::{Deserialize, Serialize};

use crate::model::{AutomationTarget, track::Track};
use crate::time_utils::TimeConverter;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppState {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub sample_rate: f32,
    pub buffer_size: usize,
    pub current_position: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub time_signature: (i32, i32),
    pub next_id: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            master_volume: 0.8,
            playing: false,
            recording: false,
            bpm: 120.0,
            sample_rate: 44100.0,
            buffer_size: 512,
            current_position: 0.0,
            loop_start: 0.0,
            loop_end: 4.0,
            loop_enabled: false,
            time_signature: (4, 4),
            next_id: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub bpm: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub sample_rate: f32,
    pub time_signature: (i32, i32),
    pub playing: bool,
    pub recording: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub version: String,
    pub name: String,
    pub tracks: Vec<Track>,
    pub bpm: f32,
    pub time_signature: (i32, i32),
    pub sample_rate: f32,
    pub master_volume: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

impl From<&AppState> for Project {
    fn from(state: &AppState) -> Self {
        Project {
            version: "1.0.0".to_string(),
            name: "Untitled Project".to_string(),
            tracks: state.tracks.clone(),
            bpm: state.bpm,
            time_signature: state.time_signature,
            sample_rate: state.sample_rate,
            master_volume: state.master_volume,
            loop_start: state.loop_start,
            loop_end: state.loop_end,
            loop_enabled: state.loop_enabled,
            created_at: chrono::Utc::now(),
            modified_at: chrono::Utc::now(),
        }
    }
}

impl AppState {
    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            tracks: self.tracks.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            playing: self.playing,
            recording: self.recording,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
        }
    }

    pub fn restore(&mut self, snapshot: AppStateSnapshot) {
        self.tracks = snapshot.tracks;
        self.bpm = snapshot.bpm;
        self.time_signature = snapshot.time_signature;
        self.sample_rate = snapshot.sample_rate;
        self.playing = snapshot.playing;
        self.recording = snapshot.recording;
        self.master_volume = snapshot.master_volume;
        self.loop_start = snapshot.loop_start;
        self.loop_end = snapshot.loop_end;
        self.loop_enabled = snapshot.loop_enabled;
        self.ensure_ids();
    }

    pub fn position_to_beats(&self, position: f64) -> f64 {
        let converter = TimeConverter::new(self.sample_rate, self.bpm);
        converter.samples_to_beats(position)
    }

    pub fn beats_to_samples(&self, beats: f64) -> f64 {
        let converter = TimeConverter::new(self.sample_rate, self.bpm);
        converter.beats_to_samples(beats)
    }

    pub fn load_project(&mut self, project: Project) {
        self.tracks = project.tracks;
        self.bpm = project.bpm;
        self.time_signature = project.time_signature;
        self.sample_rate = project.sample_rate;
        self.master_volume = project.master_volume;
        self.loop_start = project.loop_start;
        self.loop_end = project.loop_end;
        self.loop_enabled = project.loop_enabled;
        self.ensure_ids();
    }

    pub fn to_project(&self) -> Project {
        Project {
            version: "1.0.0".to_string(),
            name: "Untitled Project".to_string(),
            tracks: self.tracks.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
            created_at: chrono::Utc::now(),
            modified_at: chrono::Utc::now(),
        }
    }

    // ID management
    pub fn fresh_id(&mut self) -> u64 {
        if self.next_id == 0 {
            self.reseed_next_id_from_max();
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn ensure_ids(&mut self) {
        // Pass 1: find max existing id
        let mut max_id = 0u64;
        for t in &self.tracks {
            for c in &t.audio_clips {
                max_id = max_id.max(c.id);
            }
            for c in &t.midi_clips {
                max_id = max_id.max(c.id);
                for n in &c.notes {
                    max_id = max_id.max(n.id);
                }
            }
        }

        let mut next = if self.next_id == 0 {
            max_id.saturating_add(1).max(1)
        } else {
            self.next_id.max(max_id.saturating_add(1).max(1))
        };

        // Pass 2: assign missing ids without calling &mut self recursively
        for t in &mut self.tracks {
            for c in &mut t.audio_clips {
                if c.id == 0 {
                    c.id = next;
                    next = next.saturating_add(1);
                }
            }
            for c in &mut t.midi_clips {
                if c.id == 0 {
                    c.id = next;
                    next = next.saturating_add(1);
                }
                for n in &mut c.notes {
                    if n.id == 0 {
                        n.id = next;
                        next = next.saturating_add(1);
                    }
                }
            }
        }

        self.next_id = next;
    }

    fn reseed_next_id_from_max(&mut self) {
        let mut max_id = 0u64;
        for t in &self.tracks {
            for c in &t.audio_clips {
                max_id = max_id.max(c.id);
            }
            for c in &t.midi_clips {
                max_id = max_id.max(c.id);
                for n in &c.notes {
                    max_id = max_id.max(n.id);
                }
            }
        }
        self.next_id = max_id.saturating_add(1).max(1);
    }
}
