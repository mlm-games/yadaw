use crate::midi_input::MidiInputHandler;
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

    std::panic::set_hook(Box::new(|info| {
        eprintln!("Panic: {info}");
        let _ = std::fs::write(
            crate::paths::cache_dir().join("last_panic.txt"),
            format!("{info:?}"),
        );
    }));

    // Start audio thread
    {
        let audio_state_clone = audio_state.clone();
        let ui_tx_audio = ui_tx.clone();
        std::thread::spawn(move || {
            audio::run_audio_thread(audio_state_clone, realtime_rx, ui_tx_audio, snapshot_rx);
        });
    }

    let midi_input_handler = match MidiInputHandler::new(command_tx.clone()) {
        Ok(handler) => Some(Arc::new(handler)),
        Err(e) => {
            log::warn!("Could not create MIDI Input handler: {}", e);
            None
        }
    };

    log::info!("Starting command processor thread...");
    {
        let app_state_clone = app_state.clone();
        let audio_state_clone = audio_state.clone();
        let ui_tx_clone = ui_tx.clone();
        let midi_input_handler_clone = midi_input_handler.clone();
        std::thread::spawn(move || {
            run_command_processor(
                app_state_clone,
                audio_state_clone,
                command_rx,
                realtime_tx,
                ui_tx_clone,
                snapshot_tx,
                midi_input_handler_clone,
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

    let initial_file = file_to_open.clone();

    eframe::run_native(
        "YADAW - Yet Another DAW",
        native_options,
        Box::new(move |_cc| {
            let ui_midi_handler = midi_input_handler.clone();
            let mut app = ui::YadawApp::new(
                app_state.clone(),
                audio_state.clone(),
                command_tx.clone(),
                ui_rx,
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

    // Create channels
    let (command_tx, command_rx) = crossbeam_channel::unbounded::<AudioCommand>();
    let (realtime_tx, realtime_rx) = crossbeam_channel::unbounded::<RealtimeCommand>();
    let (ui_tx, ui_rx) = crossbeam_channel::unbounded::<UIUpdate>();
    let (snapshot_tx, snapshot_rx) = crossbeam_channel::bounded::<AudioGraphSnapshot>(1);

    import_clap_bundles_from_external();

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

    let midi_input_handler = match MidiInputHandler::new(command_tx.clone()) {
        Ok(handler) => Some(Arc::new(handler)),
        Err(e) => {
            log::warn!("Could not create MIDI Input handler: {}", e);
            None
        }
    };

    log::info!("Starting command processor thread...");
    {
        let app_state_clone = app_state.clone();
        let audio_state_clone = audio_state.clone();
        let ui_tx_clone = ui_tx.clone();
        let midi_input_handler_clone = midi_input_handler.clone();
        std::thread::spawn(move || {
            run_command_processor(
                app_state_clone,
                audio_state_clone,
                command_rx,
                realtime_tx,
                ui_tx_clone,
                snapshot_tx,
                midi_input_handler_clone,
            );
        });
    }

    // Prime audio graph
    let _ = command_tx.send(AudioCommand::UpdateTracks);

    // UI
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
            let ui_midi_handler = midi_input_handler.clone();
            Ok(Box::new(ui::YadawApp::new(
                app_state.clone(),
                audio_state.clone(),
                command_tx.clone(),
                ui_rx,
                available_plugins,
                config,
                ui_midi_handler,
            )))
        }),
    )?;

    Ok(())
}

#[cfg(target_os = "android")]
fn import_clap_bundles_from_external() {
    use std::{fs, io, path::Path};
    fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let sp = entry.path();
            let dp = dst.join(entry.file_name());
            if sp.is_dir() {
                copy_dir(&sp, &dp)?;
            } else {
                // overwrite if needed
                fs::copy(&sp, &dp)?;
            }
        }
        Ok(())
    }

    let src_docs = crate::paths::projects_dir();
    let src_legacy = Path::new("/storage/emulated/0/Android/data/com.yadaw.app/files")
        .join("plugins")
        .join("clap");

    let dst = crate::paths::plugins_dir(); // internal exec dir

    for src in [src_docs, src_legacy] {
        if let Ok(entries) = fs::read_dir(&src) {
            for e in entries.flatten() {
                let p = e.path();
                // We only import .clap bundles (directories ending with .clap)
                if p.is_dir() && p.extension().and_then(|s| s.to_str()) == Some("clap") {
                    let target = dst.join(p.file_name().unwrap());
                    if !target.exists() {
                        if let Err(err) = copy_dir(&p, &target) {
                            eprintln!("Import of {:?} failed: {}", p, err);
                        }
                    }
                }
            }
        }
    }
}
