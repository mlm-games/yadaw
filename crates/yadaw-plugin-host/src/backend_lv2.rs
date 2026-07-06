use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::editor_host::{EditorBackend, EditorHost};
use yadaw_plugin_api::{
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
                has_midi: p.has_midi || p.audio_outputs == 0,
            })
            .collect();
        Ok(infos)
    }

    fn instantiate(&self, uri: &str) -> Result<Box<dyn PluginInstance>> {
        use yadaw_plugin_api::ParamKind;

        let instance = crate::plugin_host::instantiate(uri)
            .map_err(|e| anyhow!("LV2 instantiate failed: {e}"))?;

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
                kind: ParamKind::Float,
                group: None,
                is_hidden: false,
                is_readonly: false,
                is_automatable: true,
                is_bypass: false,
                unit: None,
                value_to_text: None,
            })
            .collect();

        Ok(Box::new(Lv2Instance {
            uri: uri.to_string(),
            params,
            inner: instance,
            editor_host: None,
        }))
    }
}

#[cfg(feature = "lv2-legacy")]
pub struct Lv2Instance {
    uri: String,
    params: Vec<UnifiedParamInfo>,
    inner: crate::lv2_plugin_host::LV2PluginInstance,
    editor_host: Option<EditorHost>,
}

/// Backend used by [`EditorHost`] to manage the LV2 UI lifecycle.
///
/// The UI is pre-opened by `Lv2Instance::open_editor` and handed into this
/// backend, so `try_open_floating` is a no-op that returns `true`.
struct Lv2EditorBackend {
    ui: Option<Arc<yeli::UiInstance>>,
}

impl Lv2EditorBackend {
    fn new(ui: Arc<yeli::UiInstance>) -> Self {
        Self { ui: Some(ui) }
    }
}

impl EditorBackend for Lv2EditorBackend {
    fn has_editor(&self) -> bool {
        true
    }

    fn try_open_floating(&mut self) -> Result<bool> {
        Ok(true)
    }

    fn open_embedded(&mut self, _parent_window: u32) -> Result<()> {
        Err(anyhow!("Embedded LV2 UI not yet supported"))
    }

    fn on_idle(&mut self) {
        if let Some(ref ui) = self.ui {
            if let Some(ret) = ui.idle() {
                if ret != 0 {
                    // UI requested close; let the event loop handle it via
                    // on_gui_closed, but for now just swallow.
                }
            }
        }
    }

    fn close(&mut self) -> Result<()> {
        self.ui.take();
        Ok(())
    }

    fn preferred_size(&self) -> Option<(u32, u32)> {
        None
    }
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
        if !events.is_empty() {
            let raw: Vec<(u8, u8, u8, i64)> = events
                .iter()
                .map(|e| (e.status, e.data1, e.data2, e.time_frames))
                .collect();
            self.inner.prepare_midi_raw_events(&raw);
        } else {
            self.inner.clear_midi_events();
        }

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
    }

    fn load_state(&mut self, _data: &[u8]) -> bool {
        false
    }

    fn has_editor(&self) -> bool {
        self.inner.has_editor()
    }

    fn open_editor(&mut self) -> Result<()> {
        // Re-open after close: shut down the old editor thread first
        if self.editor_host.is_some() {
            if let Some(mut old) = self.editor_host.take() {
                old.shutdown();
            }
            // Drop the old UiInstance only after the thread has exited.
            self.inner.set_active_ui(None);
        }

        crate::editor_host::silence_x11_errors();

        use crate::editor_host::x11;

        // First-time open: spawn EditorHost from scratch.
        let (xlib, display) = x11::open_display()?;
        let size = (800, 600);
        let parent_win = x11::create_parent_window(&xlib, display, size.0, size.1)?;
        eprintln!(
            "[lv2 debug] parent_win=0x{:x} size={}x{}",
            parent_win, size.0, size.1
        );

        let ui = self.inner.open_editor_with_parent(parent_win as usize)?;
        let ui = Arc::new(ui);
        self.inner.set_active_ui(Some(ui.clone()));
        self.inner.update_ui(&ui);

        let widget = ui.widget();
        let child_win = widget as u64;
        eprintln!(
            "[lv2 debug] widget={:p} child_win=0x{:x}",
            widget, child_win
        );

        unsafe {
            (xlib.XMapSubwindows)(display, parent_win);
            (xlib.XSync)(display, 0);
        }

        let wm_delete_window =
            unsafe { (xlib.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0) };
        let wm_protocols = unsafe { (xlib.XInternAtom)(display, c"WM_PROTOCOLS".as_ptr(), 0) };

        if child_win != 0 {
            eprintln!("[lv2 debug] sending expose to child");
            x11::send_expose_event(&xlib, display, child_win, size.0 as i32, size.1 as i32);
        } else {
            eprintln!("[lv2 debug] child_win is 0 – DPF did not create a window");
        }

        let state = x11::X11State {
            xlib,
            display,
            parent_win,
            child_win,
            wm_delete_window,
            wm_protocols,
            size,
            expose_tick: 0,
        };

        let backend = Lv2EditorBackend::new(ui);
        let host = EditorHost::spawn(Box::new(backend))?;
        host.open_editor_with_state(state)?;
        self.editor_host = Some(host);
        Ok(())
    }
}
