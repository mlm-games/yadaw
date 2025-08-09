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
