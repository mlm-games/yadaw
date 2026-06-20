use crate::audio;
use crate::audio_state::{AudioGraphSnapshot, AudioState, RealtimeCommand};
use crate::config::Config;
use crate::messages::AudioCommand;
use crate::messages::UIUpdate;
use crate::midi_input::MidiInputHandler;
use crate::spawn_detached;
use crate::{project, ui};
use flume::{self, Sender};
use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use crate::constants;
#[cfg(not(target_arch = "wasm32"))]
use yadaw_plugin_api::HostConfig;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "lv2-legacy")))]
use yadaw_plugin_host::HostFacade;
#[cfg(all(not(target_arch = "wasm32"), feature = "lv2-legacy"))]
use yadaw_plugin_host::{HostFacade, legacy::init as plugin_host_init};

#[cfg(target_os = "android")]
use android_activity::AndroidApp;
#[cfg(all(target_os = "android", feature = "lv2-legacy"))]
use yadaw_plugin_host::plugin_host;

struct AppChannels {
    command_tx: Sender<AudioCommand>,
    ui_tx: Sender<UIUpdate>,
    ui_rx: flume::Receiver<UIUpdate>,
    midi_handler: Option<Arc<MidiInputHandler>>,
}

fn setup_channels_and_start_audio(
    app_state: &Arc<Mutex<project::AppState>>,
    audio_state: &Arc<AudioState>,
    start_audio: impl FnOnce(
        flume::Receiver<RealtimeCommand>,
        flume::Receiver<AudioGraphSnapshot>,
        Sender<UIUpdate>,
    ),
) -> AppChannels {
    let (command_tx, command_rx) = flume::unbounded::<AudioCommand>();
    let (realtime_tx, realtime_rx) = flume::unbounded::<RealtimeCommand>();
    let (snapshot_tx, snapshot_rx) = flume::bounded::<AudioGraphSnapshot>(1);
    let (ui_tx, ui_rx) = flume::unbounded::<UIUpdate>();

    start_audio(realtime_rx, snapshot_rx, ui_tx.clone());

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
        let mut state = app_state.lock().unwrap();
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
    let channels = setup_channels_and_start_audio(
        &app_state,
        &audio_state,
        |realtime_rx, snapshot_rx, ui_tx_audio| {
            let audio_state_audio = audio_state_audio.clone();
            std::thread::spawn(move || {
                audio::run_audio_thread(
                    audio_state_audio,
                    realtime_rx,
                    ui_tx_audio,
                    snapshot_rx,
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
        let mut state = app_state.lock().unwrap();
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
    let channels = setup_channels_and_start_audio(
        &app_state,
        &audio_state,
        |realtime_rx, snapshot_rx, ui_tx_audio| {
            let audio_state_audio = audio_state_audio.clone();
            std::thread::spawn(move || {
                audio::run_audio_thread(
                    audio_state_audio,
                    realtime_rx,
                    ui_tx_audio,
                    snapshot_rx,
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
        |realtime_rx, snapshot_rx, ui_tx_audio| {
            audio::run_audio_wasm(
                audio_state.clone(),
                realtime_rx,
                ui_tx_audio,
                snapshot_rx,
                config.audio.sample_rate,
            );
        },
    );

    ui::YadawApp::new(
        app_state,
        audio_state,
        channels.command_tx,
        channels.ui_rx,
        vec![],
        config,
        channels.midi_handler,
    )
}
