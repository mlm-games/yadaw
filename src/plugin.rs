use anyhow::Result;
use lilv;
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub uri: String,
    pub name: String,
    pub category: PluginCategory,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginCategory {
    Instrument,
    Effect,
    Generator,
    Analyzer,
    Unknown,
}

pub struct PluginScanner {
    world: lilv::World,
    plugins: HashMap<String, PluginInfo>,
}

impl PluginScanner {
    pub fn new() -> Self {
        let world = lilv::World::new();
        Self {
            world,
            plugins: HashMap::new(),
        }
    }

    pub fn discover_plugins(&mut self) {
        // Load all available LV2 bundles on the system
        self.world.load_all();
        let all_plugins = self.world.plugins();

        for plugin in all_plugins.iter() {
            let uri = plugin.uri().as_uri().unwrap_or_default().to_string();
            let name = plugin.name().as_str().unwrap_or_default().to_string();

            // Get the plugin class (like Effect, Instrument, etc.)
            let class = plugin
                .class()
                .uri()
                .expect("Yo noobs")
                .as_uri()
                .unwrap_or_default()
                .to_string();
            // We can simplify the class string for display
            let simplified_class = class.split('#').last().unwrap_or(&class).to_string();

            let info = PluginInfo {
                uri: uri.clone(),
                name,
                category: todo!(),
                path: todo!(),
                // class: simplified_class,
            };
            self.plugins.insert(uri, info);
        }
    }

    pub fn get_plugins(&self) -> Vec<PluginInfo> {
        self.plugins.values().cloned().collect()
    }
}
