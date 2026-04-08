pub mod automation;
pub mod clip;
pub mod group;
pub mod plugin;
pub mod track;

pub use automation::{AutomationLane, AutomationMode, AutomationPoint, AutomationTarget};
pub use clip::{AudioClip, MidiClip, MidiNote};
pub use group::{TrackGroup, COLOR_PALETTE};
pub use plugin::{PluginDescriptor, PluginParam};
pub use track::{Send, Track};
