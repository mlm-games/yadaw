use crate::audio_state::AudioState;
use crate::messages::AudioCommand;
use crossbeam_channel::Sender;
use std::sync::atomic::Ordering;
use std::sync::Arc;

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
    pub metronome_enabled: bool,
    pub count_in_enabled: bool,
    pub count_in_bars: u32,
    pub click_volume: f32,
}

impl Transport {
    pub fn new(audio_state: Arc<AudioState>, command_tx: Sender<AudioCommand>) -> Self {
        Self {
            audio_state,
            command_tx,
            metronome_enabled: false,
            count_in_enabled: false,
            count_in_bars: 1,
            click_volume: 0.7,
        }
    }

    pub fn play(&self) {
        let _ = self.command_tx.send(AudioCommand::Play);
    }

    pub fn stop(&self) {
        let _ = self.command_tx.send(AudioCommand::Stop);
    }

    pub fn pause(&self) {
        let _ = self.command_tx.send(AudioCommand::Pause);
    }

    pub fn record(&self) {
        let _ = self.command_tx.send(AudioCommand::Record);
    }

    pub fn toggle_playback(&self) {
        if self.audio_state.playing.load(Ordering::Relaxed) {
            self.stop();
        } else {
            self.play();
        }
    }

    pub fn set_position(&self, position: f64) {
        let _ = self.command_tx.send(AudioCommand::SetPosition(position));
    }

    pub fn set_bpm(&self, bpm: f32) {
        let _ = self.command_tx.send(AudioCommand::SetBPM(bpm));
    }

    pub fn get_position(&self) -> f64 {
        self.audio_state.get_position()
    }

    pub fn get_bpm(&self) -> f32 {
        self.audio_state.bpm.load()
    }

    pub fn is_playing(&self) -> bool {
        self.audio_state.playing.load(Ordering::Relaxed)
    }

    pub fn is_recording(&self) -> bool {
        self.audio_state.recording.load(Ordering::Relaxed)
    }

    pub fn rewind(&self) {
        self.set_position(0.0);
    }

    pub fn fast_forward(&self, beats: f64) {
        let current = self.get_position();
        let sample_rate = self.audio_state.sample_rate.load();
        let bpm = self.get_bpm();
        let samples_per_beat = (60.0 / bpm) * sample_rate;
        self.set_position(current + beats * samples_per_beat as f64);
    }

    pub fn rewind_beats(&self, beats: f64) {
        let current = self.get_position();
        let sample_rate = self.audio_state.sample_rate.load();
        let bpm = self.get_bpm();
        let samples_per_beat = (60.0 / bpm) * sample_rate;
        self.set_position((current - beats * samples_per_beat as f64).max(0.0));
    }
}
