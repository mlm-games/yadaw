#[cfg(feature = "vst3-host")]
mod vst3_impl {
    use anyhow::{Result, anyhow};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use vst3_host;
    use yadaw_plugin_api::{
        BackendKind, HostConfig, MidiEvent, ParamKey, ParamKind, PluginBackend,
        PluginInstance as UniInstance, ProcessCtx, UnifiedParamInfo, UnifiedPluginInfo,
    };

    #[cfg(unix)]
    use crate::editor_host::{EditorBackend, EditorHost};

    pub struct Vst3HostBackend {
        cfg: HostConfig,
    }

    impl Vst3HostBackend {
        pub fn new(cfg: HostConfig) -> Self {
            Self { cfg }
        }
    }

    impl PluginBackend for Vst3HostBackend {
        fn kind(&self) -> BackendKind {
            BackendKind::Vst3
        }

        fn init(&self, _cfg: &HostConfig) -> Result<()> {
            Ok(())
        }

        fn scan(&self) -> Result<Vec<UnifiedPluginInfo>> {
            let Ok(mut host) = vst3_host::Vst3Host::builder()
                .sample_rate(self.cfg.sample_rate)
                .block_size(self.cfg.max_block)
                .scan_default_paths()
                .build()
            else {
                return Ok(Vec::new());
            };

            for path in &self.cfg.plugin_scan_paths {
                if path.exists() {
                    let _ = host.add_scan_path(path);
                }
            }

            let plugins = host.discover_plugins().unwrap_or_default();

            Ok(plugins
                .into_iter()
                .map(|info| {
                    let is_instr = info.category == "Instrument"
                        || info.category == "Synth"
                        || info.category.contains("Instrument");
                    UnifiedPluginInfo {
                        backend: BackendKind::Vst3,
                        uri: info.path.to_string_lossy().to_string(),
                        name: info.name,
                        is_instrument: is_instr,
                        audio_inputs: info.audio_inputs as usize,
                        audio_outputs: info.audio_outputs as usize,
                        has_midi: info.has_midi_input,
                    }
                })
                .collect())
        }

        fn instantiate(&self, uri: &str) -> Result<Box<dyn UniInstance>> {
            let path = Path::new(uri);
            if !path.exists() {
                return Err(anyhow!("VST3 plugin not found: {}", uri));
            }

            let Ok(mut host) = vst3_host::Vst3Host::builder()
                .sample_rate(self.cfg.sample_rate)
                .block_size(self.cfg.max_block)
                .input_channels(2)
                .output_channels(2)
                .build()
            else {
                return Err(anyhow!("Failed to create VST3 host"));
            };

            let mut plugin = host.load_plugin(path)?;

            plugin.start_processing()?;

            let params = plugin.get_parameters()?;

            let param_infos: Vec<UnifiedParamInfo> = params
                .iter()
                .map(|p| {
                    let (kind, stepped) = if p.step_count == 2 {
                        (ParamKind::Bool, true)
                    } else if p.step_count > 2 {
                        (ParamKind::Int, true)
                    } else {
                        (ParamKind::Float, false)
                    };

                    let unit = if p.unit.is_empty() {
                        None
                    } else {
                        Some(p.unit.clone())
                    };

                    let value_to_text = plugin.format_parameter(p.id, p.default).ok();

                    UnifiedParamInfo {
                        key: ParamKey::Vst3(p.id),
                        name: p.name.clone(),
                        min: p.min as f32,
                        max: p.max as f32,
                        default: p.default as f32,
                        stepped,
                        enum_labels: None,
                        kind,
                        group: None,
                        is_hidden: false,
                        is_readonly: p.is_read_only,
                        is_automatable: p.can_automate,
                        is_bypass: p.is_bypass,
                        unit,
                        value_to_text,
                    }
                })
                .collect();

            let param_values: HashMap<u32, f32> =
                params.iter().map(|p| (p.id, p.value as f32)).collect();

            let plugin = Arc::new(Mutex::new(plugin));

            Ok(Box::new(Vst3PluginInstance {
                plugin,
                params: param_infos,
                param_values,
                sample_rate: self.cfg.sample_rate,
                #[cfg(unix)]
                editor_host: None,
            }))
        }
    }

    struct Vst3PluginInstance {
        plugin: Arc<Mutex<vst3_host::Plugin>>,
        params: Vec<UnifiedParamInfo>,
        param_values: HashMap<u32, f32>,
        sample_rate: f64,
        #[cfg(unix)]
        editor_host: Option<EditorHost>,
    }

    impl Drop for Vst3PluginInstance {
        fn drop(&mut self) {
            #[cfg(unix)]
            if let Some(mut host) = self.editor_host.take() {
                host.shutdown();
            }
            if let Ok(mut plugin) = self.plugin.lock() {
                let _ = plugin.stop_processing();
            }
        }
    }

    /// Backend used by [`EditorHost`] to manage the VST3 editor lifecycle.
    #[cfg(unix)]
    struct Vst3EditorBackend {
        plugin: Arc<Mutex<vst3_host::Plugin>>,
    }

    #[cfg(unix)]
    impl Vst3EditorBackend {
        fn new(plugin: Arc<Mutex<vst3_host::Plugin>>) -> Self {
            Self { plugin }
        }
    }

    #[cfg(unix)]
    impl EditorBackend for Vst3EditorBackend {
        fn has_editor(&self) -> bool {
            self.plugin.lock().map(|p| p.has_editor()).unwrap_or(false)
        }

        fn try_open_floating(&mut self) -> Result<bool> {
            Ok(false)
        }

        fn open_embedded(&mut self, parent_window: u32) -> Result<()> {
            // HACK: from_x11 does the same cast internally, but is gated behind
            // target_os = "linux" in vst3-host
            let handle = unsafe {
                vst3_host::WindowHandle::from_raw(parent_window as usize as *mut std::ffi::c_void)
            };
            self.plugin
                .lock()
                .map_err(|e| anyhow!("VST3 plugin lock poisoned: {}", e))?
                .open_editor(handle)
                .map_err(|e| anyhow!("Failed to open VST3 editor: {e}"))
        }

        fn close(&mut self) -> Result<()> {
            self.plugin
                .lock()
                .map_err(|e| anyhow!("VST3 plugin lock poisoned: {}", e))?
                .close_editor()
                .map_err(|e| anyhow!("Failed to close VST3 editor: {e}"))
        }

        fn preferred_size(&self) -> Option<(u32, u32)> {
            self.plugin
                .lock()
                .ok()
                .and_then(|p| p.get_editor_size().ok())
                .map(|(w, h)| (w as u32, h as u32))
        }
    }

    impl UniInstance for Vst3PluginInstance {
        fn process(
            &mut self,
            ctx: &ProcessCtx,
            audio_in: &[&[f32]],
            audio_out: &mut [&mut [f32]],
            events: &[MidiEvent],
        ) -> Result<()> {
            let mut plugin = self
                .plugin
                .lock()
                .map_err(|e| anyhow!("VST3 plugin lock poisoned: {}", e))?;

            let frames = ctx.frames;
            let block_size = plugin.block_size();
            let actual_frames = frames.min(block_size).max(1);

            let in_ch = audio_in.len();
            let out_ch = audio_out.len();
            let mut buffers =
                vst3_host::AudioBuffers::new(in_ch, out_ch, actual_frames, self.sample_rate);

            for (ch, input) in audio_in.iter().enumerate() {
                if let Some(buf) = buffers.inputs.get_mut(ch) {
                    let len = actual_frames.min(input.len());
                    buf[..len].copy_from_slice(&input[..len]);
                }
            }

            for e in events {
                if let Some(midi) = convert_midi_event(e) {
                    let _ = plugin.send_midi_event(midi);
                }
            }

            plugin
                .process_audio(&mut buffers)
                .map_err(|e| anyhow!("VST3 process_audio failed: {e}"))?;

            for (ch, output) in audio_out.iter_mut().enumerate() {
                if let Some(buf) = buffers.outputs.get(ch) {
                    let len = actual_frames.min(output.len()).min(buf.len());
                    output[..len].copy_from_slice(&buf[..len]);
                    if len < actual_frames {
                        output[len..actual_frames].fill(0.0);
                    }
                } else {
                    output[..actual_frames].fill(0.0);
                }
            }

            for (id, value) in plugin.get_parameter_changes() {
                self.param_values.insert(id, value as f32);
            }

            Ok(())
        }

        fn set_param(&mut self, key: &ParamKey, value: f32) {
            if let ParamKey::Vst3(id) = key {
                self.param_values.insert(*id, value);
                if let Ok(mut plugin) = self.plugin.lock() {
                    let clamped = value.clamp(0.0, 1.0);
                    let _ = plugin.set_parameter(*id, clamped as f64);
                }
            }
        }

        fn get_param(&self, key: &ParamKey) -> Option<f32> {
            match key {
                ParamKey::Vst3(id) => self.param_values.get(id).copied(),
                _ => None,
            }
        }

        fn params(&self) -> &[UnifiedParamInfo] {
            &self.params
        }

        fn save_state(&mut self) -> Option<Vec<u8>> {
            self.plugin.lock().ok().and_then(|p| p.save_state().ok())
        }

        fn load_state(&mut self, data: &[u8]) -> bool {
            self.plugin
                .lock()
                .ok()
                .and_then(|mut p| p.load_state(data).ok())
                .is_some()
        }

        fn open_editor(&mut self) -> Result<()> {
            let has_editor = self
                .plugin
                .lock()
                .map_err(|e| anyhow!("VST3 plugin lock poisoned: {}", e))?
                .has_editor();
            if !has_editor {
                return Err(anyhow!("VST3 plugin has no editor"));
            }

            #[cfg(unix)]
            {
                if self.editor_host.is_none() {
                    let backend = Vst3EditorBackend::new(self.plugin.clone());
                    let host = EditorHost::spawn(Box::new(backend))
                        .map_err(|e| anyhow!("Failed to spawn editor thread: {e}"))?;
                    self.editor_host = Some(host);
                }
                self.editor_host
                    .as_ref()
                    .unwrap()
                    .open_editor()
                    .map_err(|e| anyhow!("Failed to open VST3 editor: {e}"))
            }

            #[cfg(not(unix))]
            Err(anyhow!("VST3 editor not supported on this platform"))
        }

        fn has_editor(&self) -> bool {
            self.plugin.lock().map(|p| p.has_editor()).unwrap_or(false)
        }
    }

    fn convert_midi_event(e: &MidiEvent) -> Option<vst3_host::MidiEvent> {
        let channel = vst3_host::MidiChannel::from_index(e.status & 0x0F)?;
        match e.status & 0xF0 {
            0x90 if e.data2 > 0 => Some(vst3_host::MidiEvent::NoteOn {
                channel,
                note: e.data1,
                velocity: e.data2,
            }),
            0x80 | 0x90 => Some(vst3_host::MidiEvent::NoteOff {
                channel,
                note: e.data1,
                velocity: e.data2,
            }),
            0xB0 => Some(vst3_host::MidiEvent::ControlChange {
                channel,
                controller: e.data1,
                value: e.data2,
            }),
            0xC0 => Some(vst3_host::MidiEvent::ProgramChange {
                channel,
                program: e.data1,
            }),
            0xE0 => {
                let lsb = e.data1 as u16;
                let msb = e.data2 as u16;
                Some(vst3_host::MidiEvent::PitchBend {
                    channel,
                    value: (msb << 7) | lsb,
                })
            }
            0xD0 => Some(vst3_host::MidiEvent::ChannelAftertouch {
                channel,
                pressure: e.data1,
            }),
            0xA0 => Some(vst3_host::MidiEvent::PolyAftertouch {
                channel,
                note: e.data1,
                pressure: e.data2,
            }),
            _ => None,
        }
    }

    pub use Vst3HostBackend as Backend;
}

#[cfg(feature = "vst3-host")]
pub use vst3_impl::Backend;
