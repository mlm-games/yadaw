use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::model::plugin_api::BackendKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreset {
    pub uri: String,
    pub backend: BackendKind,
    pub name: String,
    pub params: HashMap<String, f32>,
}

fn sanitize(input: &str) -> String {
    input
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn preset_dir_for_uri(uri: &str) -> std::path::PathBuf {
    crate::paths::presets_dir().join(sanitize(uri))
}

fn preset_path(uri: &str, name: &str) -> std::path::PathBuf {
    preset_dir_for_uri(uri).join(format!("{}.json", sanitize(name)))
}

pub fn save_preset(preset: &PluginPreset) -> Result<()> {
    let dir = preset_dir_for_uri(&preset.uri);
    std::fs::create_dir_all(&dir)?;
    let path = preset_path(&preset.uri, &preset.name);
    let json = serde_json::to_string_pretty(preset)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_preset(uri: &str, name: &str) -> Result<PluginPreset> {
    let path = preset_path(uri, name);
    if !path.exists() {
        return Err(anyhow!("Preset not found: {} ({})", name, uri));
    }
    let txt = std::fs::read_to_string(path)?;
    let preset: PluginPreset = serde_json::from_str(&txt)?;
    Ok(preset)
}

pub fn list_presets_for(uri: &str) -> Vec<String> {
    let dir = preset_dir_for_uri(uri);
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}
