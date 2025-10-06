use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths::config_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub audio: AudioConfig,
    pub ui: UIConfig,
    pub paths: PathConfig,
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub buffer_size: usize,
    pub sample_rate: f32,
    pub auto_detect_audio_device: bool,
    pub preferred_output_device: Option<String>,
    pub preferred_input_device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIConfig {
    pub theme: Theme,
    pub show_tooltips: bool,
    pub auto_scroll_on_playback: bool,
    pub smooth_scrolling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    pub last_project_dir: Option<PathBuf>,
    pub plugin_scan_paths: Vec<PathBuf>,
    pub default_project_dir: Option<PathBuf>,
    pub audio_import_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    pub auto_save: bool,
    pub auto_save_interval_minutes: u32,
    pub create_backup_on_save: bool,
    pub stop_on_track_selection: bool,
    pub follow_playhead: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: AudioConfig {
                buffer_size: 512,
                sample_rate: 44100.0,
                auto_detect_audio_device: true,
                preferred_output_device: None,
                preferred_input_device: None,
            },
            ui: UIConfig {
                theme: Theme::Dark,
                show_tooltips: true,
                auto_scroll_on_playback: true,
                smooth_scrolling: true,
            },
            paths: PathConfig {
                last_project_dir: None,
                plugin_scan_paths: Self::default_plugin_paths(),
                default_project_dir: None,
                audio_import_dir: None,
            },
            behavior: BehaviorConfig {
                auto_save: false,
                auto_save_interval_minutes: 5,
                create_backup_on_save: true,
                stop_on_track_selection: false,
                follow_playhead: true,
            },
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        if let Some(config_path) = Self::config_path()
            && config_path.exists() {
                let contents = std::fs::read_to_string(config_path)?;
                return Ok(serde_json::from_str(&contents)?);
            }
        Ok(Self::default())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(config_path) = Self::config_path() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let contents = serde_json::to_string_pretty(self)?;
            std::fs::write(config_path, contents)?;
        }
        Ok(())
    }

    fn config_path() -> Option<std::path::PathBuf> {
        Some(config_path())
    }

    fn default_plugin_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Common LV2 paths on Linux
        if let Ok(home) = std::env::var("HOME") {
            paths.push(PathBuf::from(format!("{}/.lv2", home)));
            paths.push(PathBuf::from(format!("{}/.clap", home)));
        }
        paths.push(PathBuf::from("/usr/lib/lv2"));
        paths.push(PathBuf::from("/usr/local/lib/lv2"));
        paths.push(PathBuf::from("/usr/lib/clap"));
        paths.push(PathBuf::from("/usr/local/lib/clap"));

        paths
    }
}
