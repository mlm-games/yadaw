mod audio;
mod piano_roll;
mod plugin;
mod state;
mod ui;
mod waveform;

use crossbeam_channel::bounded;
use eframe::egui;
use std::sync::{Arc, Mutex};

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    // Scan for plugins
    let mut scanner = plugin::PluginScanner::new();
    scanner.discover_plugins();
    let available_plugins = scanner.get_plugins();
    println!("Found {} LV2 plugins", available_plugins.len());

    // Create communication channels
    let (ui_to_audio_tx, ui_to_audio_rx) = bounded(256);
    let (audio_to_ui_tx, audio_to_ui_rx) = bounded(256);

    // Shared state
    let app_state = Arc::new(Mutex::new(state::AppState::new()));

    // Start audio thread
    let audio_state = app_state.clone();
    std::thread::spawn(move || {
        audio::run_audio_thread(audio_state, ui_to_audio_rx, audio_to_ui_tx);
    });

    // Run UI
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };

    eframe::run_native(
        "YADAW",
        options,
        Box::new(|_cc| {
            Ok(Box::new(ui::YadawApp::new(
                app_state,
                ui_to_audio_tx,
                audio_to_ui_rx,
                available_plugins,
            )))
        }),
    )
}
