//! Contains the offline audio rendering engine and export logic.

use crate::audio::AudioEngine;
use crate::audio_state::AudioState;
use crate::constants::MAX_BUFFER_SIZE;
use crate::messages::{ExportState, UIUpdate};
use crate::project::AppState;
use crate::time_utils::TimeConverter;
use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use hound::{SampleFormat, WavSpec, WavWriter};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub path: PathBuf,
    pub sample_rate: f32,
    pub bit_depth: u16,
    pub start_beat: f64,
    pub end_beat: f64,
    pub normalize: bool,
}

pub struct AudioExporter;

impl AudioExporter {
    /// Spawns a new thread to perform the audio export.
    /// Sends progress updates back to the UI via the provided channel.
    pub fn export_to_wav(
        app_state: AppState,
        audio_state: Arc<AudioState>,
        config: ExportConfig,
        ui_tx: Sender<UIUpdate>,
    ) {
        thread::spawn(move || {
            match Self::run_export_thread(app_state, audio_state, config.clone(), ui_tx.clone()) {
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

    fn run_export_thread(
        app_state: AppState,
        audio_state: Arc<AudioState>,
        config: ExportConfig,
        ui_tx: Sender<UIUpdate>,
    ) -> Result<PathBuf> {
        let spec = WavSpec {
            channels: 2,
            sample_rate: config.sample_rate as u32,
            bits_per_sample: config.bit_depth,
            sample_format: if config.bit_depth == 32 {
                SampleFormat::Float
            } else {
                SampleFormat::Int
            },
        };

        let temp_path = config.path.with_extension("tmp.wav");
        let mut writer = WavWriter::create(&temp_path, spec)
            .map_err(|e| anyhow!("Failed to create WAV file: {}", e))?;

        let snapshots = crate::audio_snapshot::build_track_snapshots(&app_state);
        let mut offline_engine =
            AudioEngine::new_for_offline_render(&snapshots, &audio_state, config.sample_rate)?;

        let converter = TimeConverter::new(config.sample_rate, app_state.bpm);
        let start_samples = converter.beats_to_samples(config.start_beat).round() as u64;
        let end_samples = converter.beats_to_samples(config.end_beat).round() as u64;
        let total_samples_to_render = end_samples.saturating_sub(start_samples);

        if total_samples_to_render == 0 {
            return Err(anyhow!("Export range is zero length."));
        }

        let mut current_sample_pos = start_samples as f64;
        let mut samples_rendered = 0u64;

        // For normalization pass
        let mut all_samples = if config.normalize {
            Vec::with_capacity((total_samples_to_render as usize) * 2)
        } else {
            Vec::new()
        };

        // --- STAGE 1: RENDERING ---
        let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Rendering(0.0)));

        while samples_rendered < total_samples_to_render {
            let remaining = total_samples_to_render - samples_rendered;
            let frames_to_process = (remaining as usize).min(MAX_BUFFER_SIZE);

            let mut output_buffer = vec![0.0f32; frames_to_process * 2];
            let mut plugin_time_ms = 0.0;

            offline_engine.process_audio(
                &mut output_buffer,
                frames_to_process,
                2,
                current_sample_pos,
                &mut plugin_time_ms,
            );

            if config.normalize {
                all_samples.extend_from_slice(&output_buffer);
            } else {
                Self::write_samples_to_writer(&mut writer, &output_buffer, config.bit_depth)?;
            }

            current_sample_pos += frames_to_process as f64;
            samples_rendered += frames_to_process as u64;

            let progress = samples_rendered as f32 / total_samples_to_render as f32;
            let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Rendering(
                progress,
            )));
        }

        // --- STAGE 2: NORMALIZING (if needed) ---
        if config.normalize {
            let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Normalizing));

            let peak = all_samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            if peak > 1e-6 {
                let gain = 0.99 / peak; // Headroom
                for sample in &mut all_samples {
                    *sample *= gain;
                }
            }
            Self::write_samples_to_writer(&mut writer, &all_samples, config.bit_depth)?;
        }

        // --- STAGE 3: FINALIZING ---
        let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Finalizing));
        writer.finalize()?;
        std::fs::rename(&temp_path, &config.path)?;

        let _ = ui_tx.send(UIUpdate::ExportStateUpdate(ExportState::Complete(
            config.path.to_string_lossy().into_owned(),
        )));
        Ok(config.path)
    }

    fn write_samples_to_writer(
        writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>,
        samples: &[f32],
        bit_depth: u16,
    ) -> Result<()> {
        for &sample in samples {
            match bit_depth {
                16 => writer.write_sample((sample * i16::MAX as f32) as i16)?,
                24 => writer.write_sample((sample * 8_388_607.0) as i32)?,
                32 => writer.write_sample(sample)?,
                _ => return Err(anyhow!("Unsupported bit depth")),
            }
        }
        Ok(())
    }
}
