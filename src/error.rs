use std::fmt;

#[derive(Debug)]
pub enum YadawError {
    Audio(String),
    Plugin(String),
    File(String),
    State(String),
}

impl fmt::Display for YadawError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            YadawError::Audio(msg) => write!(f, "Audio error: {}", msg),
            YadawError::Plugin(msg) => write!(f, "Plugin error: {}", msg),
            YadawError::File(msg) => write!(f, "File error: {}", msg),
            YadawError::State(msg) => write!(f, "State error: {}", msg),
        }
    }
}

impl std::error::Error for YadawError {}

pub type Result<T> = std::result::Result<T, YadawError>;

// Add user-friendly message generation
impl YadawError {
    /// Get a user-friendly title for this error
    pub fn title(&self) -> &str {
        match self {
            YadawError::Audio(_) => "Audio Error",
            YadawError::Plugin(_) => "Plugin Error",
            YadawError::File(_) => "File Error",
            YadawError::State(_) => "Application Error",
        }
    }

    /// Get a user-friendly hint for resolving this error
    pub fn hint(&self) -> Option<&str> {
        match self {
            YadawError::Audio(_) => Some("Check your audio device settings"),
            YadawError::Plugin(_) => Some("Try bypassing the plugin or checking for updates"),
            YadawError::File(_) => Some("Check file permissions and disk space"),
            YadawError::State(_) => Some("Try restarting the application"),
        }
    }

    /// Get the detailed error message
    pub fn details(&self) -> &str {
        match self {
            YadawError::Audio(msg)
            | YadawError::Plugin(msg)
            | YadawError::File(msg)
            | YadawError::State(msg) => msg,
        }
    }

    /// Format for user display
    pub fn user_message(&self) -> String {
        if let Some(hint) = self.hint() {
            format!("{}: {}\n\nHint: {}", self.title(), self.details(), hint)
        } else {
            format!("{}: {}", self.title(), self.details())
        }
    }

    /// Format for logging
    pub fn log_message(&self) -> String {
        format!("{}", self) // Uses Display impl
    }
}

// Conversion helpers
impl From<std::io::Error> for YadawError {
    fn from(err: std::io::Error) -> Self {
        YadawError::File(err.to_string())
    }
}

impl From<anyhow::Error> for YadawError {
    fn from(err: anyhow::Error) -> Self {
        YadawError::Plugin(err.to_string())
    }
}

// Builder pattern for creating detailed errors
impl YadawError {
    pub fn audio(msg: impl Into<String>) -> Self {
        YadawError::Audio(msg.into())
    }

    pub fn plugin(msg: impl Into<String>) -> Self {
        YadawError::Plugin(msg.into())
    }

    pub fn file(msg: impl Into<String>) -> Self {
        YadawError::File(msg.into())
    }

    pub fn state(msg: impl Into<String>) -> Self {
        YadawError::State(msg.into())
    }
}

// Extension trait for Result types
pub trait ResultExt<T> {
    /// Show error to user via dialog
    fn notify_user(self, dialogs: &mut impl UserNotification) -> Option<T>;

    /// Log error to console
    fn log_error(self) -> Option<T>;

    /// Both notify and log
    fn handle_error(self, dialogs: &mut impl UserNotification) -> Option<T>;
}

impl<T> ResultExt<T> for Result<T> {
    fn notify_user(self, dialogs: &mut impl UserNotification) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(e) => {
                dialogs.show_error(&e.user_message());
                None
            }
        }
    }

    fn log_error(self) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(e) => {
                eprintln!("{}", e.log_message());
                None
            }
        }
    }

    fn handle_error(self, dialogs: &mut impl UserNotification) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(e) => {
                eprintln!("{}", e.log_message());
                dialogs.show_error(&e.user_message());
                None
            }
        }
    }
}

// Trait for dialog notification (to avoid circular dependency)
pub trait UserNotification {
    fn show_error(&mut self, message: &str);
    fn show_success(&mut self, message: &str);
    fn show_warning(&mut self, message: &str);
    fn show_info(&mut self, message: &str);
}

// Common error creation helpers
pub mod common {
    use super::YadawError;

    pub fn project_save_failed(e: impl std::fmt::Display) -> YadawError {
        YadawError::file(format!("Failed to save project: {}", e))
    }

    pub fn project_load_failed(e: impl std::fmt::Display) -> YadawError {
        YadawError::file(format!("Failed to load project: {}", e))
    }

    pub fn audio_import_failed(path: &std::path::Path, e: impl std::fmt::Display) -> YadawError {
        YadawError::file(format!("Failed to import {}: {}", path.display(), e))
    }

    pub fn plugin_load_failed(name: &str, e: impl std::fmt::Display) -> YadawError {
        YadawError::plugin(format!("Failed to load plugin '{}': {}", name, e))
    }

    pub fn plugin_process_error(name: &str, e: impl std::fmt::Display) -> YadawError {
        YadawError::plugin(format!("Plugin '{}' processing error: {}", name, e))
    }

    pub fn recording_error(e: impl std::fmt::Display) -> YadawError {
        YadawError::audio(format!("Recording error: {}", e))
    }

    pub fn playback_error(e: impl std::fmt::Display) -> YadawError {
        YadawError::audio(format!("Playback error: {}", e))
    }
}
