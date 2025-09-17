#[cfg(feature = "clap-host")]
mod clap_impl {
    use anyhow::{Result, anyhow};
    use std::path::{Path, PathBuf};

    use clack_host::events::event_types::{NoteOffEvent, NoteOnEvent};
    use clack_host::prelude::*;
    use clack_host::prelude::{AudioPortBuffer, AudioPortBufferType, AudioPorts, InputChannel};

    use crate::model::plugin_api::{
        BackendKind, HostConfig, MidiEvent, ParamKey, PluginBackend, PluginInstance, ProcessCtx,
        UnifiedParamInfo, UnifiedPluginInfo,
    };

    // Minimal host handlers, exactly as in the README
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
            // Find shared objects inside a bundle dir, or use the file directly
            let mut libs = Vec::new();
            if path.is_file() {
                libs.push(path.to_path_buf());
            } else if path.is_dir() {
                if let Ok(rd) = std::fs::read_dir(path) {
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
            }

            for lib in libs {
                unsafe {
                    if let Ok(bundle) = PluginBundle::load(lib.to_string_lossy().as_ref()) {
                        if let Some(factory) = bundle.get_plugin_factory() {
                            for d in factory.plugin_descriptors() {
                                // Name
                                let name = d
                                    .name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .or_else(|| {
                                        lib.file_stem()
                                            .and_then(|s| s.to_str())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_else(|| "Unknown CLAP".to_string());

                                // ID
                                let id = d
                                    .id()
                                    .map(|id| id.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "unknown_id".to_string());

                                // Features → instrument?
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
        }

        fn parse_uri(uri: &str) -> Result<(String, String)> {
            // Expected: file:///abs/path/lib.so#plugin_id
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

        fn instantiate(&self, uri: &str) -> Result<Box<dyn PluginInstance>> {
            // Validate the bundle + id by actually instantiating once, then keep the instance
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

                // Create the instance and keep it un-activated here; process() will activate per block.
                let inst = clack_host::plugin::PluginInstance::<MyHost>::new(
                    |_| MyHostShared,
                    |_| (),
                    &bundle,
                    descriptor.id().unwrap(),
                    &self.host_info,
                )
                .map_err(|e| anyhow!("CLAP instantiate failed: {e:?}"))?;

                // Params: keep empty for now; you’ll add a param editor later
                let params: Vec<UnifiedParamInfo> = Vec::new();

                Ok(Box::new(ClapInstance {
                    instance: inst,
                    params,
                    cfg: self.cfg.clone(),
                }))
            }
        }
    }

    struct ClapInstance {
        instance: clack_host::plugin::PluginInstance<MyHost>,
        params: Vec<UnifiedParamInfo>,
        cfg: HostConfig,
    }

    impl PluginInstance for ClapInstance {
        fn process(
            &mut self,
            ctx: &ProcessCtx,
            audio_in: &[&[f32]],
            audio_out: &mut [&mut [f32]],
            events: &[MidiEvent],
        ) -> Result<()> {
            // 1) Activate
            let audio_cfg = PluginAudioConfiguration {
                sample_rate: self.cfg.sample_rate,
                min_frames_count: ctx.frames as u32,
                max_frames_count: ctx.frames as u32,
            };
            let activated = self
                .instance
                .activate(|_, _| (), audio_cfg)
                .map_err(|e| anyhow!("CLAP activate failed: {e:?}"))?;

            // 2) Note events
            let mut note_ons: Vec<NoteOnEvent> = Vec::new();
            let mut note_offs: Vec<NoteOffEvent> = Vec::new();
            for e in events {
                let st = e.status & 0xF0;
                let ch = (e.status & 0x0F) as u16;
                let key = e.data1 as u16;
                let vel = (e.data2 as f32 / 127.0) as f64;
                let pckn = Pckn::new(0u16, ch, key, key as u32);
                match st {
                0x90 if e.data2 != 0 => note_ons.push(NoteOnEvent::new(
                    e.time_frames.clamp(0, ctx.frames as i64) as u32, pckn, vel)),
                0x80 | 0x90 /* vel=0 */ => note_offs.push(NoteOffEvent::new(
                    e.time_frames.clamp(0, ctx.frames as i64) as u32, pckn, vel)),
                _ => {}
            }
            }

            // 3) Start
            let mut proc = activated
                .start_processing()
                .map_err(|e| anyhow!("start_processing failed: {e:?}"))?;

            // 4) Events
            let offs = InputEvents::from_buffer(&note_offs);
            let ons = InputEvents::from_buffer(&note_ons);
            let mut out_ev_buf = EventBuffer::new();
            let mut out_events = OutputEvents::from_buffer(&mut out_ev_buf);

            // 5) Audio ports
            let mut in_copies: Vec<Vec<f32>> = audio_in
                .iter()
                .map(|ch| ch[..ctx.frames].to_vec())
                .collect();

            let mut input_ports = AudioPorts::with_capacity(in_copies.len(), 1); // usize
            let mut output_ports = AudioPorts::with_capacity(audio_out.len(), 1); // usize

            let mut in_audio = input_ports.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only(
                    in_copies
                        .iter_mut()
                        .map(|b| InputChannel::constant(&mut b[..])),
                ),
            }]);

            let mut out_audio = output_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    audio_out.iter_mut().map(|b| (*b).as_mut()),
                ),
            }]);

            // 6) Process: OFFs then ONs
            proc.process(
                &in_audio,
                &mut out_audio,
                &offs,
                &mut out_events,
                None,
                None,
            )
            .map_err(|e| anyhow!("process(off) failed: {e:?}"))?;
            proc.process(&in_audio, &mut out_audio, &ons, &mut out_events, None, None)
                .map_err(|e| anyhow!("process(on) failed: {e:?}"))?;

            // 7) Stop & deactivate
            let activated_back = proc.stop_processing();
            self.instance.deactivate(activated_back);
            Ok(())
        }

        fn set_param(&mut self, key: &ParamKey, value: f32) {
            match *key {
                ParamKey::Clap(_id) => {
                    // TODO: query and set via param extension (varies with Clack revision)
                    let _ = value;
                }
                ParamKey::Lv2(_) => { /* handled by LV2 backend */ }
            }
        }

        fn get_param(&self, _key: &ParamKey) -> Option<f32> {
            None // TODO: map via param extension later
        }

        fn params(&self) -> &[UnifiedParamInfo] {
            &self.params
        }
    }

    pub use ClapHostBackend as Backend;
}

#[cfg(feature = "clap-host")]
pub use clap_impl::Backend;
