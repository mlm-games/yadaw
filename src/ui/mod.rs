mod app;
mod automation_lane;
mod dialogs;
mod menu_bar;
mod mixer;
mod piano_roll;
mod piano_roll_view;
mod theme;
mod timeline;
mod tracks;
mod transport;

pub use app::YadawApp;
pub use theme::{Theme, ThemeManager};

// Re-export commonly used types
use crate::messages::{AudioCommand, UIUpdate};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::sync::{Arc, Mutex};
