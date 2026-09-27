use serde::{Deserialize, Serialize};

const MARKER_COLOR: (u8, u8, u8) = (241, 196, 15);

/// A named point on the timeline, used for navigation and section labelling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: u64,
    pub beat: f64,
    pub name: String,
    #[serde(default)]
    pub color: Option<(u8, u8, u8)>,
    #[serde(default)]
    pub comment: Option<String>,
}

impl Marker {
    pub fn new(id: u64, beat: f64, name: String) -> Self {
        Self {
            id,
            beat,
            name,
            color: Some(MARKER_COLOR),
            comment: None,
        }
    }
}

impl Default for Marker {
    fn default() -> Self {
        Self::new(0, 0.0, "Marker".to_string())
    }
}
