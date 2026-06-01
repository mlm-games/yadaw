pub mod android_saf;
pub mod audio;
mod audio_codecs;
mod audio_export;
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
pub mod file_picker;
mod file_picker_android;
mod file_picker_desktop;
pub mod idgen;
pub mod input;
pub mod level_meter;
pub mod messages;
pub mod metering;
pub mod midi_import;
pub mod midi_input;
pub mod midi_utils;
pub mod mixer;
pub mod model;
pub mod paths;
pub mod performance;
pub mod plugin;
pub mod presets;
pub mod project;
pub mod project_manager;
pub mod time_utils;
pub mod track_manager;
pub mod transport;
pub mod ui;

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

    if let Ok(home) = crate::android_saf::files_dir_path() {
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &home);
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_CACHE_HOME", &home);
            std::env::set_var("XDG_CONFIG_HOME", &home);
        }
        log::info!("yadaw: set XDG_*/HOME to {}", home.display());
    } else {
        log::warn!("yadaw: failed to resolve files dir; plugin storage may be read-only");
    }

    rlobkit_dialogs::init();
    rlobkit_dialogs::init_shared_pending_state();

    // Publish the JavaVM/Context pointers so dlopen'd CLAP plugins (for ex.
    // check mampler) can initialize their own copy of ndk-context.
    unsafe {
        std::env::set_var(
            "RLOBKIT_ANDROID_VM",
            format!("0x{:x}", app.vm_as_ptr() as usize),
        );
        std::env::set_var(
            "RLOBKIT_ANDROID_CTX",
            format!("0x{:x}", app.activity_as_ptr() as usize),
        );
    }

    // Start your app. If it errors, log it rather than abort.
    if let Err(e) = crate::entry::run_app_android(app) {
        log::error!("android_main error: {e}");
    }
}
