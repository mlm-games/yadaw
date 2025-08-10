mod audio;
mod audio_import;
mod audio_state;
mod automation;
mod automation_lane;
mod command_processor;
mod config;
mod constants;
mod edit_actions;
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

use crossbeam_channel::bounded;
use eframe::egui;
use std::sync::{Arc, Mutex};

use crate::{constants::CHANNEL_QUEUE_SIZE, plugin::PluginScanner};

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    println!("Starting YADAW...");

    let config = match config::Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to load config: {}, using defaults", e);
            config::Config::default()
        }
    };

    // Initialize plugin host with config sample rate
    plugin::initialize_plugin_host(config.audio.sample_rate as f64, config.audio.buffer_size)
        .expect("Failed to initialize plugin host");

    println!("Scanning for plugins...");
    let mut scanner = PluginScanner::new();
    scanner.discover_plugins();
    let available_plugins = scanner.get_plugins();
    println!("Found {} LV2 plugins", available_plugins.len());

    println!("Creating communication channels...");
    let (ui_to_command_tx, ui_to_command_rx) = bounded(CHANNEL_QUEUE_SIZE);
    let (realtime_tx, realtime_rx) = bounded(CHANNEL_QUEUE_SIZE);
    let (audio_to_ui_tx, audio_to_ui_rx) = bounded(CHANNEL_QUEUE_SIZE);

    println!("Creating app state...");
    let app_state = Arc::new(Mutex::new(state::AppState::new()));

    let audio_state = Arc::new(audio_state::AudioState::new());

    println!("Starting command processor thread...");
    let command_state = app_state.clone();
    let command_audio_state = audio_state.clone();
    let command_ui_tx = audio_to_ui_tx.clone();
    std::thread::spawn(move || {
        command_processor::run_command_processor(
            command_state,
            command_audio_state,
            ui_to_command_rx,
            realtime_tx,
            command_ui_tx,
        );
    });

    println!("Starting audio thread...");
    let audio_thread_state = audio_state.clone();
    std::thread::spawn(move || {
        audio::run_audio_thread(audio_thread_state, realtime_rx, audio_to_ui_tx);
    });

    println!("Starting UI...");
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
                audio_state,
                ui_to_command_tx,
                audio_to_ui_rx,
                available_plugins,
                config,
            )))
        }),
    )
}
