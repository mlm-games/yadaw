//! Dedicated plugin worker thread.
//!
//! All plugin instantiation, parameter application, state I/O and editor
//! management happen here (off the audio thread) and instances are handed
//! to the audio engine via [`EngineEvent::Install`].

use std::collections::HashMap;
use std::sync::Arc;

use flume::{Receiver, Sender};

use crate::audio_state::{EngineEvent, PluginDescriptorSnapshot, PluginWorkerCommand, SharedInstance};
use crate::messages::{PluginParamInfo, UIUpdate, UiTx};
use yadaw_plugin_api::{ParamKey, PluginInstance as UnifiedInstance, UnifiedParamInfo};
use yadaw_plugin_host::HostFacade;

pub struct PluginWorker {
    facade: Arc<HostFacade>,
    instances: HashMap<(u64, u64), SharedInstance>,
    engine_events_tx: Sender<EngineEvent>,
    ui_tx: UiTx,
}

impl PluginWorker {
    fn new(
        facade: Arc<HostFacade>,
        engine_events_tx: Sender<EngineEvent>,
        ui_tx: UiTx,
    ) -> Self {
        Self {
            facade,
            instances: HashMap::new(),
            engine_events_tx,
            ui_tx,
        }
    }

    fn run(mut self, command_rx: Receiver<PluginWorkerCommand>) {
        while let Ok(command) = command_rx.recv() {
            self.handle(command);
        }
        log::info!("Plugin worker shutting down");
    }

    fn handle(&mut self, command: PluginWorkerCommand) {
        match command {
            PluginWorkerCommand::AddPlugin {
                track_id,
                plugin_id,
                plugin_idx,
                backend,
                uri,
                params,
            } => {
                let key = (track_id, plugin_id);
                if let Some(instance) = self.instances.get_mut(&key) {
                    let mut guard = instance.lock();
                    apply_params(&mut **guard, &params);
                    return;
                }

                match self.facade.instantiate(backend, &uri) {
                    Ok(instance) => {
                        let instance = Arc::new(parking_lot::Mutex::new(instance));
                        {
                            let mut guard = instance.lock();
                            apply_params(&mut **guard, &params);
                        }
                        let has_editor = instance.lock().has_editor();
                        let params_for_ui = describe_params(&**instance.lock());
                        let _ = self.engine_events_tx.send(EngineEvent::Install {
                            track_id,
                            plugin_id,
                            instance: instance.clone(),
                        });
                        self.instances.insert(key, instance);
                        let _ = self.ui_tx.send_sync(UIUpdate::PluginParamsDiscovered {
                            track_id,
                            plugin_idx,
                            has_editor,
                            params: params_for_ui,
                        });
                    }
                    Err(e) => {
                        let msg = format!("Failed to instantiate plugin {uri}: {e}");
                        log::error!("{msg}");
                        let _ = self.ui_tx.send_sync(UIUpdate::Error(msg));
                    }
                }
            }
            PluginWorkerCommand::SetParams {
                track_id,
                plugin_id,
                params,
            } => {
                if let Some(instance) = self.instances.get_mut(&(track_id, plugin_id)) {
                    let mut guard = instance.lock();
                    apply_params(&mut **guard, &params);
                }
            }
            PluginWorkerCommand::SetParam {
                track_id,
                plugin_id,
                key,
                value,
            } => {
                if let Some(instance) = self.instances.get_mut(&(track_id, plugin_id)) {
                    let mut guard = instance.lock();
                    set_param_by_name(&mut **guard, &key, value);
                }
            }
            PluginWorkerCommand::RemovePlugin {
                track_id,
                plugin_id,
            } => {
                if self.instances.remove(&(track_id, plugin_id)).is_some() {
                    let _ = self.engine_events_tx.send(EngineEvent::Uninstall {
                        track_id,
                        plugin_id,
                    });
                }
            }
            PluginWorkerCommand::OpenEditor {
                track_id,
                plugin_id,
            } => {
                let Some(instance) = self.instances.get_mut(&(track_id, plugin_id)) else {
                    return;
                };
                let mut guard = instance.lock();
                if let Err(e) = guard.open_editor() {
                    let msg = format!("Failed to open editor: {e}");
                    log::error!("{msg}");
                    let _ = self.ui_tx.send_sync(UIUpdate::Error(msg));
                }
            }
            PluginWorkerCommand::RebuildChain { track_id, chain } => {
                for desc in chain {
                    self.ensure_chain_entry(track_id, desc);
                }
            }
        }
    }

    /// Get-or-create a worker entry for a chain plugin (used after project
    /// load, where chain plugins may not have gone through `AddPlugin`).
    fn ensure_chain_entry(&mut self, track_id: u64, desc: PluginDescriptorSnapshot) {
        let key = (track_id, desc.plugin_id);
        let params: Vec<(String, f32)> = desc
            .params
            .iter()
            .map(|kv| (kv.key().clone(), *kv.value()))
            .collect();

        if let Some(instance) = self.instances.get_mut(&key) {
            let mut guard = instance.lock();
            apply_params(&mut **guard, &params);
            return;
        }

        match self.facade.instantiate(desc.backend, &desc.uri) {
            Ok(instance) => {
                let instance = Arc::new(parking_lot::Mutex::new(instance));
                {
                    let mut guard = instance.lock();
                    apply_params(&mut **guard, &params);
                }
                let has_editor = instance.lock().has_editor();
                let params_for_ui = describe_params(&**instance.lock());
                let _ = self.engine_events_tx.send(EngineEvent::Install {
                    track_id,
                    plugin_id: desc.plugin_id,
                    instance: instance.clone(),
                });
                self.instances.insert(key, instance);
                let _ = self.ui_tx.send_sync(UIUpdate::PluginParamsDiscovered {
                    track_id,
                    plugin_idx: 0,
                    has_editor,
                    params: params_for_ui,
                });
            }
            Err(e) => {
                let msg = format!(
                    "Failed to load plugin '{}' on track {}: {}",
                    desc.name, track_id, e
                );
                log::error!("{msg}");
                let _ = self.ui_tx.send_sync(UIUpdate::Error(msg));
            }
        }
    }
}

fn apply_params(instance: &mut dyn UnifiedInstance, params: &[(String, f32)]) {
    let name_to_key: HashMap<String, ParamKey> = instance
        .params()
        .iter()
        .map(|p| (p.name.clone(), p.key.clone()))
        .collect();

    for (name, value) in params {
        let key = match name_to_key.get(name) {
            Some(k) => k.clone(),
            None => ParamKey::Lv2(name.clone()),
        };
        instance.set_param(&key, *value);
    }
}

fn set_param_by_name(instance: &mut dyn UnifiedInstance, name: &str, value: f32) {
    let key = instance
        .params()
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.key.clone())
        .unwrap_or_else(|| ParamKey::Lv2(name.to_string()));
    instance.set_param(&key, value);
}

fn describe_params(instance: &dyn UnifiedInstance) -> Vec<PluginParamInfo> {
    instance
        .params()
        .iter()
        .map(|p: &UnifiedParamInfo| {
            let current = instance.get_param(&p.key).unwrap_or(p.default);
            PluginParamInfo {
                name: p.name.clone(),
                min: p.min,
                max: p.max,
                default: p.default,
                current,
                kind: p.kind,
                enum_labels: p.enum_labels.clone(),
                group: p.group.clone(),
                is_hidden: p.is_hidden,
                is_readonly: p.is_readonly,
                is_automatable: p.is_automatable,
                unit: p.unit.clone(),
                display_text: p.value_to_text.clone(),
            }
        })
        .collect()
}

/// Spawn the plugin worker on a dedicated thread.
///
/// - Native: a real OS thread.
/// - wasm: a worker thread from the rayon pool initialized at startup
///   (`init_thread_pool`). The wasm build has no plugin backends, so the
///   worker is effectively inert there, but keeping the same architecture
///   avoids divergence.
pub fn spawn_plugin_worker(
    facade: Arc<HostFacade>,
    command_rx: Receiver<PluginWorkerCommand>,
    engine_events_tx: Sender<EngineEvent>,
    ui_tx: UiTx,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("plugin-worker".into())
            .spawn(move || PluginWorker::new(facade, engine_events_tx, ui_tx).run(command_rx))
            .expect("Failed to spawn plugin worker thread");
    }
    #[cfg(target_arch = "wasm32")]
    {
        rayon::spawn(move || PluginWorker::new(facade, engine_events_tx, ui_tx).run(command_rx));
    }
}