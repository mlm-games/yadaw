use super::*;
use crate::state::AudioCommand;
use crate::transport::{LoopMode, Transport, TransportState};

pub struct TransportUI {
    pub transport: Option<Transport>,
}

impl TransportUI {
    pub fn new(transport: Transport) -> Self {
        Self {
            transport: Some(transport),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        if let Some(_transport) = &mut self.transport {
            egui::TopBottomPanel::top("transport_panel").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.draw_transport_controls(ui, app);
                    ui.separator();
                    self.draw_time_display(ui, app);
                    ui.separator();
                    self.draw_tempo_control(ui);
                    ui.separator();
                    self.draw_master_volume(ui, app);
                });
            });
        }
    }

    fn draw_transport_controls(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        if let Some(transport) = &mut self.transport {
            // Play/Pause button
            let is_playing = transport.state == TransportState::Playing;
            if ui
                .button(if is_playing { "⏸" } else { "▶" })
                .on_hover_text("Play/Pause")
                .clicked()
            {
                self.toggle_playback(&app.command_tx);
            }

            // Stop button
            if ui.button("⏹").on_hover_text("Stop").clicked() {
                self.stop(&app.command_tx);
            }

            // Record button
            let is_recording = transport.state == TransportState::Recording;
            if ui
                .button(if is_recording { "⏺ Recording" } else { "⏺" })
                .on_hover_text("Record")
                .clicked()
            {
                self.toggle_recording(app);
            }

            ui.separator();

            // Loop button
            let loop_active = transport.loop_mode != LoopMode::Off;
            if ui
                .selectable_label(loop_active, "🔁")
                .on_hover_text("Loop")
                .clicked()
            {
                transport.toggle_loop();
            }

            // Metronome button
            if ui
                .selectable_label(transport.metronome_enabled, "🎵")
                .on_hover_text("Metronome")
                .clicked()
            {
                transport.toggle_metronome();
            }
        }
    }

    fn draw_time_display(&self, ui: &mut egui::Ui, app: &super::app::YadawApp) {
        if let Some(transport) = &self.transport {
            let position_beats = transport.get_position_beats();
            ui.label(transport.format_time(position_beats));

            ui.separator();

            let position_samples = app.audio_state.get_position();
            ui.label(transport.format_time_seconds(position_samples));
        }
    }

    fn draw_tempo_control(&mut self, ui: &mut egui::Ui) {
        if let Some(transport) = &mut self.transport {
            ui.label("BPM:");
            let mut bpm = transport.get_bpm();
            if ui
                .add(
                    egui::DragValue::new(&mut bpm)
                        .speed(0.5)
                        .range(20.0..=999.0),
                )
                .changed()
            {
                transport.set_bpm(bpm);
            }
        }
    }

    fn draw_master_volume(&self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.label("Master:");
        let mut master_vol = app.audio_state.master_volume.load();
        if ui
            .add(egui::Slider::new(&mut master_vol, 0.0..=1.0).show_value(false))
            .changed()
        {
            app.audio_state.master_volume.store(master_vol);
            if let Ok(mut state) = app.state.lock() {
                state.master_volume = master_vol;
            }
        }
    }

    pub fn toggle_playback(&mut self, _command_tx: &Sender<AudioCommand>) {
        if let Some(transport) = &mut self.transport {
            transport.toggle_playback();
        }
    }

    fn stop(&mut self, _command_tx: &Sender<AudioCommand>) {
        if let Some(transport) = &mut self.transport {
            transport.stop();
        }
    }

    fn toggle_recording(&mut self, app: &mut super::app::YadawApp) {
        if let Some(transport) = &mut self.transport {
            if transport.state == TransportState::Recording {
                transport.stop_recording();
            } else {
                let state = app.state.lock().unwrap();
                let armed_track = state.tracks.iter().position(|t| t.armed && !t.is_midi);
                drop(state);

                if let Some(track_id) = armed_track {
                    transport.start_recording(track_id);
                } else {
                    app.dialogs
                        .show_message("Please arm an audio track for recording");
                }
            }
        }
    }
}

impl Default for TransportUI {
    fn default() -> Self {
        Self { transport: None }
    }
}
