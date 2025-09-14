use std::sync::atomic::Ordering;

use egui::scroll_area::ScrollSource;

use super::*;
use crate::messages::AudioCommand;
use crate::transport::{LoopMode, Transport, TransportState};

pub struct TransportUI {
    pub transport: Option<Transport>,
    loop_start_input: String,
    loop_end_input: String,
    bpm_input: String,
    position_display: String,
}

impl TransportUI {
    pub fn new(transport: Transport) -> Self {
        let bpm = transport.get_bpm();
        Self {
            transport: Some(transport),
            loop_start_input: "0".to_string(),
            loop_end_input: "16".to_string(),
            bpm_input: format!("{:.1}", bpm),
            position_display: "1.1.1".to_string(),
        }
    }

    pub fn toggle_playback(&mut self, command_tx: &Sender<AudioCommand>) {
        if let Some(transport) = &self.transport {
            transport.toggle_playback();
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("tbp_tool_strip")
                .scroll_source(ScrollSource::ALL)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Transport buttons
                        if ui.button("⏮").on_hover_text("Go to Start").clicked() {
                            if let Some(transport) = &self.transport {
                                transport.rewind();
                            }
                        }

                        if ui.button("⏪").on_hover_text("Rewind").clicked() {
                            if let Some(transport) = &self.transport {
                                transport.rewind_beats(4.0);
                            }
                        }

                        let is_playing = self
                            .transport
                            .as_ref()
                            .map(|t| t.is_playing())
                            .unwrap_or(false);

                        let play_button = if is_playing { "⏸" } else { "▶" };
                        if ui.button(play_button).on_hover_text("Play/Pause").clicked() {
                            self.toggle_playback(&app.command_tx);
                        }

                        if ui.button("⏹").on_hover_text("Stop").clicked() {
                            if let Some(transport) = &self.transport {
                                transport.stop();
                            }
                        }

                        let is_recording = self
                            .transport
                            .as_ref()
                            .map(|t| t.is_recording())
                            .unwrap_or(false);

                        let record_color = if is_recording {
                            egui::Color32::RED
                        } else {
                            ui.style().visuals.text_color()
                        };

                        if ui
                            .add(egui::Button::new("⏺").fill(if is_recording {
                                egui::Color32::from_rgb(50, 0, 0)
                            } else {
                                egui::Color32::TRANSPARENT
                            }))
                            .on_hover_text("Record")
                            .clicked()
                        {
                            if let Some(transport) = &self.transport {
                                transport.record();
                            }
                        }

                        if ui.button("⏩").on_hover_text("Fast Forward").clicked() {
                            if let Some(transport) = &self.transport {
                                transport.fast_forward(4.0);
                            }
                        }

                        ui.separator();

                        // Position display
                        if let Some(transport) = &self.transport {
                            let position = transport.get_position();
                            let sample_rate = app.audio_state.sample_rate.load();
                            let bpm = transport.get_bpm();
                            let beats = (position / sample_rate as f64) * (bpm as f64 / 60.0);
                            let bar = (beats / 4.0) as u32 + 1;
                            let beat = (beats % 4.0) as u32 + 1;
                            let tick = ((beats % 1.0) * 480.0) as u32; // 480 ticks per beat

                            self.position_display = format!("{}.{}.{:03}", bar, beat, tick);
                        }

                        ui.label("Position:");
                        ui.label(&self.position_display);

                        ui.separator();

                        // BPM control
                        ui.label("BPM:");
                        ui.add(egui::TextEdit::singleline(&mut self.bpm_input).desired_width(60.0));
                        {
                            if let Ok(bpm) = self.bpm_input.parse::<f32>() {
                                if bpm >= 20.0 && bpm <= 999.0 {
                                    if let Some(transport) = &self.transport {
                                        transport.set_bpm(bpm);
                                    }
                                }
                            }
                        }

                        ui.separator();

                        // Loop controls
                        let loop_enabled = app.audio_state.loop_enabled.load(Ordering::Relaxed);
                        let mut loop_checkbox = loop_enabled;
                        if ui.checkbox(&mut loop_checkbox, "Loop").clicked() {
                            app.audio_state
                                .loop_enabled
                                .store(loop_checkbox, Ordering::Relaxed);
                            let _ = app
                                .command_tx
                                .send(AudioCommand::SetLoopEnabled(loop_checkbox));
                        }

                        if loop_enabled {
                            ui.label("Start:");
                            let loop_start = app.audio_state.loop_start.load();
                            self.loop_start_input = format!("{:.1}", loop_start);

                            ui.add(
                                egui::TextEdit::singleline(&mut self.loop_start_input)
                                    .desired_width(60.0),
                            );

                            {
                                if let Ok(start) = self.loop_start_input.parse::<f64>() {
                                    let end = app.audio_state.loop_end.load();
                                    let _ = app
                                        .command_tx
                                        .send(AudioCommand::SetLoopRegion(start, end));
                                }
                            }

                            ui.label("End:");
                            let loop_end = app.audio_state.loop_end.load();
                            self.loop_end_input = format!("{:.1}", loop_end);

                            ui.add(
                                egui::TextEdit::singleline(&mut self.loop_end_input)
                                    .desired_width(60.0),
                            );

                            {
                                if let Ok(end) = self.loop_end_input.parse::<f64>() {
                                    let start = app.audio_state.loop_start.load();
                                    let _ = app
                                        .command_tx
                                        .send(AudioCommand::SetLoopRegion(start, end));
                                }
                            }

                            if ui.button("Set to Selection").clicked() {
                                app.set_loop_to_selection();
                            }
                        }

                        ui.separator();

                        // Metronome
                        let mut metronome = self
                            .transport
                            .as_ref()
                            .map(|t| t.metronome_enabled)
                            .unwrap_or(false);

                        if ui.checkbox(&mut metronome, "Click").clicked() {
                            if let Some(transport) = &mut self.transport {
                                transport.metronome_enabled = metronome;
                            }
                        }
                    });
                });
        });
    }
}

impl Default for TransportUI {
    fn default() -> Self {
        Self {
            transport: None,
            loop_start_input: String::new(),
            loop_end_input: String::new(),
            bpm_input: "120.0".to_string(),
            position_display: "1.1.000".to_string(),
        }
    }
}
