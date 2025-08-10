mod audio;
mod audio_import;
mod audio_state;
mod automation;
mod automation_lane;
mod command_processor;
mod config;
mod constants;
mod edit_actions;
mod error;
mod integration;
mod level_meter;
mod lv2_plugin_host;
mod midi_engine;
mod mixer;
mod performance;
mod piano_roll;
mod plugin;
mod plugin_host;
mod project_manager;
mod state;
mod track_manager;
mod transport;
mod ui;
mod waveform;

use audio_state::{AudioState, RealtimeCommand};
use command_processor::run_command_processor;
use config::Config;
use plugin::PluginScanner;
use state::{AppState, AudioCommand, UIUpdate};
use std::sync::{Arc, Mutex};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    println!("Starting YADAW...");

    // Load configuration
    let config = Config::load().unwrap_or_default();

    // Initialize state
    let app_state = Arc::new(Mutex::new(AppState::new()));
    let audio_state = Arc::new(AudioState::new());

    // Create channels for communication
    let (command_tx, command_rx) = crossbeam_channel::unbounded::<AudioCommand>();
    let (realtime_tx, realtime_rx) = crossbeam_channel::unbounded::<RealtimeCommand>();
    let (ui_tx, ui_rx) = crossbeam_channel::unbounded::<UIUpdate>();

    // Initialize plugin host with audio settings
    plugin::initialize_plugin_host(
        audio_state.sample_rate.load() as f64,
        constants::MAX_BUFFER_SIZE,
    )?;

    println!("Scanning for plugins...");
    let mut plugin_scanner = PluginScanner::new();
    plugin_scanner.discover_plugins();
    let available_plugins = plugin_scanner.get_plugins();

    println!("Found {} LV2 plugins", available_plugins.len());

    // Clone for audio thread
    let ui_tx_audio = ui_tx.clone();

    // Start audio thread FIRST (it needs to be running)
    {
        let audio_state_clone = audio_state.clone();
        std::thread::spawn(move || {
            audio::run_audio_thread(audio_state_clone, realtime_rx, ui_tx_audio);
        });
    }

    println!("Starting command processor thread...");
    {
        let app_state_clone = app_state.clone();
        let audio_state_clone = audio_state.clone();
        let ui_tx_clone = ui_tx.clone();
        std::thread::spawn(move || {
            run_command_processor(
                app_state_clone,
                audio_state_clone,
                command_rx,
                realtime_tx,
                ui_tx_clone,
            );
        });
    }

    println!("Starting audio thread...");
    {
        let _ = command_tx.send(AudioCommand::UpdateTracks);
    }

    println!("Starting UI...");
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "YADAW - Yet Another DAW",
        native_options,
        Box::new(move |_cc| {
            Ok(Box::new(ui::YadawApp::new(
                app_state.clone(),
                audio_state.clone(),
                command_tx.clone(),
                ui_rx,
                available_plugins,
                config,
            )))
        }),
    )?;

    Ok(())
}
