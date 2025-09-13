pub mod audio;
pub mod audio_import;
pub mod audio_snapshot;
pub mod audio_state;
pub mod audio_utils;
pub mod command_processor;
pub mod config;
pub mod constants;
pub mod edit_actions;
pub mod entry;
pub mod error;
pub mod level_meter;
pub mod lv2_plugin_host;
pub mod messages;
pub mod metering;
pub mod midi_utils;
pub mod mixer;
pub mod model;
pub mod performance;
pub mod piano_roll;
pub mod plugin;
pub mod plugin_host;
pub mod project;
pub mod project_manager;
pub mod state;
pub mod time_utils;
pub mod track_manager;
pub mod transport;
pub mod ui;
pub mod waveform;

#[cfg(target_os = "android")]
use android_activity::WindowManagerFlags;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    use android_activity::AndroidApp;

    app.set_window_flags(
        WindowManagerFlags::FULLSCREEN | WindowManagerFlags::LAYOUT_NO_LIMITS,
        WindowManagerFlags::empty(),
    );

    // Initialize Android logging
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("yadaw"),
    );

    log::info!("Starting YADAW on Android...");

    // Start your app. If it errors, log it rather than abort.
    if let Err(e) = crate::entry::run_app_android(app) {
        log::error!("android_main error: {e}");
    }
}
