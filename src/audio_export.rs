//! Contains the offline audio rendering engine and export logic.

use crate::audio::AudioEngine;
use crate::audio_state::AudioState;
use crate::constants::MAX_BUFFER_SIZE;
use crate::messages::{ExportState, UIUpdate};
use crate::project::AppState;
use crate::time_utils::TimeConverter;

use anyhow::{anyhow, bail, Result};
use crossbeam_channel::Sender;
use dissonia::prelude::*;
use serde::{Deserialize, Serialize};

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Wav,
    Flac,
    Ogg,
}

impl ExportFormat {
    fn default_extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub path: PathBuf,
    pub format: Option<ExportFormat>,
    pub sample_rate: f32,
    pub bit_depth: u16,
    pub start_beat: f64,
    pub end_beat: f64,
    pub normalize: bool,
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
        ui_tx: Sender<UIUpdate>,
    ) {
        thread::spawn(move || {
            let result = run_export(app_state, audio_state, &config, &ui_tx);
            match result {
                Ok(path) => {
                    let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Complete(
                        path.to_string_lossy().into_owned(),
                    )));
                }
                Err(e) => {
                    let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Error(
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
    ui_tx: &Sender<UIUpdate>,
) -> Result<PathBuf> {
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
            ExportFormat::Wav => write_wav(file, &pcm, config, layout, sample_format)?,
            ExportFormat::Flac => write_flac(file, &pcm, config, layout, sample_format)?,
            ExportFormat::Ogg => write_ogg(file, &pcm, config, layout)?,
        }
    }

    std::fs::rename(&temp_path, &output_path)
        .map_err(|e| anyhow!("Failed to move temp file: {e}"))?;

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
        _ => bail!("OGG/Opus export requires sample rate of 8000, 12000, 16000, 24000, 44100, or 48000 Hz. Current: {sample_rate} Hz"),
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

fn send(ui_tx: &Sender<UIUpdate>, state: ExportState) {
    let _ = ui_tx.send(UIUpdate::ExportStateUpdate(state));
}
