use eframe::egui;

// Audio Engine Constants
pub const MAX_BUFFER_SIZE: usize = 8192;
pub const RECORDING_BUFFER_SIZE: usize = 44100 * 60 * 5; // 5 minutes at 44.1kHz
pub const DEFAULT_SAMPLE_RATE: f64 = 44100.0;
pub const DEFAULT_BPM: f32 = 120.0;
pub const DEFAULT_MASTER_VOLUME: f32 = 0.8;
pub const DEFAULT_TRACK_VOLUME: f32 = 0.7;

// UI Layout Constants
pub const PIANO_KEY_WIDTH: f32 = 60.0;
pub const TRACK_HEIGHT: f32 = 80.0;
pub const AUTOMATION_LANE_HEIGHT: f32 = 30.0;
pub const TRACK_PANEL_WIDTH: f32 = 300.0;
pub const MIXER_STRIP_WIDTH: f32 = 80.0;
pub const MIXER_DEFAULT_WIDTH: f32 = 600.0;
pub const MIXER_DEFAULT_HEIGHT: f32 = 400.0;

// Timeline Constants
pub const DEFAULT_TIMELINE_ZOOM: f32 = 100.0; // pixels per beat
pub const MIN_TIMELINE_ZOOM: f32 = 10.0;
pub const MAX_TIMELINE_ZOOM: f32 = 500.0;
pub const TIMELINE_ZOOM_FACTOR: f32 = 1.25;

// Piano Roll Constants
pub const DEFAULT_GRID_SNAP: f32 = 0.25; // 1/16th notes
pub const DEFAULT_PIANO_ZOOM_X: f32 = 100.0;
pub const DEFAULT_PIANO_ZOOM_Y: f32 = 20.0;
pub const PIANO_ZOOM_MIN: f32 = 10.0;
pub const PIANO_ZOOM_MAX: f32 = 500.0;
pub const MIDDLE_C_SCROLL_Y: f32 = 60.0 * 20.0;

// Interaction Constants
pub const EDGE_RESIZE_THRESHOLD: f32 = 5.0;
pub const NOTE_EDGE_THRESHOLD: f32 = 8.0;
pub const UNDO_STACK_LIMIT: usize = 100;

// Audio Processing Constants
pub const PREVIEW_NOTE_DURATION: f64 = 0.5; // seconds
pub const PREVIEW_NOTE_AMPLITUDE: f32 = 0.3;
pub const SINE_WAVE_AMPLITUDE: f32 = 0.1;
pub const NORMALIZE_TARGET_DB: f32 = -0.1; // dB
pub const NORMALIZE_TARGET_LINEAR: f32 = 0.989;
pub const SILENCE_THRESHOLD: f32 = 0.001; // -60dB

// Channel Configuration
pub const CHANNEL_QUEUE_SIZE: usize = 256;

// Colors
pub const COLOR_TRACK_BG_EVEN: egui::Color32 = egui::Color32::from_gray(25);
pub const COLOR_TRACK_BG_ODD: egui::Color32 = egui::Color32::from_gray(30);
pub const COLOR_GRID_BEAT: egui::Color32 = egui::Color32::from_gray(60);
pub const COLOR_GRID_SUBDIVISION: egui::Color32 = egui::Color32::from_gray(40);
pub const COLOR_PLAYHEAD: egui::Color32 = egui::Color32::from_rgb(255, 100, 100);
pub const COLOR_AUTOMATION_LINE: egui::Color32 = egui::Color32::from_rgb(100, 150, 255);
pub const COLOR_AUTOMATION_POINT: egui::Color32 = egui::Color32::from_rgb(150, 180, 255);
pub const COLOR_AUTOMATION_POINT_SELECTED: egui::Color32 = egui::Color32::from_rgb(255, 200, 100);

// Default Track Names
pub const DEFAULT_AUDIO_TRACK_PREFIX: &str = "Audio";
pub const DEFAULT_MIDI_TRACK_PREFIX: &str = "MIDI";
pub const DEFAULT_PATTERN_NAME: &str = "Pattern";
pub const DEFAULT_PROJECT_NAME: &str = "Untitled Project";

// File Extensions
pub const PROJECT_EXTENSION: &str = "yadaw";
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "ogg"];

// Others
pub const MIDI_TIMING_SAMPLE_RATE: f32 = 44100.0; // Might need to change later
pub const DEBUG_PLUGIN_AUDIO: bool = cfg!(debug_assertions);

pub const DEFAULT_MIN_PROJECT_BEATS: f64 = 64.0;
pub const DEFAULT_MIDI_CLIP_LEN: f64 = 4.0;
