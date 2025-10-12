use std::sync::atomic::Ordering;

use egui::scroll_area::ScrollSource;

use super::*;
use crate::messages::AudioCommand;
use crate::transport::Transport;

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
                        if ui.button("⏮").on_hover_text("Go to Start").clicked()
                            && let Some(transport) = &self.transport
                        {
                            transport.rewind();
                        }

                        if ui.button("⏪").on_hover_text("Rewind").clicked()
                            && let Some(transport) = &self.transport
                        {
                            transport.rewind_beats(4.0);
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

                        if ui.button("⏹").on_hover_text("Stop").clicked()
                            && let Some(transport) = &self.transport
                        {
                            transport.stop();
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

                        let is_recording = app.is_recording_ui;

                        let record_button = egui::Button::new("⏺").fill(if is_recording {
                            // Blinking effect for recording
                            let time = ctx.input(|i| i.time);
                            let alpha = (time.sin() * 0.5 + 0.5) as f32; // Varies between 0 and 1
                            egui::Color32::from_rgb(150 + (105.0 * alpha) as u8, 0, 0)
                        } else {
                            egui::Color32::TRANSPARENT
                        });

                        if ui.add(record_button).on_hover_text("Record").clicked() {
                            let should_record = !app.audio_state.recording.load(Ordering::Relaxed);
                            app.audio_state
                                .recording
                                .store(should_record, Ordering::Relaxed);

                            if should_record {
                                app.audio_state.playing.store(true, Ordering::Relaxed);
                            }
                        }

                        if ui.button("⏩").on_hover_text("Fast Forward").clicked()
                            && let Some(transport) = &self.transport
                        {
                            transport.fast_forward(4.0);
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

                        let current_bpm = self
                            .transport
                            .as_ref()
                            .map(|t| t.get_bpm())
                            .unwrap_or(120.0);

                        // Auto-update display if transport BPM changed externally
                        let displayed_bpm = self.bpm_input.parse::<f32>().unwrap_or(current_bpm);
                        if (current_bpm - displayed_bpm).abs() > 0.1 {
                            self.bpm_input = format!("{:.1}", current_bpm);
                        }

                        let bpm_response = ui.add(
                            egui::TextEdit::singleline(&mut self.bpm_input).desired_width(60.0),
                        );

                        // Validate and show feedback
                        let bpm_valid = if let Ok(bpm) = self.bpm_input.parse::<f32>() {
                            (20.0..=999.0).contains(&bpm)
                        } else {
                            false
                        };

                        // Visual feedback for invalid input
                        if !bpm_valid && !self.bpm_input.is_empty() {
                            let bpm_response = bpm_response.clone();
                            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                            bpm_response.on_hover_text("BPM must be between 20 and 999");
                        }

                        // Apply on Enter or focus lost
                        if (bpm_response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                            || (bpm_response.lost_focus() && bpm_valid)
                        {
                            if let Ok(bpm) = self.bpm_input.parse::<f32>() {
                                if (20.0..=999.0).contains(&bpm) {
                                    if let Some(transport) = &self.transport {
                                        transport.set_bpm(bpm);
                                    }
                                    self.bpm_input = format!("{:.1}", bpm);
                                }
                            }
                        }

                        ui.separator();

                        // Loop controls with similar validation
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

                            let loop_start_response = ui.add(
                                egui::TextEdit::singleline(&mut self.loop_start_input)
                                    .desired_width(60.0),
                            );

                            if loop_start_response.lost_focus() {
                                if let Ok(start) = self.loop_start_input.parse::<f64>() {
                                    let end = app.audio_state.loop_end.load();
                                    if start < end && start >= 0.0 {
                                        app.audio_state.loop_start.store(start);
                                        let _ = app
                                            .command_tx
                                            .send(AudioCommand::SetLoopRegion(start, end));
                                        self.loop_start_input = format!("{:.1}", start);
                                    } else {
                                        // Reset to current value on invalid input
                                        self.loop_start_input =
                                            format!("{:.1}", app.audio_state.loop_start.load());
                                    }
                                }
                            }

                            ui.label("End:");

                            let loop_end_response = ui.add(
                                egui::TextEdit::singleline(&mut self.loop_end_input)
                                    .desired_width(60.0),
                            );

                            if loop_end_response.lost_focus() {
                                if let Ok(end) = self.loop_end_input.parse::<f64>() {
                                    let start = app.audio_state.loop_start.load();
                                    if end > start {
                                        app.audio_state.loop_end.store(end);
                                        let _ = app
                                            .command_tx
                                            .send(AudioCommand::SetLoopRegion(start, end));
                                        self.loop_end_input = format!("{:.1}", end);
                                    } else {
                                        // Reset to current value on invalid input
                                        self.loop_end_input =
                                            format!("{:.1}", app.audio_state.loop_end.load());
                                    }
                                }
                            }

                            if ui.button("Set to Selection").clicked() {
                                app.set_loop_to_selection();
                                // Update display
                                self.loop_start_input =
                                    format!("{:.1}", app.audio_state.loop_start.load());
                                self.loop_end_input =
                                    format!("{:.1}", app.audio_state.loop_end.load());
                            }
                        }

                        ui.separator();

                        // Metronome
                        let mut metronome = self
                            .transport
                            .as_ref()
                            .map(|t| t.metronome_enabled)
                            .unwrap_or(false);

                        if ui.checkbox(&mut metronome, "Click").clicked()
                            && let Some(transport) = &mut self.transport
                        {
                            transport.metronome_enabled = metronome;
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
