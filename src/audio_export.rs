//! Contains the offline audio rendering engine and export logic.

use crate::audio::AudioEngine;
use crate::audio_state::{AudioState, TrackSnapshot};
use crate::constants::{DEFAULT_SAMPLE_RATE, MAX_BUFFER_SIZE};
use crate::messages::UIUpdate;
use crate::project::AppState;
use crate::time_utils::TimeConverter;
use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use hound::{SampleFormat, WavSpec, WavWriter};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;

/// Configuration for an audio export job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub path: PathBuf,
    pub sample_rate: f32,
    pub bit_depth: u16,
    pub start_beat: f64,
    pub end_beat: f64,
}

/// Manages the audio export process on a separate thread.
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
            let result = Self::run_export_thread(app_state, audio_state, config, ui_tx.clone());
            if let Err(e) = result {
                let _ = ui_tx.send(UIUpdate::Error(format!("Export failed: {}", e)));
            }
        });
    }

    /// The core offline rendering loop executed on the export thread.
    fn run_export_thread(
        app_state: AppState,
        audio_state: Arc<AudioState>,
        config: ExportConfig,
        ui_tx: Sender<UIUpdate>,
    ) -> Result<()> {
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

        let mut writer = WavWriter::create(&config.path, spec)
            .map_err(|e| anyhow!("Failed to create WAV file: {}", e))?;

        // 1. Build an offline version of the AudioEngine
        let snapshots = crate::audio_snapshot::build_track_snapshots(&app_state);
        let mut offline_engine =
            AudioEngine::new_for_offline_render(&snapshots, &audio_state, config.sample_rate)?;

        // 2. Calculate total samples to render
        let converter = TimeConverter::new(config.sample_rate, app_state.bpm);
        let start_samples = converter.beats_to_samples(config.start_beat).round() as u64;
        let end_samples = converter.beats_to_samples(config.end_beat).round() as u64;
        let total_samples_to_render = end_samples.saturating_sub(start_samples);

        if total_samples_to_render == 0 {
            return Err(anyhow!("Export range is zero length."));
        }

        let mut current_sample_pos = start_samples as f64;
        let mut samples_rendered = 0u64;

        // 3. Main offline rendering loop
        while samples_rendered < total_samples_to_render {
            let remaining = total_samples_to_render - samples_rendered;
            let frames_to_process = (remaining as usize).min(MAX_BUFFER_SIZE);

            // Render a block of audio
            let mut output_buffer = vec![0.0f32; frames_to_process * 2];
            let mut plugin_time_ms = 0.0;

            offline_engine.process_audio(
                &mut output_buffer,
                frames_to_process,
                2, // Stereo
                current_sample_pos,
                &mut plugin_time_ms,
            );

            // Write samples to WAV file
            for sample in output_buffer {
                match config.bit_depth {
                    16 => writer.write_sample((sample * i16::MAX as f32) as i16)?,
                    24 => writer.write_sample((sample * 8_388_607.0) as i32)?,
                    32 => writer.write_sample(sample)?,
                    _ => return Err(anyhow!("Unsupported bit depth")),
                }
            }

            current_sample_pos += frames_to_process as f64;
            samples_rendered += frames_to_process as u64;

            // Send progress update to UI
            let progress = samples_rendered as f32 / total_samples_to_render as f32;
            let _ = ui_tx.send(UIUpdate::ExportProgress(progress));
        }

        writer.finalize()?;
        let _ = ui_tx.send(UIUpdate::Info("Export completed successfully!".to_string()));
        Ok(())
    }
}
