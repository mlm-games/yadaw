use serde::{Deserialize, Serialize};

use crate::model::{track::Track, AutomationTarget};
use crate::time_utils::TimeConverter;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
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
}
