use serde::{Deserialize, Serialize};

use super::{automation::AutomationLane, plugin::PluginDescriptor};
use crate::model::clip::{AudioClip, MidiClip};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Send {
    pub destination_track: usize,
    pub amount: f32,
    pub pre_fader: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub is_midi: bool,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub midi_clips: Vec<MidiClip>,
    pub audio_clips: Vec<AudioClip>,
    pub plugin_chain: Vec<PluginDescriptor>,
    pub automation_lanes: Vec<AutomationLane>,
    pub sends: Vec<Send>,
    pub group_id: Option<usize>,
    pub color: Option<(u8, u8, u8)>,
    pub height: f32,
    pub minimized: bool,
    pub record_enabled: bool,
    pub monitor_enabled: bool,
    pub input_gain: f32,
    pub phase_inverted: bool,
    pub frozen: bool,
    pub frozen_buffer: Option<Vec<f32>>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            name: "New Track".to_string(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
            armed: false,
            is_midi: false,
            input_device: None,
            output_device: None,
            midi_clips: Vec::new(),
            audio_clips: Vec::new(),
            plugin_chain: Vec::new(),
            automation_lanes: Vec::new(),
            sends: Vec::new(),
            group_id: None,
            color: None,
            height: 80.0,
            minimized: false,
            record_enabled: false,
            monitor_enabled: false,
            input_gain: 1.0,
            phase_inverted: false,
            frozen: false,
            frozen_buffer: None,
        }
    }
}
