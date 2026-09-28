pub mod automation;
pub mod clip;
pub mod group;
pub mod marker;
pub mod plugin;
pub mod track;

pub use automation::{AutomationLane, AutomationMode, AutomationPoint, AutomationTarget};
pub use clip::{AudioClip, MidiClip, MidiNote, WarpCurve, WarpPoint};
pub use group::{COLOR_PALETTE, GroupLinkMode, GroupNode, GroupResolution, TrackGroup};
pub use marker::Marker;
pub use plugin::{PluginDescriptor, PluginParam};
pub use track::{Send, Track};
