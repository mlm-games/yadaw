use crate::state::{PluginDescriptor, PluginParam};
use anyhow::Result;
use lilv;
use lilv::port::Port;
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
        self.world.load_all();
        let all_plugins = self.world.plugins();

        for plugin in all_plugins.iter() {
            let uri = plugin.uri().as_uri().unwrap_or_default().to_string();
            let name = plugin.name().as_str().unwrap_or_default().to_string();

            let class = plugin
                .class()
                .uri()
                .expect("Plugin class URI")
                .as_uri()
                .unwrap_or_default()
                .to_string();

            let category = match class.as_str() {
                s if s.contains("Instrument") => PluginCategory::Instrument,
                s if s.contains("Generator") => PluginCategory::Generator,
                s if s.contains("Analyzer") => PluginCategory::Analyzer,
                s if s.contains("Effect") => PluginCategory::Effect,
                _ => PluginCategory::Unknown,
            };

            let info = PluginInfo {
                uri: uri.clone(),
                name,
                category,
                path: PathBuf::new(),
            };
            self.plugins.insert(uri, info);
        }
    }

    pub fn get_plugins(&self) -> Vec<PluginInfo> {
        self.plugins.values().cloned().collect()
    }
}

/// Create a plugin descriptor from URI
pub fn create_plugin_instance(uri: &str, sample_rate: f32) -> Result<PluginDescriptor> {
    let world = lilv::World::new();
    world.load_all();

    let plugin = world
        .plugins()
        .iter()
        .find(|p| p.uri().as_uri().unwrap_or("") == uri)
        .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", uri))?;

    let plugin_name = plugin.name().as_str().unwrap_or(uri).to_string();
    let mut params = HashMap::new();

    // Scan ports for parameters
    for port in plugin.iter_ports() {
        let index = port.index() as usize;

        if port.is_a(&world.new_uri("http://lv2plug.in/ns/lv2core#ControlPort"))
            && port.is_a(&world.new_uri("http://lv2plug.in/ns/lv2core#InputPort"))
        {
            let name = port
                .name()
                .expect("Port name")
                .as_str()
                .unwrap_or("")
                .to_string();

            // Get port ranges
            let (default, min, max) = get_port_range(&world, port);

            params.insert(
                name.clone(),
                PluginParam {
                    index,
                    name: name.clone(),
                    value: default,
                    min,
                    max,
                    default,
                },
            );
        }
    }

    Ok(PluginDescriptor {
        uri: uri.to_string(),
        name: plugin_name,
        bypass: false,
        params,
    })
}

fn get_port_range(world: &lilv::World, port: Port) -> (f32, f32, f32) {
    let default_uri = world.new_uri("http://lv2plug.in/ns/lv2core#default");
    let minimum_uri = world.new_uri("http://lv2plug.in/ns/lv2core#minimum");
    let maximum_uri = world.new_uri("http://lv2plug.in/ns/lv2core#maximum");

    let default = port
        .get(&default_uri)
        .and_then(|node| node.as_float())
        .unwrap_or(0.5);

    let min = port
        .get(&minimum_uri)
        .and_then(|node| node.as_float())
        .unwrap_or(0.0);

    let max = port
        .get(&maximum_uri)
        .and_then(|node| node.as_float())
        .unwrap_or(1.0);

    (default, min, max)
}
