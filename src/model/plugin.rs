use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::model::plugin_api::BackendKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: u64,
    pub uri: String,
    pub name: String,
    pub backend: BackendKind,
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
