use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths::config_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    pub metronome_enabled: bool,
    pub click_volume: f32,
    pub count_in_enabled: bool,
    pub count_in_bars: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub ui: UIConfig,
    #[serde(default)]
    pub paths: PathConfig,
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f32,
    #[serde(default = "default_true")]
    pub auto_detect_audio_device: bool,
    #[serde(default)]
    pub preferred_output_device: Option<String>,
    #[serde(default)]
    pub preferred_input_device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIConfig {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_true")]
    pub show_tooltips: bool,
    #[serde(default = "default_true")]
    pub auto_scroll_on_playback: bool,
    #[serde(default = "default_true")]
    pub smooth_scrolling: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    #[serde(default)]
    pub last_project_dir: Option<PathBuf>,
    #[serde(default)]
    pub plugin_scan_paths: Vec<PathBuf>,
    #[serde(default)]
    pub default_project_dir: Option<PathBuf>,
    #[serde(default)]
    pub audio_import_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    #[serde(default)]
    pub auto_save: bool,
    #[serde(default = "default_autosave_interval")]
    pub auto_save_interval_minutes: u32,
    #[serde(default = "default_true")]
    pub create_backup_on_save: bool,
    #[serde(default)]
    pub stop_on_track_selection: bool,
    #[serde(default = "default_true")]
    pub follow_playhead: bool,
}

fn default_buffer_size() -> usize {
    512
}
fn default_sample_rate() -> f32 {
    44100.0
}
fn default_true() -> bool {
    true
}
fn default_autosave_interval() -> u32 {
    5
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Dark
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            buffer_size: default_buffer_size(),
            sample_rate: default_sample_rate(),
            auto_detect_audio_device: true,
            preferred_output_device: None,
            preferred_input_device: None,
        }
    }
}

impl Default for UIConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            show_tooltips: true,
            auto_scroll_on_playback: true,
            smooth_scrolling: true,
        }
    }
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            last_project_dir: None,
            plugin_scan_paths: Config::default_plugin_paths(),
            default_project_dir: None,
            audio_import_dir: None,
        }
    }
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            auto_save: false,
            auto_save_interval_minutes: default_autosave_interval(),
            create_backup_on_save: true,
            stop_on_track_selection: false,
            follow_playhead: true,
        }
    }
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
        if let Some(data) = crate::wasm_persist::read_config_string(
            crate::paths::opfs::FILE_CONFIG,
            &Self::config_path().unwrap_or_default(),
        ) {
            match serde_json::from_str::<Self>(&data) {
                Ok(mut cfg) => {
                    cfg.validate();
                    return Ok(cfg);
                }
                Err(e) => {
                    log::warn!("Corrupt config.json ({}); backing up and using defaults", e);
                    let _ = Self::backup_corrupt(&data);
                }
            }
        }
        Ok(Self::default())
    }

    /// Write the unparseable bytes aside before defaults overwrite them.
    fn backup_corrupt(data: &str) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = crate::wasm_persist::save_config_string(
                "config/config.json.corrupt",
                &std::path::PathBuf::new(),
                data,
            );
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = Self::config_path().unwrap_or_default();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let backup = path.with_extension("json.corrupt");
            std::fs::write(backup, data)?;
            Ok(())
        }
    }

    /// Clamp invalid values from corrupt/hand-edited configs so they can't
    /// poison TimeConverter, stream setup, or autosave timers.
    pub fn validate(&mut self) {
        if !self.audio.buffer_size.is_power_of_two()
            || !(64..=8192).contains(&self.audio.buffer_size)
        {
            self.audio.buffer_size = 512;
        }
        if !self.audio.sample_rate.is_finite() || self.audio.sample_rate <= 0.0 {
            self.audio.sample_rate = 44100.0;
        }
        if self.behavior.auto_save_interval_minutes == 0
            || self.behavior.auto_save_interval_minutes > 120
        {
            self.behavior.auto_save_interval_minutes = 5;
        }
    }

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        crate::wasm_persist::save_config_string(
            crate::paths::opfs::FILE_CONFIG,
            &Self::config_path().unwrap_or_default(),
            &json,
        )?;
        Ok(())
    }

    fn config_path() -> Option<std::path::PathBuf> {
        Some(config_path())
    }

    fn default_plugin_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // for Flatpak envs mainly
        if let Ok(path_str) = std::env::var("CLAP_PATH") {
            for path in path_str.split(':') {
                let p = PathBuf::from(path);
                if p.exists() {
                    paths.push(p);
                }
            }
        }

        if let Ok(path_str) = std::env::var("LV2_PATH") {
            for path in path_str.split(':') {
                let p = PathBuf::from(path);
                if p.exists() {
                    paths.push(p);
                }
            }
        }

        if let Ok(home) = std::env::var("HOME") {
            paths.push(PathBuf::from(format!("{}/.lv2", home)));
            paths.push(PathBuf::from(format!("{}/.clap", home)));
        }
        paths.push(PathBuf::from("/usr/lib/lv2"));
        paths.push(PathBuf::from("/usr/local/lib/lv2"));
        paths.push(PathBuf::from("/usr/lib/clap"));
        paths.push(PathBuf::from("/usr/local/lib/clap"));

        paths.sort();
        paths.dedup();

        paths
    }
}
