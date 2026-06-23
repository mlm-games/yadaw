#[cfg(feature = "clap-host")]
mod clap_impl {
    use anyhow::{Result, anyhow};
    #[cfg(feature = "clap-host")]
    use clack_host::utils::Cookie;
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::mem;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::Path;
    #[cfg(unix)]
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    #[cfg(feature = "clap-host")]
    use clack_host::entry::PluginEntry;
    #[cfg(feature = "clap-host")]
    use clack_host::events::event_types::{NoteOffEvent, NoteOnEvent, ParamValueEvent};
    #[cfg(feature = "clap-host")]
    use clack_host::prelude::*;
    #[cfg(feature = "clap-host")]
    use clack_host::process::StartedPluginAudioProcessor;

    #[cfg(feature = "clap-host")]
    use clack_extensions::gui::{
        GuiApiType, GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui,
    };
    use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
    #[cfg(feature = "clap-host")]
    use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams as ParamsExt};
    use clack_extensions::timer::{HostTimer, HostTimerImpl, PluginTimer, TimerId};

    #[cfg(unix)]
    use x11_dl::xlib;

    #[cfg(not(unix))]
    mod xlib {
        pub struct Xlib;
        impl Xlib {
            pub fn open() -> Result<Xlib, ()> {
                Err(())
            }
        }
    }

    use yadaw_plugin_api::{
        BackendKind, HostConfig, MidiEvent, ParamKey, ParamKind, PluginBackend,
        PluginInstance as UniInstance, ProcessCtx, UnifiedParamInfo, UnifiedPluginInfo,
    };

    struct MyHostShared {
        callback_requested: Arc<AtomicBool>,
        gui_cmd_tx: mpsc::Sender<MainThreadCommand>,
    }

    impl<'a> SharedHandler<'a> for MyHostShared {
        fn request_restart(&self) {}
        fn request_process(&self) {}
        fn request_callback(&self) {
            self.callback_requested.store(true, Ordering::SeqCst);
        }
    }

    impl HostGuiImpl for MyHostShared {
        fn resize_hints_changed(&self) {}

        fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
            self.gui_cmd_tx
                .send(MainThreadCommand::RequestResize(new_size))?;
            Ok(())
        }

        fn request_show(&self) -> Result<(), HostError> {
            Ok(())
        }

        fn request_hide(&self) -> Result<(), HostError> {
            Ok(())
        }

        fn closed(&self, _was_destroyed: bool) {
            self.gui_cmd_tx.send(MainThreadCommand::GuiClosed).ok();
        }
    }

    impl HostLogImpl for MyHostShared {
        fn log(&self, severity: LogSeverity, message: &str) {
            match severity {
                LogSeverity::Debug | LogSeverity::Info => log::info!("[plugin] {message}"),
                LogSeverity::Warning | LogSeverity::HostMisbehaving => {
                    log::warn!("[plugin] {message}")
                }
                LogSeverity::Error | LogSeverity::Fatal | LogSeverity::PluginMisbehaving => {
                    log::error!("[plugin] {message}")
                }
            }
        }
    }

    struct TimerState {
        timers: Vec<(TimerId, u32, Instant)>,
        next_id: u32,
    }

    struct MyHostMainThread {
        timer_state: Arc<Mutex<TimerState>>,
    }

    impl<'a> MainThreadHandler<'a> for MyHostMainThread {}

    impl HostTimerImpl for MyHostMainThread {
        fn register_timer(&mut self, period_ms: u32) -> Result<TimerId, HostError> {
            let mut state = self.timer_state.lock().unwrap();
            let id = TimerId(state.next_id);
            state.next_id += 1;
            state.timers.push((id, period_ms, Instant::now()));
            Ok(id)
        }

        fn unregister_timer(&mut self, timer_id: TimerId) -> Result<(), HostError> {
            let mut state = self.timer_state.lock().unwrap();
            state.timers.retain(|(id, _, _)| *id != timer_id);
            Ok(())
        }
    }

    struct MyHost;
    impl HostHandlers for MyHost {
        type Shared<'a> = MyHostShared;
        type MainThread<'a> = MyHostMainThread;
        type AudioProcessor<'a> = ();

        fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
            builder
                .register::<HostGui>()
                .register::<HostLog>()
                .register::<HostTimer>();
        }
    }

    pub struct ClapHostBackend {
        cfg: HostConfig,
        host_info: HostInfo,
    }

    impl ClapHostBackend {
        pub fn new(cfg: HostConfig) -> Result<Self> {
            let host_info = HostInfo::new("YADAW", "YADAW", "https://example.invalid", "0.1.0")?;
            Ok(Self { cfg, host_info })
        }

        fn enumerate_bundle(path: &Path, out: &mut Vec<UnifiedPluginInfo>) {
            let mut libs = Vec::new();
            if path.is_file() {
                libs.push(path.to_path_buf());
            } else if path.is_dir()
                && let Ok(rd) = std::fs::read_dir(path)
            {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e == "so" || e == "dylib" || e == "dll")
                        .unwrap_or(false)
                    {
                        libs.push(p);
                    }
                }
            }

            for lib in libs {
                unsafe {
                    if let Ok(entry) = PluginEntry::load(lib.to_string_lossy().as_ref())
                        && let Some(factory) = entry.get_plugin_factory()
                    {
                        for d in factory.plugin_descriptors() {
                            let name = d
                                .name()
                                .map(|n| n.to_string_lossy().to_string())
                                .or_else(|| {
                                    lib.file_stem()
                                        .and_then(|s| s.to_str())
                                        .map(|s| s.to_string())
                                })
                                .unwrap_or_else(|| "Unknown CLAP".to_string());

                            let id = d
                                .id()
                                .map(|id| id.to_string_lossy().to_string())
                                .unwrap_or_else(|| "unknown_id".to_string());

                            let is_instr =
                                d.features().any(|f| f.to_string_lossy() == "instrument");

                            let (audio_inputs, audio_outputs) =
                                if is_instr { (0, 2) } else { (2, 2) };

                            out.push(UnifiedPluginInfo {
                                backend: BackendKind::Clap,
                                uri: format!("file://{}#{}", lib.display(), id),
                                name,
                                is_instrument: is_instr,
                                audio_inputs,
                                audio_outputs,
                                has_midi: true,
                            });
                        }
                    }
                }
            }
        }

        pub fn parse_uri(uri: &str) -> Result<(String, String)> {
            let (path, id) = uri.split_once('#').ok_or_else(|| {
                anyhow!("CLAP URI must be file:///.../lib.so#plugin_id, got: {uri}")
            })?;
            let path = path.strip_prefix("file://").unwrap_or(path).to_string();
            Ok((path, id.to_string()))
        }

        fn detect_kind_labels_and_unit(
            params_ext: &ParamsExt,
            plugin: &mut PluginMainThreadHandle<'_>,
            info: &clack_extensions::params::ParamInfo<'_>,
        ) -> (ParamKind, Option<Vec<String>>, Option<String>) {
            let min = info.min_value;
            let max = info.max_value;
            let default = info.default_value;
            let stepped = info.flags.contains(ParamInfoFlags::IS_STEPPED);

            let unit = Self::extract_unit(params_ext, plugin, info.id, default);

            if !stepped {
                return (ParamKind::Float, None, unit);
            }

            let range = (max - min).round() as i32;

            if range == 1 && min.round() as i32 == 0 && max.round() as i32 == 1 {
                return (ParamKind::Bool, None, unit);
            }

            const MAX_ENUM_STEPS: i32 = 1024;
            if range > 0 && range <= MAX_ENUM_STEPS {
                let mut labels = Vec::with_capacity((range + 1) as usize);
                let mut all_valid = true;
                let mut all_numeric = true;

                for step in 0..=range {
                    let value = min + step as f64;
                    let mut buf = [0u8; 256];

                    match params_ext.value_to_text(plugin, info.id, value, &mut buf) {
                        Ok(bytes) => {
                            let text = String::from_utf8_lossy(bytes)
                                .trim_end_matches('\0')
                                .trim()
                                .to_string();
                            if text.is_empty() {
                                all_valid = false;
                                break;
                            }
                            if text.parse::<f64>().is_err() {
                                all_numeric = false;
                            }
                            labels.push(text);
                        }
                        Err(_) => {
                            all_valid = false;
                            break;
                        }
                    }
                }

                if all_valid && labels.len() == (range + 1) as usize && !all_numeric {
                    return (ParamKind::Enum, Some(labels), None);
                }
            }

            (ParamKind::Int, None, unit)
        }

        fn extract_unit(
            params_ext: &ParamsExt,
            plugin: &mut PluginMainThreadHandle<'_>,
            param_id: ClapId,
            value: f64,
        ) -> Option<String> {
            let mut buf = [0u8; 256];

            let bytes = params_ext
                .value_to_text(plugin, param_id, value, &mut buf)
                .ok()?;
            let text = String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .to_string();

            if let Some(pos) = text.find(|c: char| c.is_alphabetic() || c == '%' || c == '×') {
                let unit = text[pos..].trim().to_string();
                if !unit.is_empty() {
                    return Some(format!(" {}", unit));
                }
            }

            if let Some(pos) = text.rfind(' ') {
                let suffix = &text[pos..];
                if suffix
                    .chars()
                    .skip(1)
                    .any(|c| c.is_alphabetic() || c == '%')
                {
                    return Some(suffix.to_string());
                }
            }

            None
        }

        fn format_value(
            params_ext: &ParamsExt,
            plugin: &mut PluginMainThreadHandle<'_>,
            param_id: ClapId,
            value: f64,
        ) -> Option<String> {
            let mut buf = [0u8; 256];

            params_ext
                .value_to_text(plugin, param_id, value, &mut buf)
                .ok()
                .map(|bytes| {
                    String::from_utf8_lossy(bytes)
                        .trim_end_matches('\0')
                        .to_string()
                })
        }

        fn fetch_params_with_values(
            instance: &mut PluginInstance<MyHost>,
        ) -> (Vec<UnifiedParamInfo>, HashMap<u32, f32>) {
            let Some(params_ext) = instance.plugin_shared_handle().get_extension::<ParamsExt>()
            else {
                return (Vec::new(), HashMap::new());
            };

            let mut plugin = instance.plugin_handle();
            let count = params_ext.count(&mut plugin);
            let mut param_infos = Vec::with_capacity(count as usize);
            let mut param_values = HashMap::with_capacity(count as usize);

            for i in 0..count {
                let mut buf = ParamInfoBuffer::new();
                if let Some(info) = params_ext.get_info(&mut plugin, i, &mut buf) {
                    let id = info.id.get();
                    let flags = info.flags;

                    if flags.contains(ParamInfoFlags::IS_HIDDEN) {
                        continue;
                    }

                    let name = std::str::from_utf8(info.name)
                        .ok()
                        .and_then(|s| s.split('\0').next().map(str::to_string))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("Param {}", id));

                    let min = info.min_value as f32;
                    let max = info.max_value as f32;
                    let default = info.default_value as f32;
                    let stepped = flags.contains(ParamInfoFlags::IS_STEPPED);

                    let group = std::str::from_utf8(info.module)
                        .ok()
                        .and_then(|s| s.split('\0').next().map(str::to_string))
                        .filter(|s| !s.is_empty());

                    let (kind, enum_labels, unit) =
                        Self::detect_kind_labels_and_unit(&params_ext, &mut plugin, &info);

                    let current_value = params_ext
                        .get_value(&mut plugin, info.id)
                        .map(|v| v as f32)
                        .unwrap_or(default);

                    let value_to_text =
                        Self::format_value(&params_ext, &mut plugin, info.id, current_value as f64);

                    param_values.insert(id, current_value);

                    param_infos.push(UnifiedParamInfo {
                        key: ParamKey::Clap(id),
                        name,
                        min,
                        max,
                        default,
                        stepped,
                        enum_labels,
                        kind,
                        group,
                        is_hidden: flags.contains(ParamInfoFlags::IS_HIDDEN),
                        is_readonly: flags.contains(ParamInfoFlags::IS_READONLY),
                        is_automatable: flags.contains(ParamInfoFlags::IS_AUTOMATABLE),
                        is_bypass: flags.contains(ParamInfoFlags::IS_BYPASS),
                        unit,
                        value_to_text,
                    });
                }
            }

            (param_infos, param_values)
        }
    }

    impl PluginBackend for ClapHostBackend {
        fn kind(&self) -> BackendKind {
            BackendKind::Clap
        }

        fn init(&self, _cfg: &HostConfig) -> Result<()> {
            Ok(())
        }

        fn scan(&self) -> Result<Vec<UnifiedPluginInfo>> {
            let mut out = Vec::new();
            for dir in &self.cfg.plugin_scan_paths {
                if !dir.exists() {
                    continue;
                }
                if let Ok(rd) = std::fs::read_dir(dir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.is_dir() || p.extension().and_then(|e| e.to_str()) == Some("clap") {
                            Self::enumerate_bundle(&p, &mut out);
                        }
                    }
                }
            }
            Ok(out)
        }

        fn instantiate(&self, uri: &str) -> Result<Box<dyn UniInstance>> {
            let (lib, plugin_id) = Self::parse_uri(uri)?;
            let host_info = self.host_info.clone();
            let sample_rate = self.cfg.sample_rate;
            let max_block = self.cfg.max_block;

            // Load entry before the thread spawn (dlopen must happen on this thread
            // because it uses thread-local state for library search paths).
            let entry = unsafe {
                PluginEntry::load(&lib).map_err(|e| anyhow!("CLAP load entry failed: {e:?}"))?
            };

            let callback_requested = Arc::new(AtomicBool::new(false));
            let (cmd_tx, cmd_rx) = mpsc::channel::<MainThreadCommand>();
            let (result_tx, result_rx) = mpsc::channel();
            let instance_id = NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed);

            thread::Builder::new()
                .name(format!("clap-main-{}", &plugin_id))
                .spawn(move || {
                    let cb_flag = callback_requested.clone();
                    let gui_cmd_tx = cmd_tx.clone();
                    let timer_state = Arc::new(Mutex::new(TimerState {
                        timers: Vec::new(),
                        next_id: 0,
                    }));
                    let create_result = catch_unwind(AssertUnwindSafe(|| {
                        let desc_id = {
                            let factory = entry
                                .get_plugin_factory()
                                .ok_or_else(|| anyhow!("No plugin factory in entry: {}", lib))?;
                            let descriptor = factory
                                .plugin_descriptors()
                                .find(|d| {
                                    d.id()
                                        .map(|id| id.to_string_lossy() == plugin_id)
                                        .unwrap_or(false)
                                })
                                .ok_or_else(|| {
                                    anyhow!("Plugin id not found in entry: {}", plugin_id)
                                })?;
                            descriptor
                                .id()
                                .ok_or_else(|| anyhow!("Descriptor has no id"))?
                        };

                        let ts = timer_state.clone();
                        let mut instance = PluginInstance::<MyHost>::new(
                            |_| MyHostShared {
                                callback_requested: cb_flag,
                                gui_cmd_tx,
                            },
                            |_| MyHostMainThread { timer_state: ts },
                            &entry,
                            desc_id,
                            &host_info,
                        )
                        .map_err(|e| anyhow!("CLAP instantiate failed: {e:?}"))?;

                        let (params, param_values) =
                            ClapHostBackend::fetch_params_with_values(&mut instance);

                        let cfg = PluginAudioConfiguration {
                            sample_rate,
                            min_frames_count: 1,
                            max_frames_count: max_block as u32,
                        };
                        let activated = instance
                            .activate(|_, _| (), cfg)
                            .map_err(|e| anyhow!("CLAP activate failed: {e:?}"))?;
                        let processor = activated
                            .start_processing()
                            .map_err(|e| anyhow!("CLAP start_processing failed: {e:?}"))?;

                        let has_gui = instance
                            .plugin_handle()
                            .get_extension::<PluginGui>()
                            .is_some();

                        Ok::<_, anyhow::Error>((
                            instance,
                            entry,
                            params,
                            param_values,
                            processor,
                            has_gui,
                        ))
                    }));

                    match create_result {
                        Ok(Ok((instance, entry, params, param_values, processor, has_gui))) => {
                            register_main_thread(instance_id, cmd_tx.clone());
                            result_tx
                                .send(Ok((processor, params, param_values, has_gui)))
                                .ok();
                            clap_main_loop(
                                instance,
                                entry,
                                callback_requested,
                                cmd_rx,
                                instance_id,
                                timer_state,
                            );
                        }
                        Ok(Err(e)) => {
                            result_tx.send(Err(e)).ok();
                        }
                        Err(panic) => {
                            let msg = panic
                                .downcast_ref::<&str>()
                                .copied()
                                .or_else(|| panic.downcast_ref::<String>().map(|s| s.as_str()))
                                .unwrap_or("<unknown panic>");
                            result_tx
                                .send(Err(anyhow!("CLAP main thread panicked: {msg}")))
                                .ok();
                        }
                    }
                })
                .map_err(|e| anyhow!("Failed to spawn CLAP main thread: {e}"))?;

            let (processor, params, param_values, has_gui) = result_rx
                .recv()
                .map_err(|_| anyhow!("CLAP main thread failed to start"))??;

            Ok(Box::new(ClapAudioInstance {
                processor: Some(processor),
                params,
                param_values,
                main_thread_id: instance_id,
                has_gui,
                input_copies: vec![vec![0.0; max_block]; 2],
                note_ons: Vec::with_capacity(128),
                note_offs: Vec::with_capacity(128),
                pending_param_changes: Vec::new(),
            }))
        }
    }

    static NEXT_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);

    type MainThreadId = u64;

    type MainThreadRegistry = Mutex<HashMap<MainThreadId, mpsc::Sender<MainThreadCommand>>>;

    fn main_thread_registry() -> &'static MainThreadRegistry {
        static REGISTRY: OnceLock<MainThreadRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn register_main_thread(id: MainThreadId, tx: mpsc::Sender<MainThreadCommand>) {
        main_thread_registry().lock().unwrap().insert(id, tx);
    }

    fn lookup_main_thread(id: MainThreadId) -> Option<mpsc::Sender<MainThreadCommand>> {
        main_thread_registry().lock().unwrap().get(&id).cloned()
    }

    fn unregister_main_thread(id: MainThreadId) {
        main_thread_registry().lock().unwrap().remove(&id);
    }

    /// Audio-thread side: holds only the audio processor + params.
    /// The PluginInstance (main-thread side) lives on a dedicated CLAP main thread.
    pub struct ClapAudioInstance {
        processor: Option<StartedPluginAudioProcessor<MyHost>>,
        params: Vec<UnifiedParamInfo>,
        param_values: HashMap<u32, f32>,
        main_thread_id: MainThreadId,
        has_gui: bool,
        input_copies: Vec<Vec<f32>>,
        note_ons: Vec<NoteOnEvent>,
        note_offs: Vec<NoteOffEvent>,
        pending_param_changes: Vec<(u32, f64)>,
    }

    impl Drop for ClapAudioInstance {
        fn drop(&mut self) {
            let Some(processor) = self.processor.take() else {
                return;
            };

            if let Some(tx) = lookup_main_thread(self.main_thread_id) {
                let (result_tx, result_rx) = mpsc::channel();
                let _ = tx.send(MainThreadCommand::Shutdown {
                    processor,
                    result_tx,
                });
                let _ = result_rx.recv_timeout(Duration::from_secs(2));
            }

            unregister_main_thread(self.main_thread_id);
        }
    }

    impl UniInstance for ClapAudioInstance {
        fn process(
            &mut self,
            ctx: &ProcessCtx,
            audio_in: &[&[f32]],
            audio_out: &mut [&mut [f32]],
            events: &[MidiEvent],
        ) -> Result<()> {
            let frames = ctx.frames;

            self.note_ons.clear();
            self.note_offs.clear();

            for e in events {
                let time = (e.time_frames.clamp(0, frames as i64) as u32).min(frames as u32 - 1);
                let port = 0u16;
                let channel = (e.status & 0x0F) as u16;
                let key = e.data1 as u16;
                let note_id = key as u32;
                let velocity = (e.data2 as f32 / 127.0) as f64;
                let pckn = Pckn::new(port, channel, key, note_id);

                match e.status & 0xF0 {
                    0x90 if e.data2 > 0 => {
                        self.note_ons.push(NoteOnEvent::new(time, pckn, velocity));
                    }
                    0x80 | 0x90 => {
                        self.note_offs.push(NoteOffEvent::new(time, pckn, velocity));
                    }
                    _ => {}
                }
            }

            self.note_ons.sort_by_key(|e| e.time());
            self.note_offs.sort_by_key(|e| e.time());

            for (i, &input_channel) in audio_in.iter().enumerate() {
                if i < self.input_copies.len() {
                    let len = frames.min(input_channel.len());
                    self.input_copies[i][..len].copy_from_slice(&input_channel[..len]);
                }
            }

            let mut in_ports = AudioPorts::with_capacity(audio_in.len(), 1);
            let mut out_ports = AudioPorts::with_capacity(audio_out.len(), 1);

            let in_buffers = vec![AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only(
                    self.input_copies
                        .iter_mut()
                        .take(audio_in.len())
                        .map(|buf| InputChannel::variable(&mut buf[..frames])),
                ),
            }];

            let out_buffers = vec![AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    audio_out.iter_mut().map(|b| &mut b[..frames]),
                ),
            }];

            let in_audio = in_ports.with_input_buffers(in_buffers);
            let mut out_audio = out_ports.with_output_buffers(out_buffers);

            let mut out_buffer = EventBuffer::new();
            let mut out_events = OutputEvents::from_buffer(&mut out_buffer);

            let mut combined_buffer = EventBuffer::new();

            for (id, value) in self.pending_param_changes.drain(..) {
                combined_buffer.push(&ParamValueEvent::new(
                    0,
                    ClapId::new(id),
                    Pckn::match_all(),
                    value,
                    Cookie::empty(),
                ));
            }

            for event in &self.note_offs {
                combined_buffer.push(event);
            }

            for event in &self.note_ons {
                combined_buffer.push(event);
            }

            let combined_events = InputEvents::from_buffer(&combined_buffer);
            let processor = self
                .processor
                .as_mut()
                .ok_or_else(|| anyhow!("processor dropped"))?;
            processor
                .process(
                    &in_audio,
                    &mut out_audio,
                    &combined_events,
                    &mut out_events,
                    None,
                    None,
                )
                .map_err(|e| anyhow!("CLAP process failed: {e:?}"))?;

            Ok(())
        }

        fn set_param(&mut self, key: &ParamKey, value: f32) {
            if let ParamKey::Clap(id) = key {
                self.param_values.insert(*id, value);
                self.pending_param_changes.push((*id, value as f64));
            }
        }

        fn get_param(&self, key: &ParamKey) -> Option<f32> {
            match key {
                ParamKey::Clap(id) => self.param_values.get(id).copied(),
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

        fn open_editor(&mut self) -> Result<()> {
            let tx = lookup_main_thread(self.main_thread_id)
                .ok_or_else(|| anyhow!("CLAP main thread not found"))?;
            let (result_tx, result_rx) = mpsc::channel();
            tx.send(MainThreadCommand::OpenEditor(result_tx))
                .map_err(|_| anyhow!("CLAP main thread disconnected"))?;
            thread::spawn(move || match result_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => log::error!("Failed to open CLAP editor: {e}"),
                Err(_) => log::error!("CLAP editor open response channel was dropped"),
            });
            Ok(())
        }

        fn has_editor(&self) -> bool {
            self.has_gui
        }
    }

    enum MainThreadCommand {
        OpenEditor(mpsc::Sender<Result<()>>),
        CloseEditor,
        RequestResize(GuiSize),
        GuiClosed,
        Shutdown {
            processor: StartedPluginAudioProcessor<MyHost>,
            result_tx: mpsc::Sender<()>,
        },
    }

    /// Main loop for the CLAP main thread. Owns PluginInstance and handles
    /// GUI lifecycle, callback pump, and X11 event polling.
    fn clap_main_loop(
        mut instance: PluginInstance<MyHost>,
        _entry: PluginEntry,
        _callback_requested: Arc<AtomicBool>,
        cmd_rx: mpsc::Receiver<MainThreadCommand>,
        instance_id: MainThreadId,
        timer_state: Arc<Mutex<TimerState>>,
    ) {
        let xlib = xlib::Xlib::open().ok();
        let mut editor: Option<EditorState> = None;

        loop {
            // Fire any expired timers
            let expired: Vec<(TimerId, u32)> = {
                let mut ts = timer_state.lock().unwrap();
                let now = std::time::Instant::now();
                let mut expired = Vec::new();
                for (id, period_ms, next_fire) in &mut ts.timers {
                    if now >= *next_fire {
                        expired.push((*id, *period_ms));
                        *next_fire = now + std::time::Duration::from_millis(*period_ms as u64);
                    }
                }
                expired
            };
            let plugin_timer: Option<PluginTimer> =
                { instance.plugin_handle().get_extension::<PluginTimer>() };
            if let Some(timer) = plugin_timer {
                for (timer_id, _period_ms) in &expired {
                    timer.on_timer(&mut instance.plugin_handle(), *timer_id);
                }
            }

            // Pump callbacks — must call on_main_thread regularly per CLAP spec,
            // even when the plugin has not explicitly requested it.
            instance.call_on_main_thread_callback();

            // Process commands (non-blocking, wake immediately on arrival)
            match cmd_rx.recv_timeout(Duration::from_millis(16)) {
                Ok(MainThreadCommand::OpenEditor(result_tx)) => {
                    let result = open_editor_on_main_thread(&mut instance, &xlib, &mut editor);
                    if result.is_err() {
                        if let Some(state) = editor.take() {
                            close_editor_state(&mut instance, &xlib, state);
                        }
                    }
                    let _ = result_tx.send(result);
                }
                Ok(MainThreadCommand::CloseEditor) => {
                    if let Some(state) = editor.take() {
                        close_editor_state(&mut instance, &xlib, state);
                    }
                }
                Ok(MainThreadCommand::RequestResize(new_size)) => {
                    log::info!(
                        "Plugin requested resize to {}x{}",
                        new_size.width,
                        new_size.height
                    );
                }
                Ok(MainThreadCommand::GuiClosed) => {
                    log::info!("Plugin GUI closed by plugin");
                    if let Some(state) = editor.take() {
                        close_editor_state(&mut instance, &xlib, state);
                    }
                }
                Ok(MainThreadCommand::Shutdown {
                    processor,
                    result_tx,
                }) => {
                    if let Some(state) = editor.take() {
                        close_editor_state(&mut instance, &xlib, state);
                    }
                    let stopped = processor.stop_processing();
                    instance.deactivate(stopped);
                    let _ = result_tx.send(());
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(state) = editor.take() {
                        close_editor_state(&mut instance, &xlib, state);
                    }
                    break;
                }
            }

            // Poll X11 events for embedded editor
            #[cfg(unix)]
            if let Some(ref mut state) = editor {
                if pump_x11_events(&xlib, state) {
                    if let Some(state) = editor.take() {
                        close_editor_state(&mut instance, &xlib, state);
                    }
                }
            }
        }

        unregister_main_thread(instance_id);
    }

    enum EditorState {
        Floating,
        #[cfg(unix)]
        Embedded {
            display: *mut xlib::Display,
            parent_win: xlib::Window,
            child_win: xlib::Window,
            wm_delete_window: u64,
            wm_protocols: u64,
            size: (u32, u32),
            expose_tick: u32,
        },
    }

    unsafe impl Send for EditorState {}
    unsafe impl Sync for EditorState {}

    #[cfg(unix)]
    fn send_expose_event(
        xlib: &xlib::Xlib,
        display: *mut xlib::Display,
        win: xlib::Window,
        w: i32,
        h: i32,
    ) {
        unsafe {
            let mut ev: xlib::XEvent = mem::zeroed();
            ev.type_ = xlib::Expose as i32;
            let expose = &mut ev.expose;
            expose.type_ = xlib::Expose as i32;
            expose.window = win;
            expose.x = 0;
            expose.y = 0;
            expose.width = w;
            expose.height = h;
            expose.count = 0;
            (xlib.XSendEvent)(display, win, 0, 0, &mut ev);
            (xlib.XFlush)(display);
        }
    }

    #[cfg(unix)]
    unsafe fn cleanup_x11_window(
        xlib: &xlib::Xlib,
        display: *mut xlib::Display,
        win: xlib::Window,
    ) {
        unsafe { (xlib.XDestroyWindow)(display, win) };
        unsafe { (xlib.XCloseDisplay)(display) };
    }

    /// Tries floating first, falls back to embedded.
    fn open_editor_on_main_thread(
        instance: &mut PluginInstance<MyHost>,
        _xlib: &Option<xlib::Xlib>,
        editor: &mut Option<EditorState>,
    ) -> Result<()> {
        if editor.is_some() {
            return Ok(());
        }

        let gui_ext = instance
            .plugin_handle()
            .get_extension::<PluginGui>()
            .ok_or_else(|| anyhow!("Plugin has no CLAP GUI extension"))?;

        // Try X11 floating first
        let floating_cfg = GuiConfiguration {
            api_type: GuiApiType::X11,
            is_floating: true,
        };
        if gui_ext.is_api_supported(&mut instance.plugin_handle(), floating_cfg) {
            gui_ext
                .create(&mut instance.plugin_handle(), floating_cfg)
                .map_err(|e| anyhow!("Plugin GUI create (floating) failed: {e:?}"))?;

            if let Err(e) = gui_ext.show(&mut instance.plugin_handle()) {
                gui_ext.destroy(&mut instance.plugin_handle());
                return Err(anyhow!("Plugin GUI show (floating) failed: {e:?}"));
            }

            *editor = Some(EditorState::Floating);
            log::info!("Plugin GUI opened in floating mode");
            return Ok(());
        }

        // Try Wayland floating
        let wayland_cfg = GuiConfiguration {
            api_type: GuiApiType::WAYLAND,
            is_floating: true,
        };
        if gui_ext.is_api_supported(&mut instance.plugin_handle(), wayland_cfg) {
            gui_ext
                .create(&mut instance.plugin_handle(), wayland_cfg)
                .map_err(|e| anyhow!("Plugin GUI create (Wayland) failed: {e:?}"))?;

            if let Err(e) = gui_ext.show(&mut instance.plugin_handle()) {
                gui_ext.destroy(&mut instance.plugin_handle());
                return Err(anyhow!("Plugin GUI show (Wayland) failed: {e:?}"));
            }

            *editor = Some(EditorState::Floating);
            log::info!("Plugin GUI opened in Wayland floating mode");
            return Ok(());
        }

        #[cfg(unix)]
        {
            // Try X11 embedded
            let embedded_cfg = GuiConfiguration {
                api_type: GuiApiType::X11,
                is_floating: false,
            };
            if !gui_ext.is_api_supported(&mut instance.plugin_handle(), embedded_cfg) {
                return Err(anyhow!(
                    "Plugin does not support X11 floating, Wayland floating, or X11 embedded"
                ));
            }

            let xlib = _xlib.as_ref().ok_or_else(|| anyhow!("Xlib not available"))?;

            // Open X11 display
            let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
            if display.is_null() {
                return Err(anyhow!("Cannot open X11 display"));
            }

            let screen = unsafe { (xlib.XDefaultScreen)(display) };
            let root = unsafe { (xlib.XRootWindow)(display, screen) };
            let white = unsafe { (xlib.XWhitePixel)(display, screen) };

            let parent_win = unsafe {
                (xlib.XCreateSimpleWindow)(display, root, 100, 100, 800, 600, 0, 0, white)
            };

            unsafe {
                (xlib.XSelectInput)(
                    display,
                    parent_win,
                    (xlib::StructureNotifyMask
                        | xlib::ExposureMask
                        | xlib::SubstructureNotifyMask
                        | xlib::PropertyChangeMask) as i64,
                );
            }

            let wm_delete_window =
                unsafe { (xlib.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0) };
            let wm_protocols =
                unsafe { (xlib.XInternAtom)(display, c"WM_PROTOCOLS".as_ptr(), 0) };
            unsafe {
                (xlib.XSetWMProtocols)(
                    display,
                    parent_win,
                    &wm_delete_window as *const _ as *mut _,
                    1,
                );
            }

            unsafe {
                (xlib.XMapWindow)(display, parent_win);
                (xlib.XFlush)(display);
            }

            // Now, create (embedded)
            gui_ext
                .create(&mut instance.plugin_handle(), embedded_cfg)
                .map_err(|e| {
                    unsafe {
                        cleanup_x11_window(xlib, display, parent_win);
                    }
                    anyhow!("Plugin GUI create (embedded) failed: {e:?}")
                })?;

            // Preferred size
            let preferred_size = gui_ext.get_size(&mut instance.plugin_handle());
            log::info!("Plugin preferred size: {:?}", preferred_size);
            if let Some(size) = preferred_size {
                unsafe {
                    (xlib.XResizeWindow)(display, parent_win, size.width, size.height);
                    (xlib.XSync)(display, 0);
                }
            }

            // set_parent
            let clap_window = clack_extensions::gui::Window::from_x11_handle(parent_win);
            let sp_result =
                unsafe { gui_ext.set_parent(&mut instance.plugin_handle(), clap_window) };
            log::info!("set_parent result: {:?}", sp_result);
            if let Err(e) = sp_result {
                let _ = gui_ext.destroy(&mut instance.plugin_handle());
                unsafe {
                    cleanup_x11_window(xlib, display, parent_win);
                }
                return Err(anyhow!("set_parent failed: {e:?}"));
            }

            // set_size before show
            if let Some(size) = gui_ext.get_size(&mut instance.plugin_handle()) {
                let ss_result = gui_ext.set_size(&mut instance.plugin_handle(), size);
                log::info!(
                    "set_size(pre-show) size={}x{} result={:?}",
                    size.width,
                    size.height,
                    ss_result
                );
            }

            // show
            let show_result = gui_ext.show(&mut instance.plugin_handle());
            log::info!("show result: {:?}", show_result);
            if let Err(e) = show_result {
                let _ = gui_ext.destroy(&mut instance.plugin_handle());
                unsafe {
                    cleanup_x11_window(xlib, display, parent_win);
                }
                return Err(anyhow!("show failed: {e:?}"));
            }

            // set_size after show, as JUCE could defer resized() on invisible components (surge)
            if let Some(size) = gui_ext.get_size(&mut instance.plugin_handle()) {
                let ss_result = gui_ext.set_size(&mut instance.plugin_handle(), size);
                log::info!(
                    "set_size(post-show) size={}x{} result={:?}",
                    size.width,
                    size.height,
                    ss_result
                );
            }

            unsafe {
                (xlib.XMapSubwindows)(display, parent_win);
                (xlib.XSync)(display, 0);
            }

            // get ID for Expose-based paint triggering
            let child_win = unsafe {
                let mut root_ret: xlib::Window = 0;
                let mut parent_ret: xlib::Window = 0;
                let mut children: *mut xlib::Window = ptr::null_mut();
                let mut nchildren: std::os::raw::c_uint = 0;
                let ok = (xlib.XQueryTree)(
                    display,
                    parent_win,
                    &mut root_ret,
                    &mut parent_ret,
                    &mut children,
                    &mut nchildren,
                );
                let found = if ok != 0 && !children.is_null() && nchildren > 0 {
                    *children.offset(0)
                } else {
                    0
                };
                if !children.is_null() {
                    (xlib.XFree)(children as *mut _);
                }
                found
            };

            let size = preferred_size
                .map(|s| (s.width, s.height))
                .unwrap_or((800, 600));

            // Send initial Expose to child to kick-start painting
            if child_win != 0 {
                let (w, h) = size;
                send_expose_event(xlib, display, child_win, w as i32, h as i32);
                log::info!("Sent initial Expose to child 0x{:x}", child_win);
            }

            *editor = Some(EditorState::Embedded {
                display,
                parent_win,
                child_win,
                wm_delete_window,
                wm_protocols,
                size,
                expose_tick: 0,
            });
            log::info!("Plugin GUI opened in embedded mode");
            return Ok(());
        }

        #[cfg(not(unix))]
        return Err(anyhow!(
            "Plugin does not support X11 floating, Wayland floating, or X11 embedded"
        ));
    }

    fn close_editor_state(
        instance: &mut PluginInstance<MyHost>,
        _xlib: &Option<xlib::Xlib>,
        state: EditorState,
    ) {
        if let Some(gui_ext) = instance.plugin_handle().get_extension::<PluginGui>() {
            let _ = gui_ext.hide(&mut instance.plugin_handle());
            gui_ext.destroy(&mut instance.plugin_handle());
        }

        match state {
            EditorState::Floating => {}

            #[cfg(unix)]
            EditorState::Embedded {
                display,
                parent_win,
                ..
            } => {
                if let Some(xlib) = _xlib {
                    unsafe {
                        (xlib.XDestroyWindow)(display, parent_win);
                        (xlib.XCloseDisplay)(display);
                    }
                }
            }
        }
    }

    /// Returns `true` if the editor was closed (user clicked the close button).
    #[cfg(unix)]
    fn pump_x11_events(xlib: &Option<xlib::Xlib>, state: &mut EditorState) -> bool {
        let xlib = match xlib {
            Some(x) => x,
            None => return false,
        };

        let (display, child_win, wm_delete_window, wm_protocols, size, expose_tick) = match state {
            EditorState::Embedded {
                display,
                child_win,
                wm_delete_window,
                wm_protocols,
                size,
                expose_tick,
                ..
            } => (
                display,
                child_win,
                wm_delete_window,
                wm_protocols,
                size,
                expose_tick,
            ),
            _ => return false,
        };

        while unsafe { (xlib.XPending)(*display) } > 0 {
            let mut event: xlib::XEvent = unsafe { mem::zeroed() };
            unsafe { (xlib.XNextEvent)(*display, &mut event) };

            match unsafe { event.type_ } {
                xlib::ClientMessage => {
                    let msg = unsafe { event.client_message };
                    if msg.message_type == *wm_protocols
                        && msg.data.as_longs()[0] as u64 == *wm_delete_window
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }

        // Periodically send Expose to child as a paint kick
        if *child_win != 0 {
            *expose_tick += 1;
            // Send Expose every ~8 calls (~128ms at 16ms loop)
            if *expose_tick % 8 == 0 {
                let (w, h) = *size;
                send_expose_event(xlib, *display, *child_win, w as i32, h as i32);
            }
        }
        false
    }

    pub use ClapHostBackend as Backend;
}

#[cfg(feature = "clap-host")]
pub use clap_impl::Backend;
