use anyhow::{Result, anyhow};

use crate::model::plugin_api::{
    BackendKind, HostConfig, MidiEvent, ParamKey, PluginBackend, PluginInstance, ProcessCtx,
    UnifiedParamInfo, UnifiedPluginInfo,
};

#[cfg(feature = "lv2-legacy")]
pub struct Lv2HostBackend;

#[cfg(feature = "lv2-legacy")]
impl Default for Lv2HostBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Lv2HostBackend {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "lv2-legacy")]
impl PluginBackend for Lv2HostBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Lv2
    }

    fn init(&self, cfg: &HostConfig) -> Result<()> {
        // Use your global facade to init LV2 host
        crate::plugin_host::ensure(cfg.sample_rate, cfg.max_block)?;
        Ok(())
    }

    fn scan(&self) -> Result<Vec<UnifiedPluginInfo>> {
        let list = crate::plugin_host::get_available_plugins()?;
        let infos = list
            .into_iter()
            .map(|p| UnifiedPluginInfo {
                backend: BackendKind::Lv2,
                uri: p.uri.clone(),
                name: p.name.clone(),
                is_instrument: p.is_instrument,
                audio_inputs: p.audio_inputs,
                audio_outputs: p.audio_outputs,
                has_midi: p.has_midi || p.audio_outputs == 0, // common LV2 instruments pattern
            })
            .collect();
        Ok(infos)
    }

    fn instantiate(&self, uri: &str) -> Result<Box<dyn PluginInstance>> {
        // Instantiate LV2 instance using your existing API
        let instance = crate::plugin_host::instantiate(uri)
            .map_err(|e| anyhow!("LV2 instantiate failed: {e}"))?;

        // Build parameter metadata once from the scan results
        let list = crate::plugin_host::get_available_plugins()?;
        let info = list
            .into_iter()
            .find(|p| p.uri == uri)
            .ok_or_else(|| anyhow!("Plugin not found in cache: {uri}"))?;

        let params: Vec<UnifiedParamInfo> = info
            .control_ports
            .iter()
            .map(|cp| UnifiedParamInfo {
                key: ParamKey::Lv2(cp.symbol.clone()),
                name: cp.name.clone(),
                min: cp.min,
                max: cp.max,
                default: cp.default,
                stepped: false,
                enum_labels: None,
            })
            .collect();

        Ok(Box::new(Lv2Instance {
            uri: uri.to_string(),
            params,
            inner: instance,
        }))
    }
}

#[cfg(feature = "lv2-legacy")]
pub struct Lv2Instance {
    uri: String,
    params: Vec<UnifiedParamInfo>,
    inner: crate::lv2_plugin_host::LV2PluginInstance,
}

#[cfg(feature = "lv2-legacy")]
impl PluginInstance for Lv2Instance {
    fn process(
        &mut self,
        ctx: &ProcessCtx,
        audio_in: &[&[f32]],
        audio_out: &mut [&mut [f32]],
        events: &[MidiEvent],
    ) -> Result<()> {
        // Prepare MIDI into LV2AtomSequence only if the plugin expects it
        if !events.is_empty() {
            // Convert to raw bytes; your LV2 instance already exposes a helper
            let raw: Vec<(u8, u8, u8, i64)> = events
                .iter()
                .map(|e| (e.status, e.data1, e.data2, e.time_frames))
                .collect();
            self.inner.prepare_midi_raw_events(&raw);
        } else {
            self.inner.clear_midi_events();
        }

        // Run. We do not retain mutable borrows across this call.
        self.inner
            .process_multi(audio_in, audio_out, !events.is_empty(), ctx.frames)?;
        Ok(())
    }

    fn set_param(&mut self, key: &ParamKey, value: f32) {
        if let ParamKey::Lv2(sym) = key {
            self.inner.set_parameter(sym, value);
        }
    }

    fn get_param(&self, key: &ParamKey) -> Option<f32> {
        match key {
            ParamKey::Lv2(sym) => self.inner.get_parameter(sym),
            _ => None,
        }
    }

    fn params(&self) -> &[UnifiedParamInfo] {
        &self.params
    }

    fn save_state(&mut self) -> Option<Vec<u8>> {
        None
    } // optional for LV2
    fn load_state(&mut self, _data: &[u8]) -> bool {
        false
    }
}
