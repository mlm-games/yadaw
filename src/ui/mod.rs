mod app;
mod dialogs;
mod menu_bar;
mod mixer;
mod piano_roll_view;
mod theme;
mod timeline;
mod tracks;
mod transport;
mod widgets;

pub use app::YadawApp;
pub use theme::{Theme, ThemeManager};
pub use widgets::*;

// Re-export commonly used types
use crate::messages::{AudioCommand, UIUpdate};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::sync::{Arc, Mutex};
