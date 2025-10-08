use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: f64,
    pub value: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AutomationMode {
    Off,
    Read,
    Write,
    Touch,
    Latch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    pub parameter: AutomationTarget,
    pub points: Vec<AutomationPoint>,
    pub visible: bool,
    pub height: f32,
    pub color: Option<(u8, u8, u8)>,
    pub write_mode: AutomationMode,
    pub read_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AutomationTarget {
    TrackVolume,
    TrackPan,
    TrackSend(u64),
    PluginParam { plugin_id: u64, param_name: String },
}
