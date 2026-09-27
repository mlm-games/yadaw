//! DAWproject 1.0 interchange (ZIP of `project.xml` + `metadata.xml`).
//!
//! Yadaw has a flat, tempo-constant model: beat positions, decoded mono PCM
//! inline in the project, plugin parameter values by name. The mapping below
//! covers that model and reports everything a file cannot express.

mod export;
mod import;

pub use export::export;
pub use import::import;

pub(crate) const DAWPROJECT_VERSION: &str = "1.0";
pub(crate) const PROJECT_ENTRY: &str = "project.xml";
pub(crate) const METADATA_ENTRY: &str = "metadata.xml";
pub(crate) const AUDIO_DIR: &str = "Audio Files";

/// Why a yadaw file cannot be written as DAWproject, or what an imported file
/// lost on the way in.
#[derive(Default, Debug, Clone)]
pub struct Report {
    pub summary: String,
    pub notes: Vec<String>,
}

impl Report {
    pub fn note(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.notes.contains(&msg) {
            self.notes.push(msg);
        }
    }

    pub fn count(&mut self, msg: &str, n: usize) {
        if n > 0 {
            self.note(format!("{n} {msg}"));
        }
    }
}

pub(crate) fn beats_to_seconds(beats: f64, bpm: f64) -> f64 {
    beats * 60.0 / bpm
}

pub(crate) fn seconds_to_beats(seconds: f64, bpm: f64) -> f64 {
    seconds * bpm / 60.0
}

pub(crate) fn pan_to_normalized(pan: f32) -> f64 {
    f64::from(pan.clamp(-1.0, 1.0)) * 0.5 + 0.5
}

pub(crate) fn velocity_to_normalized(velocity: u8) -> f64 {
    f64::from(velocity) / 127.0
}

pub(crate) fn normalized_to_velocity(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 127.0).round() as u8
}

pub(crate) fn rgb_to_hex(rgb: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

pub(crate) fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

pub(crate) fn xml_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn sanitize_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if (c as u32) < 0x20 { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn num(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_sign_positive() {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    let mut s = format!("{value:.6}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s.push('0');
    }
    s
}

pub(crate) fn parse_f64(value: Option<&str>) -> Option<f64> {
    value.and_then(|v| v.trim().parse::<f64>().ok())
}

pub(crate) fn parse_bool(value: Option<&str>) -> Option<bool> {
    match value?.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// DAWproject `TimeUnit`: `beats` unless a scope says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeUnit {
    Beats,
    Seconds,
}

impl TimeUnit {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("seconds") => TimeUnit::Seconds,
            _ => TimeUnit::Beats,
        }
    }

    pub(crate) fn to_beats(self, value: f64, bpm: f64) -> f64 {
        match self {
            TimeUnit::Beats => value,
            TimeUnit::Seconds => seconds_to_beats(value, bpm),
        }
    }

    pub(crate) fn to_seconds(self, value: f64, bpm: f64) -> f64 {
        match self {
            TimeUnit::Beats => beats_to_seconds(value, bpm),
            TimeUnit::Seconds => value,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            TimeUnit::Beats => "beats",
            TimeUnit::Seconds => "seconds",
        }
    }
}

pub(crate) fn plugin_device_id(uri: &str) -> Option<&str> {
    uri.rsplit_once('#')
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
}

/// DAWproject addresses a plug-in by its own identifier, not by a path, and the
/// schema has no slot for a filesystem path. CLAP exposes a stable id after the
/// `#`; other backends fall back to the yadaw URI so the round trip is exact.
pub(crate) fn device_id(uri: &str) -> String {
    plugin_device_id(uri)
        .map(str::to_string)
        .unwrap_or_else(|| uri.to_string())
}

/// Resolves a DAWproject device to an installed yadaw plugin URI.
pub type PluginResolver<'a> =
    dyn Fn(yadaw_plugin_api::BackendKind, &str, &str) -> Option<String> + 'a;
