pub mod automation;
pub mod clip;
pub mod plugin;
pub mod plugin_api;
pub mod track;

pub use automation::{AutomationLane, AutomationMode, AutomationPoint, AutomationTarget};
pub use clip::{AudioClip, MidiClip, MidiNote};
pub use plugin::{PluginDescriptor, PluginParam};
pub use track::{Send, Track};
