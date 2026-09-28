use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::constants::DEFAULT_GROUP_VOLUME;

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

pub const GROUP_CHAIN_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GroupLinkMode {
    /// Attenuates members without rewriting their faders.
    #[default]
    Vca,
    /// Also drives the member faders. Rejected on load until implemented.
    Linked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackGroup {
    pub id: u64,
    pub name: String,
    pub color: (u8, u8, u8),
    pub collapsed: bool,

    #[serde(default)]
    pub parent_id: Option<u64>,

    #[serde(default = "default_group_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub link_mode: GroupLinkMode,
}

fn default_group_volume() -> f32 {
    DEFAULT_GROUP_VOLUME
}

impl Default for TrackGroup {
    fn default() -> Self {
        Self {
            id: 0,
            name: "New Group".into(),
            color: COLOR_PALETTE[5], // Blue
            collapsed: false,
            parent_id: None,
            volume: DEFAULT_GROUP_VOLUME,
            muted: false,
            solo: false,
            link_mode: GroupLinkMode::Vca,
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

#[derive(Debug, Clone, Copy)]
pub struct GroupNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupResolution {
    /// Product of the volumes of the group and every ancestor.
    pub gain: f32,
    pub muted: bool,
    pub soloed: bool,
    /// Enclosing groups, innermost first; metering walks this.
    pub chain: SmallVec<[u64; 4]>,
}

impl GroupResolution {
    pub fn neutral() -> Self {
        Self {
            gain: 1.0,
            muted: false,
            soloed: false,
            chain: SmallVec::new(),
        }
    }
}

pub fn group_index(nodes: &[GroupNode]) -> HashMap<u64, GroupNode> {
    nodes.iter().map(|n| (n.id, *n)).collect()
}

pub fn any_group_soloed(index: &HashMap<u64, GroupNode>) -> bool {
    index.values().any(|n| n.solo)
}

/// Stops at a missing, repeated or absurdly deep group, so a broken
/// `parent_id` cannot spin or silence a track.
pub fn resolve_group_chain(
    index: &HashMap<u64, GroupNode>,
    group_id: Option<u64>,
) -> GroupResolution {
    let mut res = GroupResolution::neutral();
    let mut cursor = group_id;
    let mut depth = 0usize;

    while let Some(id) = cursor {
        if depth >= GROUP_CHAIN_LIMIT || res.chain.contains(&id) {
            break;
        }
        let Some(node) = index.get(&id) else {
            break;
        };
        res.gain *= if node.volume.is_finite() && node.volume >= 0.0 {
            node.volume
        } else {
            1.0
        };
        res.muted |= node.muted;
        res.soloed |= node.solo;
        res.chain.push(id);
        cursor = node.parent;
        depth += 1;
    }

    res
}

pub fn resolve_track_groups(
    index: &HashMap<u64, GroupNode>,
    tracks: impl IntoIterator<Item = (u64, Option<u64>)>,
) -> HashMap<u64, GroupResolution> {
    tracks
        .into_iter()
        .map(|(track_id, group_id)| (track_id, resolve_group_chain(index, group_id)))
        .collect()
}
