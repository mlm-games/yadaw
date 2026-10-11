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
                    let (kind, stepped) = if p.step_count == 1 {
                        (ParamKind::Bool, true)
                    } else if p.step_count > 1 {
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

            #[cfg(unix)]
            let editor_host =
                match EditorHost::spawn(Box::new(Vst3EditorBackend::new(plugin.clone()))) {
                    Ok(host) => Some(host),
                    Err(e) => {
                        log::warn!("VST3 plugin loaded without its control thread: {e}");
                        None
                    }
                };

            Ok(Box::new(Vst3PluginInstance {
                plugin,
                params: param_infos,
                param_values,
                sample_rate: self.cfg.sample_rate,
                #[cfg(unix)]
                editor_host,
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

    const DEFAULT_TEMPO: f64 = 120.0;

    #[cfg(unix)]
    impl EditorBackend for Vst3EditorBackend {
        fn has_editor(&self) -> bool {
            self.plugin
                .lock()
                .map(|p| p.is_editor_open() || p.has_editor())
                .unwrap_or(false)
        }

        fn try_open_floating(&mut self) -> Result<bool> {
            Ok(false)
        }

        fn on_idle(&mut self) {
            let Ok(mut plugin) = self.plugin.try_lock() else {
                return;
            };

            if plugin.is_editor_open() {
                plugin.service_run_loop();
            }

            for notification in plugin.take_host_notifications() {
                match notification {
                    vst3_host::HostNotification::BusActivationRequested {
                        bus_index,
                        active,
                        ..
                    } => log::debug!(
                        "VST3 plugin asked for bus {bus_index} to be {}; yadaw has no auxiliary bus routing to grant it",
                        if active { "activated" } else { "deactivated" }
                    ),
                    vst3_host::HostNotification::OpenEditorRequested { .. } => log::debug!(
                        "VST3 plugin asked the host to open its editor; yadaw opens it from the plugin panel"
                    ),
                    _ => {}
                }
            }

            let flags = plugin.take_restart_flags();
            if (flags.latency_changed() || flags.io_changed())
                && let Err(e) = plugin.service_host_requests()
            {
                log::warn!("VST3 plugin could not be reconfigured: {e}");
            }
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
            let mut plugin = self
                .plugin
                .lock()
                .map_err(|e| anyhow!("VST3 plugin lock poisoned: {}", e))?;
            if !plugin.is_editor_open() {
                return Ok(());
            }
            plugin
                .close_editor()
                .map_err(|e| anyhow!("Failed to close VST3 editor: {e}"))
        }

        fn preferred_size(&self) -> Option<(u32, u32)> {
            let plugin = self.plugin.lock().ok()?;
            match plugin.attached_editor_size() {
                Ok(Some(size)) => Some((size.0 as u32, size.1 as u32)),
                _ => plugin
                    .get_editor_size()
                    .ok()
                    .map(|(w, h)| (w as u32, h as u32)),
            }
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
            if ctx.frames == 0 {
                return Ok(());
            }
            let mut plugin = self
                .plugin
                .lock()
                .map_err(|e| anyhow!("VST3 plugin lock poisoned: {}", e))?;

            let frames = ctx.frames;
            let block_size = plugin.block_size().max(1);

            for e in events {
                if let Some(midi) = convert_midi_event(e) {
                    let _ = plugin.send_midi_event(midi);
                }
            }

            // A tempo the plugin cannot be told about would leave its transport
            // frozen, so an unusable project tempo falls back to a plain one.
            let tempo = match ctx.bpm {
                bpm if bpm.is_finite() && bpm > 0.0 => f64::from(bpm),
                _ => DEFAULT_TEMPO,
            };
            let _ = plugin.set_tempo(tempo);
            let _ = plugin.set_playing(ctx.playing);
            let beats_per_second = tempo / 60.0;
            let start_samples = ctx.time_samples;
            let start_beats = ctx.beats;

            let mut offset = 0;
            while offset < frames {
                let actual_frames = (frames - offset).min(block_size);

                // Position of this sub-block's first frame, in the host's samples
                // and quarter notes. A project position that is not a number is
                // left alone rather than half-substituted.
                let position = vst3_host::TransportPosition {
                    samples: start_samples as i64 + offset as i64,
                    quarter_notes: start_beats
                        + offset as f64 / self.sample_rate * beats_per_second,
                };
                if start_samples.is_finite() && start_beats.is_finite() {
                    let _ = plugin.set_transport_position(position);
                }

                let in_ch = audio_in.len();
                let out_ch = audio_out.len();
                let mut buffers =
                    vst3_host::AudioBuffers::new(in_ch, out_ch, actual_frames, self.sample_rate);

                for (ch, input) in audio_in.iter().enumerate() {
                    if let Some(buf) = buffers.inputs.get_mut(ch) {
                        let avail = input.len().saturating_sub(offset);
                        let len = actual_frames.min(avail).min(buf.len());
                        if len > 0 {
                            buf[..len].copy_from_slice(&input[offset..offset + len]);
                        }
                    }
                }

                plugin
                    .process_audio(&mut buffers)
                    .map_err(|e| anyhow!("VST3 process_audio failed: {e}"))?;

                for (ch, output) in audio_out.iter_mut().enumerate() {
                    if offset >= output.len() {
                        continue;
                    }
                    if let Some(buf) = buffers.outputs.get(ch) {
                        let avail = output.len() - offset;
                        let len = actual_frames.min(avail).min(buf.len());
                        output[offset..offset + len].copy_from_slice(&buf[..len]);
                        if len < actual_frames.min(avail) {
                            output[offset + len..offset + actual_frames.min(avail)].fill(0.0);
                        }
                    } else {
                        let avail = (output.len() - offset).min(actual_frames);
                        output[offset..offset + avail].fill(0.0);
                    }
                }

                offset += actual_frames;
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
            #[cfg(unix)]
            {
                let host = self
                    .editor_host
                    .as_ref()
                    .ok_or_else(|| anyhow!("VST3 editor is unavailable on this system"))?;
                host.open_editor()
                    .map_err(|e| anyhow!("Failed to open VST3 editor: {e}"))
            }

            #[cfg(not(unix))]
            Err(anyhow!("VST3 editor not supported on this platform"))
        }

        fn has_editor(&self) -> bool {
            self.plugin
                .lock()
                .map(|p| p.is_editor_open() || p.has_editor())
                .unwrap_or(false)
        }
    }

    /// A control change, pitch bend or channel aftertouch is not a VST3 event: the
    /// host library resolves it through the plugin's `IMidiMapping` table, queues
    /// the mapped parameter and drops what the plugin does not map. Keeping a copy
    /// of that table here would only be a staler version of the same decision.
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
