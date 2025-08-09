use crate::audio_state::AudioState;
use crate::constants::DEFAULT_BPM;
use crate::state::AudioCommand;
use crossbeam_channel::Sender;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportState {
    Stopped,
    Playing,
    Recording,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopMode {
    Off,
    Pattern,
    Range,
}

pub struct Transport {
    audio_state: Arc<AudioState>,
    command_tx: Sender<AudioCommand>,
    pub state: TransportState,
    pub loop_mode: LoopMode,
    pub loop_start: f64,        // in beats
    pub loop_end: f64,          // in beats
    pub punch_in: Option<f64>,  // in beats
    pub punch_out: Option<f64>, // in beats
    pub metronome_enabled: bool,
    pub metronome_volume: f32,
    pub count_in_bars: u32,
    pub pre_roll_bars: u32,
}

impl Transport {
    pub fn new(audio_state: Arc<AudioState>, command_tx: Sender<AudioCommand>) -> Self {
        Self {
            audio_state,
            command_tx,
            state: TransportState::Stopped,
            loop_mode: LoopMode::Off,
            loop_start: 0.0,
            loop_end: 4.0,
            punch_in: None,
            punch_out: None,
            metronome_enabled: false,
            metronome_volume: 0.7,
            count_in_bars: 0,
            pre_roll_bars: 0,
        }
    }

    pub fn play(&mut self) {
        match self.state {
            TransportState::Stopped | TransportState::Paused => {
                let _ = self.command_tx.send(AudioCommand::Play);
                self.state = TransportState::Playing;
            }
            _ => {}
        }
    }

    pub fn stop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Stop);
        self.state = TransportState::Stopped;
    }

    pub fn pause(&mut self) {
        if self.state == TransportState::Playing {
            let _ = self.command_tx.send(AudioCommand::Pause);
            self.state = TransportState::Paused;
        }
    }

    pub fn toggle_playback(&mut self) {
        match self.state {
            TransportState::Playing => self.stop(),
            _ => self.play(),
        }
    }

    pub fn start_recording(&mut self, track_id: usize) {
        if self.state != TransportState::Recording {
            let _ = self.command_tx.send(AudioCommand::StartRecording(track_id));
            self.state = TransportState::Recording;
        }
    }

    pub fn stop_recording(&mut self) {
        if self.state == TransportState::Recording {
            let _ = self.command_tx.send(AudioCommand::StopRecording);
            self.state = TransportState::Playing;
        }
    }

    pub fn set_position(&mut self, position_beats: f64) {
        let sample_rate = self.audio_state.sample_rate.load();
        let bpm = self.audio_state.bpm.load();
        let position_samples = (position_beats * 60.0 / bpm as f64) * sample_rate as f64;
        self.audio_state.set_position(position_samples);
    }

    pub fn get_position_beats(&self) -> f64 {
        let position = self.audio_state.get_position();
        let sample_rate = self.audio_state.sample_rate.load();
        let bpm = self.audio_state.bpm.load();
        (position / sample_rate as f64) * (bpm as f64 / 60.0)
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.audio_state.bpm.store(bpm.clamp(20.0, 999.0));
    }

    pub fn get_bpm(&self) -> f32 {
        self.audio_state.bpm.load()
    }

    pub fn set_loop_range(&mut self, start: f64, end: f64) {
        self.loop_start = start;
        self.loop_end = end;
    }

    pub fn toggle_loop(&mut self) {
        self.loop_mode = match self.loop_mode {
            LoopMode::Off => LoopMode::Range,
            _ => LoopMode::Off,
        };
    }

    pub fn toggle_metronome(&mut self) {
        self.metronome_enabled = !self.metronome_enabled;
    }

    pub fn format_time(&self, beats: f64) -> String {
        let bars = (beats / 4.0) as i32 + 1;
        let beat = (beats % 4.0) as i32 + 1;
        let sixteenth = ((beats % 1.0) * 4.0) as i32 + 1;
        format!("{:03}:{:02}:{:02}", bars, beat, sixteenth)
    }

    pub fn format_time_seconds(&self, position_samples: f64) -> String {
        let sample_rate = self.audio_state.sample_rate.load();
        let seconds = position_samples / sample_rate as f64;
        let minutes = (seconds / 60.0) as i32;
        let secs = seconds % 60.0;
        format!("{:02}:{:05.2}", minutes, secs)
    }
}
