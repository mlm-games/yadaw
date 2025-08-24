use crate::audio::AudioEngine;
use crate::automation::AutomationEngine;
use crate::midi_engine::MidiEngine;
use crate::mixer::MixerEngine;
use crate::performance::{PerformanceMetrics, PerformanceMonitor};
use crate::project::AppState;
use crate::project_manager::ProjectManager;
use crate::track_manager::{TrackManager, TrackType};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct DawCore {
    pub state: Arc<RwLock<AppState>>,
    pub mixer: Arc<RwLock<MixerEngine>>,
    pub track_manager: Arc<RwLock<TrackManager>>,
    pub project_manager: Arc<RwLock<ProjectManager>>,
    pub automation: Arc<RwLock<AutomationEngine>>,
    pub midi_engine: Arc<RwLock<MidiEngine>>,
    pub performance: Arc<RwLock<PerformanceMonitor>>,
}

impl DawCore {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(AppState::default())),
            mixer: Arc::new(RwLock::new(MixerEngine::new())),
            track_manager: Arc::new(RwLock::new(TrackManager::new())),
            project_manager: Arc::new(RwLock::new(ProjectManager::new())),
            automation: Arc::new(RwLock::new(AutomationEngine::new())),
            midi_engine: Arc::new(RwLock::new(MidiEngine::new())),
            performance: Arc::new(RwLock::new(PerformanceMonitor::new())),
        }
    }

    pub fn process_audio_cycle(&self, position_beats: f64, buffer_size: usize, sample_rate: f32) {
        let start_time = Instant::now();

        // Apply automation (example)
        let track_count = {
            let state = self.state.read();
            state.tracks.len()
        };

        let mut automation = self.automation.write();
        for track_idx in 0..track_count {
            if let Some(volume) = automation.get_value(track_idx, position_beats) {
                self.state.write().tracks[track_idx].volume = volume;
            }
            if let Some(pan) = automation.get_value(track_idx + 1000, position_beats) {
                self.state.write().tracks[track_idx].pan = (pan - 0.5) * 2.0;
            }
        }

        let processing_time = start_time.elapsed();
        let cpu_usage = processing_time.as_secs_f32() / (buffer_size as f32 / sample_rate);

        let metrics = PerformanceMetrics {
            cpu_usage,
            memory_usage: self.estimate_memory_usage(),
            disk_streaming_rate: 0.0,
            audio_buffer_health: 1.0 - cpu_usage,
            plugin_processing_time: Duration::from_millis(0),
            xruns: 0,
            latency_ms: (buffer_size as f32 / sample_rate) * 1000.0,
        };

        self.performance.write().update_metrics(metrics);
    }

    fn estimate_memory_usage(&self) -> usize {
        let state = self.state.read();
        let mut total = 0;

        for track in &state.tracks {
            for clip in &track.audio_clips {
                total += clip.samples.len() * std::mem::size_of::<f32>();
            }

            for clip in &track.midi_clips {
                total += clip.notes.len() * std::mem::size_of::<crate::model::MidiNote>();
            }
        }

        total
    }

    pub fn create_new_project(&self, name: String) {
        let mut state = self.state.write();
        let mut track_manager = self.track_manager.write();

        // Clear current project
        state.tracks.clear();
        state.master_volume = 0.8;
        state.bpm = 120.0;

        // Create default tracks using the factory
        state.tracks.push(
            track_manager.create_track(TrackType::Audio, Some(format!("{} - Audio 1", name))),
        );
        state
            .tracks
            .push(track_manager.create_track(TrackType::Midi, Some(format!("{} - MIDI 1", name))));

        // Create default bus
        self.mixer.write().create_bus("Reverb Bus".to_string());
    }

    pub fn export_audio(
        &self,
        _path: &std::path::Path,
        _format: ExportFormat,
    ) -> Result<(), String> {
        // Stub
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    Wav16,
    Wav24,
    Wav32Float,
    Mp3(u32), // bitrate
    Flac,
    Ogg,
}
