#[cfg(feature = "clap-host")]
mod clap_impl {
    use anyhow::{Result, anyhow};
    
    use std::path::{Path, PathBuf};

    // Core clack-host types
    use clack_host::prelude::*;
    use clack_host::process::StartedPluginAudioProcessor;

    // Event I/O types
    use clack_host::events::Pckn;
    use clack_host::events::event_types::{NoteOffEvent, NoteOnEvent, ParamValueEvent};
    use clack_host::events::io::{EventBuffer, InputEvents, OutputEvents};
    use clack_host::utils::{ClapId, Cookie};

    use crate::model::plugin_api::{
        BackendKind, HostConfig, MidiEvent, ParamKey, PluginBackend, PluginInstance as UniInstance,
        ProcessCtx, UnifiedParamInfo, UnifiedPluginInfo,
    };

    struct MyHostShared;
    impl<'a> SharedHandler<'a> for MyHostShared {
        fn request_restart(&self) {}
        fn request_process(&self) {}
        fn request_callback(&self) {}
    }

    struct MyHost;
    impl HostHandlers for MyHost {
        type Shared<'a> = MyHostShared;
        type MainThread<'a> = ();
        type AudioProcessor<'a> = ();
    }

    // ---------------- Backend ----------------
    pub struct ClapHostBackend {
        cfg: HostConfig,
        host_info: HostInfo,
    }

    impl ClapHostBackend {
        pub fn new(cfg: HostConfig) -> Result<Self> {
            let host_info = HostInfo::new("YADAW", "YADAW", "https://example.invalid", "0.1.0")?;
            Ok(Self { cfg, host_info })
        }

        fn scan_dirs() -> Vec<PathBuf> {
            let mut v = Vec::new();
            if let Ok(home) = std::env::var("HOME") {
                v.push(PathBuf::from(format!("{home}/.clap")));
            }
            v.push(PathBuf::from("/usr/lib/clap"));
            v.push(PathBuf::from("/usr/local/lib/clap"));
            v
        }

        fn enumerate_bundle(path: &Path, out: &mut Vec<UnifiedPluginInfo>) {
            let mut libs = Vec::new();
            if path.is_file() {
                libs.push(path.to_path_buf());
            } else if path.is_dir()
                && let Ok(rd) = std::fs::read_dir(path) {
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
                    if let Ok(bundle) = PluginBundle::load(lib.to_string_lossy().as_ref())
                        && let Some(factory) = bundle.get_plugin_factory() {
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

        fn parse_uri(uri: &str) -> Result<(String, String)> {
            let (path, id) = uri.split_once('#').ok_or_else(|| {
                anyhow!("CLAP URI must be file:///.../lib.so#plugin_id, got: {uri}")
            })?;
            let path = path.strip_prefix("file://").unwrap_or(path).to_string();
            Ok((path, id.to_string()))
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
            for dir in Self::scan_dirs() {
                if !dir.exists() {
                    continue;
                }
                if let Ok(rd) = std::fs::read_dir(&dir) {
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
            unsafe {
                let bundle = PluginBundle::load(&lib)
                    .map_err(|e| anyhow!("CLAP load bundle failed: {e:?}"))?;
                let factory = bundle
                    .get_plugin_factory()
                    .ok_or_else(|| anyhow!("No plugin factory in bundle: {}", lib))?;
                let descriptor = factory
                    .plugin_descriptors()
                    .find(|d| {
                        d.id()
                            .map(|id| id.to_string_lossy() == plugin_id)
                            .unwrap_or(false)
                    })
                    .ok_or_else(|| anyhow!("Plugin id not found in bundle: {}", plugin_id))?;

                let inst = PluginInstance::<MyHost>::new(
                    |_| MyHostShared,
                    |_| (),
                    &bundle,
                    descriptor.id().unwrap(),
                    &self.host_info,
                )
                .map_err(|e| anyhow!("CLAP instantiate failed: {e:?}"))?;

                // For now, we'll leave params empty - can be added with clack-extensions later
                let params = Vec::new();

                Ok(Box::new(ClapInstance {
                    instance: inst,
                    started: None,
                    params,
                    sample_rate: self.cfg.sample_rate,
                    max_block: self.cfg.max_block,
                    input_copies: vec![vec![0.0; self.cfg.max_block]; 2],
                    note_ons: Vec::with_capacity(128),
                    note_offs: Vec::with_capacity(128),
                    pending_param_changes: Vec::new(),
                }))
            }
        }
    }

    pub struct ClapInstance {
        instance: PluginInstance<MyHost>,
        started: Option<StartedPluginAudioProcessor<MyHost>>,
        params: Vec<UnifiedParamInfo>,
        sample_rate: f64,
        max_block: usize,
        // Unfortunately we need to copy due to InputChannel requiring &mut [f32]
        input_copies: Vec<Vec<f32>>,
        // Pre-allocated event buffers (reused across calls)
        note_ons: Vec<NoteOnEvent>,
        note_offs: Vec<NoteOffEvent>,
        // Pending parameter changes
        pending_param_changes: Vec<(u32, f64)>,
    }

    impl ClapInstance {
        fn ensure_started(&mut self, frames: usize) -> Result<()> {
            if self.started.is_none() {
                let cfg = PluginAudioConfiguration {
                    sample_rate: self.sample_rate,
                    min_frames_count: 1, // Allow variable buffer sizes
                    max_frames_count: self.max_block as u32,
                };
                let activated = self
                    .instance
                    .activate(|_, _| (), cfg)
                    .map_err(|e| anyhow!("CLAP activate failed: {e:?}"))?;
                let started = activated
                    .start_processing()
                    .map_err(|e| anyhow!("CLAP start_processing failed: {e:?}"))?;
                self.started = Some(started);
            }
            Ok(())
        }
    }

    impl UniInstance for ClapInstance {
        fn process(
            &mut self,
            ctx: &ProcessCtx,
            audio_in: &[&[f32]],
            audio_out: &mut [&mut [f32]],
            events: &[MidiEvent],
        ) -> Result<()> {
            let frames = ctx.frames;
            self.ensure_started(frames)?;

            // Clear and reuse event buffers
            self.note_ons.clear();
            self.note_offs.clear();

            // Build typed note events
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

            // Sort by time
            self.note_ons.sort_by_key(|e| e.time());
            self.note_offs.sort_by_key(|e| e.time());

            // Copy input audio (InputChannel requires &mut [f32])
            for (i, &input_channel) in audio_in.iter().enumerate() {
                if i < self.input_copies.len() {
                    let len = frames.min(input_channel.len());
                    self.input_copies[i][..len].copy_from_slice(&input_channel[..len]);
                }
            }

            // Build audio ports
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

            let proc = self.started.as_mut().unwrap();
            let mut out_buffer = EventBuffer::new();
            let mut out_events = OutputEvents::from_buffer(&mut out_buffer);

            // Process parameter changes first
            if !self.pending_param_changes.is_empty() {
                let mut param_buffer = EventBuffer::new();
                for (id, value) in self.pending_param_changes.drain(..) {
                    // For parameter changes, use match_all() for PCKN (not note-specific)
                    param_buffer.push(&ParamValueEvent::new(
                        0,                 // time
                        ClapId::new(id),   // param_id
                        Pckn::match_all(), // pckn - wildcard for all notes
                        value,             // value
                        Cookie::empty(),   // cookie
                    ));
                }
                let param_events = InputEvents::from_buffer(&param_buffer);
                proc.process(
                    &in_audio,
                    &mut out_audio,
                    &param_events,
                    &mut out_events,
                    None,
                    None,
                )
                .map_err(|e| anyhow!("CLAP process (params) failed: {e:?}"))?;
            }

            // Process note events or empty buffer (ALWAYS process to generate audio)
            if !self.note_offs.is_empty() || !self.note_ons.is_empty() {
                // Process offs first
                if !self.note_offs.is_empty() {
                    let mut offs_buffer = EventBuffer::new();
                    for event in &self.note_offs {
                        offs_buffer.push(event);
                    }
                    let offs_events = InputEvents::from_buffer(&offs_buffer);
                    proc.process(
                        &in_audio,
                        &mut out_audio,
                        &offs_events,
                        &mut out_events,
                        None,
                        None,
                    )
                    .map_err(|e| anyhow!("CLAP process (offs) failed: {e:?}"))?;
                }

                // Then ons
                if !self.note_ons.is_empty() {
                    let mut ons_buffer = EventBuffer::new();
                    for event in &self.note_ons {
                        ons_buffer.push(event);
                    }
                    let ons_events = InputEvents::from_buffer(&ons_buffer);
                    proc.process(
                        &in_audio,
                        &mut out_audio,
                        &ons_events,
                        &mut out_events,
                        None,
                        None,
                    )
                    .map_err(|e| anyhow!("CLAP process (ons) failed: {e:?}"))?;
                }
            } else {
                // No events - still process to continue generating audio
                let empty_buffer = EventBuffer::new();
                let empty_events = InputEvents::from_buffer(&empty_buffer);
                proc.process(
                    &in_audio,
                    &mut out_audio,
                    &empty_events,
                    &mut out_events,
                    None,
                    None,
                )
                .map_err(|e| anyhow!("CLAP process (empty) failed: {e:?}"))?;
            }

            Ok(())
        }

        fn set_param(&mut self, key: &ParamKey, value: f32) {
            if let ParamKey::Clap(id) = key {
                self.pending_param_changes.push((*id, value as f64));
            }
        }

        fn get_param(&self, _key: &ParamKey) -> Option<f32> {
            None
        }

        fn params(&self) -> &[UnifiedParamInfo] {
            &self.params
        }
    }

    pub use ClapHostBackend as Backend;
}

#[cfg(feature = "clap-host")]
pub use clap_impl::Backend;
