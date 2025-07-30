use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: usize,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub plugin_chain: Vec<PluginInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInstance {
    pub uri: String,
    pub name: String,
    pub bypass: bool,
    pub params: HashMap<String, f32>,
}

#[derive(Debug)]
pub struct AppState {
    pub tracks: Vec<Track>,
    pub master_volume: f32,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub sample_rate: f32,
    pub buffer_size: usize,
    pub current_position: f64, // in samples
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tracks: vec![Track {
                id: 0,
                name: "Track 1".to_string(),
                volume: 0.7,
                pan: 0.0,
                muted: false,
                solo: false,
                armed: false,
                plugin_chain: vec![],
            }],
            master_volume: 0.8,
            playing: false,
            recording: false,
            bpm: 120.0,
            sample_rate: 44100.0,
            buffer_size: 512,
            current_position: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    Stop,
    Record,
    StopRecording,
    SetTrackVolume(usize, f32),
    SetTrackPan(usize, f32),
    MuteTrack(usize, bool),
    SoloTrack(usize, bool),
    AddPlugin(usize, String),                  // track_id, plugin_uri
    RemovePlugin(usize, usize),                // track_id, plugin_index
    SetPluginParam(usize, usize, String, f32), // track, plugin, param, value
}

#[derive(Debug, Clone)]
pub enum UIUpdate {
    Position(f64),
    PeakLevel(usize, f32, f32), // track_id, left, right
    PluginAdded(usize, String),
}
