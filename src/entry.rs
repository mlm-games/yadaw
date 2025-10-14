use crate::{
    audio,
    audio_state::{AudioState, RealtimeCommand},
    command_processor::run_command_processor,
    config::Config,
    constants,
    messages::{AudioCommand, UIUpdate},
    plugin_host, ui,
};
use crate::{audio_state::AudioGraphSnapshot, model::plugin_api::HostConfig, project};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "android")]
use android_activity::AndroidApp;

#[cfg(not(target_os = "android"))]
pub fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    // Logging

    #[cfg(not(target_os = "android"))]
    env_logger::init();
    // #[cfg(target_os = "android")]
    // {
    //     android_logger::init_once(
    //         android_logger::Config::default()
    //             .with_min_level(log::Level::Info)
    //             .with_tag("yadaw"),
    //     );
    // }

    log::info!("Starting YADAW...");

    // Load configuration
    let config = Config::load().unwrap_or_default();

    // Initialize state
    let app_state = Arc::new(Mutex::new(project::AppState::default()));
    let audio_state = Arc::new(AudioState::new());

    // Create channels for communication
    let (command_tx, command_rx) = crossbeam_channel::unbounded::<AudioCommand>();
    let (realtime_tx, realtime_rx) = crossbeam_channel::unbounded::<RealtimeCommand>();
    let (ui_tx, ui_rx) = crossbeam_channel::unbounded::<UIUpdate>();

    let (snapshot_tx, snapshot_rx) = crossbeam_channel::bounded::<AudioGraphSnapshot>(1);

    // Initialize the global LV2 plugin host with current audio settings
    plugin_host::init(
        audio_state.sample_rate.load() as f64,
        constants::MAX_BUFFER_SIZE,
    )?;

    log::info!("Scanning for plugins...");
    let host_cfg = HostConfig {
        sample_rate: audio_state.sample_rate.load() as f64, // or device_rate if you queried CPAL first
        max_block: constants::MAX_BUFFER_SIZE,
    };
    let ui_facade = crate::plugin_facade::HostFacade::new(host_cfg)?;
    let available_plugins = ui_facade.scan().unwrap_or_default();

    // Start audio thread
    {
        let audio_state_clone = audio_state.clone();
        let ui_tx_audio = ui_tx.clone();
        std::thread::spawn(move || {
            audio::run_audio_thread(audio_state_clone, realtime_rx, ui_tx_audio, snapshot_rx);
        });
    }

    log::info!("Starting command processor thread...");
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
                snapshot_tx,
            );
        });
    }

    // Prime audio graph
    let _ = command_tx.send(AudioCommand::UpdateTracks);

    // UI
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

#[cfg(target_os = "android")]
pub fn run_app_android(app: AndroidApp) -> Result<(), Box<dyn std::error::Error>> {
    use eframe::wgpu;

    // Initialize logging
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("yadaw"),
    );

    log::info!("Starting YADAW...");

    // Load configuration
    let config = Config::load().unwrap_or_default();

    // Initialize state
    let app_state = Arc::new(Mutex::new(crate::project::AppState::default()));
    let audio_state = Arc::new(AudioState::new());

    // Create channels
    let (command_tx, command_rx) = crossbeam_channel::unbounded::<AudioCommand>();
    let (realtime_tx, realtime_rx) = crossbeam_channel::unbounded::<RealtimeCommand>();
    let (ui_tx, ui_rx) = crossbeam_channel::unbounded::<UIUpdate>();
    let (snapshot_tx, snapshot_rx) = crossbeam_channel::bounded::<AudioGraphSnapshot>(1);

    // Initialize plugin host
    plugin_host::init(
        audio_state.sample_rate.load() as f64,
        constants::MAX_BUFFER_SIZE,
    )?;

    log::info!("Scanning for plugins...");
    let host_cfg = HostConfig {
        sample_rate: audio_state.sample_rate.load() as f64, // or device_rate if you queried CPAL first
        max_block: constants::MAX_BUFFER_SIZE,
    };
    let ui_facade = crate::plugin_facade::HostFacade::new(host_cfg)?;
    let available_plugins = ui_facade.scan().unwrap_or_default();

    // Start audio thread
    {
        let audio_state_clone = audio_state.clone();
        let ui_tx_audio = ui_tx.clone();
        let snapshot_tx_audio = snapshot_tx.clone();
        std::thread::spawn(move || {
            audio::run_audio_thread(audio_state_clone, realtime_rx, ui_tx_audio, snapshot_rx);
        });
    }

    // Start command processor
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
                snapshot_tx,
            );
        });
    }

    // Prime audio graph
    let _ = command_tx.send(AudioCommand::UpdateTracks);

    // Android-specific eframe options
    let native_options = eframe::NativeOptions {
        android_app: Some(app), // Pass the Android app here!
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
