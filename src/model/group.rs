use serde::{Deserialize, Serialize};

/// Predefined color palette for quick selection
pub const COLOR_PALETTE: &[(u8, u8, u8)] = &[
    (231, 76, 60),   // Red
    (230, 126, 34),  // Orange
    (241, 196, 15),  // Yellow
    (46, 204, 113),  // Green
    (26, 188, 156),  // Teal
    (52, 152, 219),  // Blue
    (155, 89, 182),  // Purple
    (236, 240, 241), // Light gray
    (149, 165, 166), // Gray
    (44, 62, 80),    // Dark
];

/// A track group for organizational and control-linking purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackGroup {
    pub id: u64,
    pub name: String,
    pub color: (u8, u8, u8),
    pub collapsed: bool,

    // Control linking options (VCA-style)
    pub link_volume: bool,
    pub link_mute: bool,
    pub link_solo: bool,
}

impl Default for TrackGroup {
    fn default() -> Self {
        Self {
            id: 0,
            name: "New Group".into(),
            color: COLOR_PALETTE[5], // Blue
            collapsed: false,
            link_volume: true,
            link_mute: true,
            link_solo: true,
        }
    }
}

impl TrackGroup {
    pub fn new(id: u64, name: String) -> Self {
        // Assign color based on id for variety
        let color_idx = (id as usize) % COLOR_PALETTE.len();
        Self {
            id,
            name,
            color: COLOR_PALETTE[color_idx],
            ..Default::default()
        }
    }

    pub fn color_egui(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.color.0, self.color.1, self.color.2)
    }
}
