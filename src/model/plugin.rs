use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub uri: String,
    pub name: String,
    pub bypass: bool,
    pub params: HashMap<String, f32>,
    pub preset_name: Option<String>,
    pub custom_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginParam {
    pub index: usize,
    pub name: String,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}
