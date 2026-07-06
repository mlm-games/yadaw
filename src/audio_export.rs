use crate::audio::AudioEngine;
use crate::audio_state::AudioState;
use crate::constants::MAX_BUFFER_SIZE;
use crate::messages::{ExportConfig, ExportFormat, ExportState, UIUpdate, UiTx};
use crate::project::AppState;
use crate::time_utils::TimeConverter;

use anyhow::{Result, anyhow, bail};
use dissonia::prelude::*;
#[cfg(target_os = "android")]
use rlobkit_dialogs::PlatformFile;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::HtmlAnchorElement;

use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::PathBuf;
use std::sync::Arc;

impl ExportFormat {
    pub fn default_extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
        }
    }
}

#[cfg(target_os = "android")]
fn export_through_cache_then_copy(config: &ExportConfig) -> ExportConfig {
    let mut normalized = config.clone();
    if normalized.export_uri.is_some() {
        normalized.path = crate::paths::cache_dir().join("android_export_temp");
    }
    normalized
}

impl ExportConfig {
    fn channel_layout(&self) -> ChannelLayout {
        ChannelLayout::STEREO
    }

    fn sample_format(&self) -> Result<SampleFormat> {
        let format = self.format.unwrap_or(ExportFormat::Wav);
        match (format, self.bit_depth) {
            (_, 16) => Ok(SampleFormat::I16),
            (_, 24) => Ok(SampleFormat::I24),
            (ExportFormat::Wav, 32) => Ok(SampleFormat::F32),
            (ExportFormat::Flac, 32) => bail!("FLAC does not support 32-bit float samples"),
            (ExportFormat::Ogg, _) => Ok(SampleFormat::F32),
            (_, d) => bail!("Unsupported bit depth: {d}"),
        }
    }

    fn output_path(&self) -> PathBuf {
        let format = self.format.unwrap_or(ExportFormat::Wav);
        self.path.with_extension(format.default_extension())
    }

    fn resolved_format(&self) -> ExportFormat {
        self.format.unwrap_or(ExportFormat::Wav)
    }
}

pub struct AudioExporter;

impl AudioExporter {
    pub fn export(
        app_state: AppState,
        audio_state: Arc<AudioState>,
        config: ExportConfig,
        ui_tx: UiTx,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        crate::runtime::RT.spawn_blocking(move || {
            let result = run_export(app_state, audio_state, &config, &ui_tx);
            match result {
                Ok(path) => {
                    let _ = ui_tx.send_sync(UIUpdate::ExportStateUpdate(ExportState::Complete(
                        path.to_string_lossy().into_owned(),
                    )));
                }
                Err(e) => {
                    let _ = ui_tx.send_sync(UIUpdate::ExportStateUpdate(ExportState::Error(
                        e.to_string(),
                    )));
                }
            }
        });
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let result = run_export_wasm(app_state, audio_state, &config).await;
            match result {
                Ok(filename) => {
                    let _ = ui_tx
                        .send_sync(UIUpdate::ExportStateUpdate(ExportState::Complete(filename)));
                }
                Err(e) => {
                    let _ = ui_tx.send_sync(UIUpdate::ExportStateUpdate(ExportState::Error(
                        e.to_string(),
                    )));
                }
            }
        });
    }
}

fn run_export(
    app_state: AppState,
    audio_state: Arc<AudioState>,
    config: &ExportConfig,
    ui_tx: &UiTx,
) -> Result<PathBuf> {
    #[cfg(target_os = "android")]
    let config = export_through_cache_then_copy(config);

    #[cfg(not(target_os = "android"))]
    let config = config.clone();

    let sample_format = config.sample_format()?;
    let layout = config.channel_layout();
    let channels = layout.count() as usize;
    let format = config.resolved_format();

    let converter = TimeConverter::new(config.sample_rate, app_state.bpm);
    let start_sample = converter.beats_to_samples(config.start_beat).round() as u64;
    let end_sample = converter.beats_to_samples(config.end_beat).round() as u64;
    let total_frames = end_sample.saturating_sub(start_sample);

    if total_frames == 0 {
        bail!("Export range is zero length.");
    }

    let snapshots = crate::audio_snapshot::build_track_snapshots(&app_state);
    let mut engine =
        AudioEngine::new_for_offline_render(&snapshots, &audio_state, config.sample_rate)?;

    send(ui_tx, ExportState::Rendering(0.0));

    let total_samples = total_frames as usize * channels;
    let mut pcm = Vec::<f32>::with_capacity(total_samples);
    let mut current_pos = start_sample as f64;
    let mut frames_done = 0u64;

    while frames_done < total_frames {
        let batch = ((total_frames - frames_done) as usize).min(MAX_BUFFER_SIZE);
        let mut buf = vec![0.0f32; batch * channels];
        let mut plugin_time_ms = 0.0f32;

        engine.process_audio(&mut buf, batch, channels, current_pos, &mut plugin_time_ms);

        pcm.extend_from_slice(&buf);
        current_pos += batch as f64;
        frames_done += batch as u64;

        send(
            ui_tx,
            ExportState::Rendering(frames_done as f32 / total_frames as f32),
        );
    }

    if config.normalize {
        send(ui_tx, ExportState::Normalizing);
        let peak = pcm.iter().copied().map(f32::abs).fold(0.0f32, f32::max);
        if peak > 1e-6 {
            let gain = 0.99 / peak;
            for s in &mut pcm {
                *s *= gain;
            }
        }
    }

    send(ui_tx, ExportState::Finalizing);

    let output_path = config.output_path();
    let temp_path = output_path.with_extension("tmp");

    {
        let file = BufWriter::new(
            File::create(&temp_path).map_err(|e| anyhow!("Cannot create temp file: {e}"))?,
        );

        match format {
            ExportFormat::Wav => write_wav(file, &pcm, &config, layout, sample_format)?,
            ExportFormat::Flac => write_flac(file, &pcm, &config, layout, sample_format)?,
            ExportFormat::Ogg => write_ogg(file, &pcm, &config, layout)?,
        }
    }

    std::fs::rename(&temp_path, &output_path)
        .map_err(|e| anyhow!("Failed to move temp file: {e}"))?;

    #[cfg(target_os = "android")]
    if let Some(uri) = &config.export_uri {
        let target = PlatformFile::from_uri(
            std::path::Path::new(uri.as_str())
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("export.bin"),
            uri.as_str(),
            None,
            None,
        );
        rlobkit_dialogs::RlobKit::write_file_from_path(&target, &output_path)
            .map_err(|e| anyhow!("Failed to write exported file to SAF URI '{uri}': {e}"))?;
        let _ = std::fs::remove_file(&output_path);
        return Ok(PathBuf::from(uri));
    }

    Ok(output_path)
}

fn write_wav(
    sink: BufWriter<File>,
    pcm: &[f32],
    config: &ExportConfig,
    layout: ChannelLayout,
    sample_format: SampleFormat,
) -> Result<()> {
    let audio_spec = AudioSpec::new(config.sample_rate as u32, layout, sample_format);

    let mut encoder = PcmEncoder::new(audio_spec)?;
    let mut muxer = WavMuxer::new(sink);

    let track = muxer.add_track(TrackSpec::new(
        encoder.codec_parameters().clone(),
        TimeBase::audio_sample_rate(config.sample_rate as u32),
    ))?;

    let mut sink = muxer.track_writer(track);
    encode_pcm_from_f32(&mut encoder, pcm, sample_format, &mut sink)?;
    encoder.flush(&mut sink)?;
    drop(sink);

    muxer.finalize()?;
    Ok(())
}

fn write_flac(
    sink: BufWriter<File>,
    pcm: &[f32],
    config: &ExportConfig,
    layout: ChannelLayout,
    sample_format: SampleFormat,
) -> Result<()> {
    let audio_spec = AudioSpec::new(config.sample_rate as u32, layout, sample_format);

    let mut encoder = FlacEncoder::new(audio_spec)?;
    let mut muxer = FlacMuxer::new(sink);

    let track = muxer.add_track(TrackSpec::new(
        encoder.codec_parameters().clone(),
        TimeBase::audio_sample_rate(config.sample_rate as u32),
    ))?;

    {
        let mut sink = muxer.track_writer(track);
        encode_pcm_from_f32(&mut encoder, pcm, sample_format, &mut sink)?;
        encoder.flush(&mut sink)?;
    }

    if let Some(info) = encoder.codec_parameters().flac_stream_info() {
        muxer.update_stream_info(track, info.md5)?;
    }

    muxer.finalize()?;
    Ok(())
}

fn write_ogg(
    sink: BufWriter<File>,
    pcm: &[f32],
    config: &ExportConfig,
    layout: ChannelLayout,
) -> Result<()> {
    let sample_rate = config.sample_rate as u32;
    let output_rate: u32 = match sample_rate {
        8000 | 12000 | 16000 | 24000 | 48000 => sample_rate,
        44100 => 48000,
        _ => bail!(
            "OGG/Opus export requires sample rate of 8000, 12000, 16000, 24000, 44100, or 48000 Hz. Current: {sample_rate} Hz"
        ),
    };

    let pcm_data = if output_rate != sample_rate {
        let ratio = output_rate as f64 / sample_rate as f64;
        let new_len = (pcm.len() as f64 * ratio) as usize;
        let mut resampled = Vec::with_capacity(new_len);
        for i in 0..new_len {
            let src_idx = i as f64 / ratio;
            let lo = src_idx.floor() as usize;
            let hi = (lo + 1).min(pcm.len() - 1);
            let t = src_idx.fract() as f32;
            resampled.push(pcm[lo] * (1.0 - t) + pcm[hi] * t);
        }
        resampled
    } else {
        pcm.to_vec()
    };

    let audio_spec = AudioSpec::new(output_rate, layout, SampleFormat::F32);

    let mut encoder = OpusEncoder::new(audio_spec)?;
    let mut muxer = OggOpusMuxer::new(sink);

    let track = muxer.add_track(TrackSpec::new(
        encoder.codec_parameters().clone(),
        TimeBase::audio_sample_rate(output_rate),
    ))?;

    {
        let mut sink = muxer.track_writer(track);
        encode_pcm_from_f32(&mut encoder, &pcm_data, SampleFormat::F32, &mut sink)?;
        encoder.flush(&mut sink)?;
    }

    muxer.finalize()?;
    Ok(())
}

fn encode_pcm_from_f32(
    encoder: &mut dyn Encoder,
    pcm: &[f32],
    sample_format: SampleFormat,
    sink: &mut dyn PacketSink,
) -> Result<()> {
    match sample_format {
        SampleFormat::I16 => {
            let samples: Vec<i16> = pcm
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect();
            encoder.encode(AudioBufferRef::I16(&samples), sink)?;
        }
        SampleFormat::I24 => {
            const MAX24: f32 = 8_388_607.0;
            let samples: Vec<i32> = pcm
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * MAX24) as i32)
                .collect();
            encoder.encode(AudioBufferRef::I24(&samples), sink)?;
        }
        SampleFormat::F32 => {
            let clamped: Vec<f32> = pcm.iter().map(|&s| s.clamp(-1.0, 1.0)).collect();
            encoder.encode(AudioBufferRef::F32(&clamped), sink)?;
        }
        _ => bail!("Unsupported sample format for export: {:?}", sample_format),
    }
    Ok(())
}

fn send(ui_tx: &UiTx, state: ExportState) {
    let _ = ui_tx.send_sync(UIUpdate::ExportStateUpdate(state));
}

#[cfg(target_arch = "wasm32")]
async fn run_export_wasm(
    app_state: AppState,
    audio_state: Arc<AudioState>,
    config: &ExportConfig,
) -> Result<String> {
    let sample_format = config.sample_format()?;
    let layout = config.channel_layout();
    let channels = layout.count() as usize;
    let _ = sample_format;

    let converter = TimeConverter::new(config.sample_rate, app_state.bpm);
    let start_sample = converter.beats_to_samples(config.start_beat).round() as u64;
    let end_sample = converter.beats_to_samples(config.end_beat).round() as u64;
    let total_frames = end_sample.saturating_sub(start_sample);

    if total_frames == 0 {
        bail!("Export range is zero length.");
    }

    let snapshots = crate::audio_snapshot::build_track_snapshots(&app_state);
    let mut engine =
        AudioEngine::new_for_offline_render(&snapshots, &audio_state, config.sample_rate)?;

    let total_samples = total_frames as usize * channels;
    let mut pcm = Vec::<f32>::with_capacity(total_samples);
    let mut current_pos = start_sample as f64;
    let mut frames_done = 0u64;

    while frames_done < total_frames {
        let batch = ((total_frames - frames_done) as usize).min(MAX_BUFFER_SIZE);
        let mut buf = vec![0.0f32; batch * channels];
        let mut plugin_time_ms = 0.0f32;

        engine.process_audio(&mut buf, batch, channels, current_pos, &mut plugin_time_ms);

        pcm.extend_from_slice(&buf);
        current_pos += batch as f64;
        frames_done += batch as u64;
    }

    if config.normalize {
        let peak = pcm.iter().copied().map(f32::abs).fold(0.0f32, f32::max);
        if peak > 1e-6 {
            let gain = 0.99 / peak;
            for s in &mut pcm {
                *s *= gain;
            }
        }
    }

    let filename = config
        .output_path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("export.wav")
        .to_string();

    let spec = hound::WavSpec {
        channels: channels as u16,
        sample_rate: config.sample_rate as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let wav_bytes = {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec)
                .map_err(|e| anyhow!("Failed to create WAV writer: {e}"))?;
            for &sample in &pcm {
                writer
                    .write_sample(sample)
                    .map_err(|e| anyhow!("Failed to write sample: {e}"))?;
            }
            writer
                .finalize()
                .map_err(|e| anyhow!("Failed to finalize WAV: {e}"))?;
        }
        cursor.into_inner()
    };

    let uint8 = js_sys::Uint8Array::from(&wav_bytes[..]);
    let parts = js_sys::Array::new();
    parts.push(&uint8.into());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)
        .map_err(|e| anyhow!("Failed to create Blob: {:?}", e))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| anyhow!("Failed to create object URL: {:?}", e))?;

    let window = web_sys::window().ok_or_else(|| anyhow!("No window"))?;
    let document = window.document().ok_or_else(|| anyhow!("No document"))?;
    let anchor = document
        .create_element("a")
        .map_err(|e| anyhow!("Failed to create anchor: {:?}", e))?
        .dyn_into::<HtmlAnchorElement>()
        .map_err(|e| anyhow!("Failed to cast anchor: {:?}", e))?;

    anchor.set_href(&url);
    anchor.set_download(&filename);
    anchor.click();

    let _ = web_sys::Url::revoke_object_url(&url);

    Ok(filename)
}
