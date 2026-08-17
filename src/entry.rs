use crate::audio;
use crate::audio_state::{AudioGraphSnapshot, AudioState, EngineEvent, PluginWorkerCommand, RealtimeCommand};
use crate::config::Config;
use crate::messages::{AudioCommand, UiRx, UiTx};
use crate::midi_input::MidiInputHandler;
use crate::spawn_detached;
use crate::{project, ui};
use flume::{self, Sender};
use std::sync::Arc;
use web_workers::sync::mpsc::{Receiver, channel};
use web_workers::sync::Mutex;

#[cfg(not(target_arch = "wasm32"))]
use crate::constants;
use yadaw_plugin_api::HostConfig;
use yadaw_plugin_host::HostFacade;
#[cfg(all(not(target_arch = "wasm32"), feature = "lv2-legacy"))]
use yadaw_plugin_host::legacy::init as plugin_host_init;

#[cfg(target_os = "android")]
use android_activity::AndroidApp;
#[cfg(all(target_os = "android", feature = "lv2-legacy"))]
use yadaw_plugin_host::plugin_host;

struct AppChannels {
    command_tx: Sender<AudioCommand>,
    ui_tx: UiTx,
    ui_rx: UiRx,
    midi_handler: Option<Arc<MidiInputHandler>>,
}

fn setup_channels_and_start_audio(
    app_state: &Arc<Mutex<project::AppState>>,
    audio_state: &Arc<AudioState>,
    host_cfg: HostConfig,
    start_audio: impl FnOnce(
        Receiver<RealtimeCommand>,
        Receiver<AudioGraphSnapshot>,
        flume::Receiver<EngineEvent>,
        flume::Sender<PluginWorkerCommand>,
        UiTx,
    ),
) -> AppChannels {
    let (command_tx, command_rx) = flume::unbounded::<AudioCommand>();
    let (realtime_tx, realtime_rx) = channel::<RealtimeCommand>();
    let (snapshot_tx, snapshot_rx) = channel::<AudioGraphSnapshot>();
    let (ui_tx, ui_rx) = channel();

    let (plugin_worker_tx, plugin_worker_rx) = flume::bounded::<PluginWorkerCommand>(512);
    let (engine_events_tx, engine_events_rx) = flume::unbounded::<EngineEvent>();
    if let Ok(facade) = HostFacade::new(host_cfg) {
        crate::plugin_worker::spawn_plugin_worker(
            Arc::new(facade),
            plugin_worker_rx,
            engine_events_tx,
            ui_tx.clone(),
        );
    }

    start_audio(
        realtime_rx,
        snapshot_rx,
        engine_events_rx,
        plugin_worker_tx.clone(),
        ui_tx.clone(),
    );

    let midi_handler = match MidiInputHandler::new(command_tx.clone()) {
        Ok(handler) => Some(Arc::new(handler)),
        Err(e) => {
            log::warn!("Could not create MIDI Input handler: {}", e);
            None
        }
    };

    spawn_detached!(crate::command_processor::run_command_processor(
        app_state.clone(),
        audio_state.clone(),
        command_rx,
        realtime_tx,
        ui_tx.clone(),
        snapshot_tx,
        midi_handler.clone(),
        plugin_worker_tx,
    ));

    let _ = command_tx.send(AudioCommand::UpdateTracks);

    AppChannels {
        command_tx,
        ui_tx,
        ui_rx,
        midi_handler,
    }
}

#[cfg(all(not(target_os = "android"), not(target_arch = "wasm32")))]
pub fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    yadaw_plugin_host::init_xlib_threads_early();

    // Logging

    #[cfg(not(target_os = "android"))]
    env_logger::init();

    rlobkit_dialogs::init();

    log::info!("Starting YADAW...");

    let file_to_open: Option<String> = std::env::args().nth(1).and_then(|arg| {
        let path = std::path::Path::new(&arg);
        if path.exists() && path.is_file() {
            Some(arg)
        } else {
            None
        }
    });

    // Load configuration
    let config = Config::load().unwrap_or_default();

    // Initialize state
    let app_state = Arc::new(Mutex::new(project::AppState::default()));
    let audio_state = Arc::new(AudioState::new());

    let preferred_sample_rate = config.audio.sample_rate;
    let host_sample_rate = audio::resolve_output_sample_rate(preferred_sample_rate);
    audio_state.sample_rate.store(host_sample_rate);
    {
        let mut state = app_state.lock_sync();
        state.sample_rate = host_sample_rate;
    }

    // Initialize the global LV2 plugin host with current audio settings
    #[cfg(feature = "lv2-legacy")]
    plugin_host_init(host_sample_rate as f64, constants::MAX_BUFFER_SIZE)?;

    log::info!("Scanning for plugins...");
    let host_cfg = HostConfig {
        sample_rate: host_sample_rate as f64,
        max_block: constants::MAX_BUFFER_SIZE,
        plugin_scan_paths: config.paths.plugin_scan_paths.clone(),
    };
    let ui_facade = HostFacade::new(host_cfg)?;
    let available_plugins = ui_facade.scan().unwrap_or_default();

    let audio_state_audio = audio_state.clone();
    let worker_host_cfg = HostConfig {
        sample_rate: host_sample_rate as f64,
        max_block: constants::MAX_BUFFER_SIZE,
        plugin_scan_paths: config.paths.plugin_scan_paths.clone(),
    };
    let channels = setup_channels_and_start_audio(
        &app_state,
        &audio_state,
        worker_host_cfg,
        |realtime_rx, snapshot_rx, engine_events_rx, plugin_worker_tx, ui_tx_audio| {
            let audio_state_audio = audio_state_audio.clone();
            std::thread::spawn(move || {
                audio::run_audio_thread(
                    audio_state_audio,
                    realtime_rx,
                    ui_tx_audio,
                    snapshot_rx,
                    engine_events_rx,
                    plugin_worker_tx,
                    host_sample_rate,
                );
            });
        },
    );

    // UI
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    let initial_file = file_to_open.clone();

    eframe::run_native(
        "Yadaw",
        native_options,
        Box::new(move |_cc| {
            let ui_midi_handler = channels.midi_handler.clone();
            let mut app = ui::YadawApp::new(
                app_state.clone(),
                audio_state.clone(),
                channels.command_tx.clone(),
                channels.ui_rx,
                channels.ui_tx.clone(),
                available_plugins,
                config,
                ui_midi_handler,
            );

            // Open file if provided
            if let Some(ref path) = initial_file {
                app.open_file_from_path(std::path::Path::new(path));
            }

            Ok(Box::new(app))
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

    let preferred_sample_rate = config.audio.sample_rate;
    let host_sample_rate = audio::resolve_output_sample_rate(preferred_sample_rate);
    audio_state.sample_rate.store(host_sample_rate);
    {
        let mut state = app_state.lock_sync();
        state.sample_rate = host_sample_rate;
    }

    // Initialize plugin host
    #[cfg(feature = "lv2-legacy")]
    plugin_host::init(host_sample_rate as f64, constants::MAX_BUFFER_SIZE)?;

    log::info!("Scanning for plugins...");
    let host_cfg = HostConfig {
        sample_rate: host_sample_rate as f64,
        max_block: constants::MAX_BUFFER_SIZE,
        plugin_scan_paths: config.paths.plugin_scan_paths.clone(),
    };
    let ui_facade = HostFacade::new(host_cfg)?;
    let available_plugins = ui_facade.scan().unwrap_or_default();

    let audio_state_audio = audio_state.clone();
    let worker_host_cfg = HostConfig {
        sample_rate: host_sample_rate as f64,
        max_block: constants::MAX_BUFFER_SIZE,
        plugin_scan_paths: config.paths.plugin_scan_paths.clone(),
    };
    let channels = setup_channels_and_start_audio(
        &app_state,
        &audio_state,
        worker_host_cfg,
        |realtime_rx, snapshot_rx, engine_events_rx, plugin_worker_tx, ui_tx_audio| {
            let audio_state_audio = audio_state_audio.clone();
            std::thread::spawn(move || {
                audio::run_audio_thread(
                    audio_state_audio,
                    realtime_rx,
                    ui_tx_audio,
                    snapshot_rx,
                    engine_events_rx,
                    plugin_worker_tx,
                    host_sample_rate,
                );
            });
        },
    );

    // UI
    let native_options = eframe::NativeOptions {
        android_app: Some(app), // Pass the Android app here!

        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Yadaw",
        native_options,
        Box::new(move |_cc| {
            let ui_midi_handler = channels.midi_handler.clone();
            Ok(Box::new(ui::YadawApp::new(
                app_state.clone(),
                audio_state.clone(),
                channels.command_tx.clone(),
                channels.ui_rx,
                channels.ui_tx.clone(),
                available_plugins,
                config,
                ui_midi_handler,
            )))
        }),
    )?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn create_app() -> ui::YadawApp {
    let config = Config::default();
    let app_state = Arc::new(Mutex::new(project::AppState::default()));
    let audio_state = Arc::new(AudioState::new());

    let channels = setup_channels_and_start_audio(
        &app_state,
        &audio_state,
        HostConfig {
            sample_rate: config.audio.sample_rate as f64,
            max_block: crate::constants::MAX_BUFFER_SIZE,
            plugin_scan_paths: Vec::new(),
        },
        |realtime_rx, snapshot_rx, engine_events_rx, plugin_worker_tx, ui_tx_audio| {
            audio::run_audio_wasm(
                audio_state.clone(),
                realtime_rx,
                ui_tx_audio,
                snapshot_rx,
                engine_events_rx,
                plugin_worker_tx,
                config.audio.sample_rate,
            );
        },
    );

    ui::YadawApp::new(
        app_state,
        audio_state,
        channels.command_tx,
        channels.ui_rx,
        channels.ui_tx.clone(),
        vec![],
        config,
        channels.midi_handler,
    )
}
