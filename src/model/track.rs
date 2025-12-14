use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{automation::AutomationLane, plugin::PluginDescriptor};
use crate::model::clip::{AudioClip, MidiClip};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TrackType {
    Audio,
    Midi,
    Bus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Send {
    pub destination_track: u64,
    pub amount: f32,
    pub pre_fader: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    #[serde(default)]
    pub id: u64,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub armed: bool,
    pub track_type: TrackType,
    pub midi_input_port: Option<String>,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub midi_clips: Vec<MidiClip>,
    pub audio_clips: Vec<AudioClip>,
    pub plugin_chain: Vec<PluginDescriptor>,
    pub automation_lanes: Vec<AutomationLane>,
    pub sends: Vec<Send>,
    pub group_id: Option<u64>,
    pub color: Option<(u8, u8, u8)>,
    pub height: f32,
    pub minimized: bool,
    pub record_enabled: bool,
    pub monitor_enabled: bool,
    pub input_gain: f32,
    pub phase_inverted: bool,
    pub frozen: bool,
    pub frozen_buffer: Option<Vec<f32>>,

    #[serde(skip)]
    pub plugin_by_id: HashMap<u64, usize>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: 0,
            name: "New Track".to_string(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
            armed: false,
            track_type: TrackType::Audio,
            midi_input_port: None,
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
            plugin_by_id: HashMap::new(),
        }
    }
}

impl Track {
    pub fn rebuild_plugin_index(&mut self) {
        self.plugin_by_id.clear();
        for (idx, plugin) in self.plugin_chain.iter().enumerate() {
            if plugin.id != 0 {
                self.plugin_by_id.insert(plugin.id, idx);
            }
        }
    }

    pub fn find_plugin(&self, plugin_id: u64) -> Option<&PluginDescriptor> {
        self.plugin_by_id
            .get(&plugin_id)
            .and_then(|&idx| self.plugin_chain.get(idx))
    }

    pub fn find_plugin_mut(&mut self, plugin_id: u64) -> Option<&mut PluginDescriptor> {
        let idx = *self.plugin_by_id.get(&plugin_id)?;
        self.plugin_chain.get_mut(idx)
    }
}
