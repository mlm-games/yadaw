use super::*;
use crate::piano_roll::{PianoRoll, PianoRollAction};
use crate::state::{AudioCommand, MidiNote};

pub struct PianoRollView {
    piano_roll: PianoRoll,
    selected_pattern: usize,

    // View settings
    show_velocity_lane: bool,
    show_controller_lanes: bool,
    velocity_lane_height: f32,

    // Tool modes
    tool_mode: ToolMode,

    // MIDI input
    midi_input_enabled: bool,
    midi_octave_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ToolMode {
    Select,
    Draw,
    Erase,
    Split,
    Glue,
    Velocity,
}

impl PianoRollView {
    pub fn new() -> Self {
        Self {
            piano_roll: PianoRoll::default(),
            selected_pattern: 0,

            show_velocity_lane: true,
            show_controller_lanes: false,
            velocity_lane_height: 100.0,

            tool_mode: ToolMode::Select,

            midi_input_enabled: false,
            midi_octave_offset: 0,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.vertical(|ui| {
            // Piano roll header
            self.draw_header(ui, app);

            ui.separator();

            // Main piano roll area
            let available_height = ui.available_height();
            let piano_roll_height = if self.show_velocity_lane {
                available_height - self.velocity_lane_height - 5.0
            } else {
                available_height
            };

            ui.allocate_ui(egui::vec2(ui.available_width(), piano_roll_height), |ui| {
                self.draw_piano_roll(ui, app);
            });

            // Velocity lane
            if self.show_velocity_lane {
                ui.separator();
                ui.allocate_ui(
                    egui::vec2(ui.available_width(), self.velocity_lane_height),
                    |ui| {
                        self.draw_velocity_lane(ui, app);
                    },
                );
            }

            // Controller lanes
            if self.show_controller_lanes {
                ui.separator();
                self.draw_controller_lanes(ui, app);
            }
        });
    }

    fn draw_header(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.horizontal(|ui| {
            ui.heading("Piano Roll");

            ui.separator();

            // Pattern selector
            ui.label("Pattern:");
            let state = app.state.lock().unwrap();
            if let Some(track) = state.tracks.get(app.selected_track) {
                egui::ComboBox::from_id_source("pattern_selector")
                    .selected_text(
                        track
                            .patterns
                            .get(self.selected_pattern)
                            .map(|p| p.name.as_str())
                            .unwrap_or("No Pattern"),
                    )
                    .show_ui(ui, |ui| {
                        for (i, pattern) in track.patterns.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.selected_pattern, i, &pattern.name)
                                .clicked()
                            {
                                // Pattern changed
                            }
                        }
                    });

                // Add pattern button
                if ui.button("➕").on_hover_text("Add Pattern").clicked() {
                    // Add new pattern
                }

                // Duplicate pattern button
                if ui.button("⎘").on_hover_text("Duplicate Pattern").clicked() {
                    // Duplicate current pattern
                }
            }
            drop(state);

            ui.separator();

            // Tool selection
            ui.label("Tool:");
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.tool_mode == ToolMode::Select, "↖")
                    .on_hover_text("Select Tool")
                    .clicked()
                {
                    self.tool_mode = ToolMode::Select;
                }

                if ui
                    .selectable_label(self.tool_mode == ToolMode::Draw, "✏")
                    .on_hover_text("Draw Tool")
                    .clicked()
                {
                    self.tool_mode = ToolMode::Draw;
                }

                if ui
                    .selectable_label(self.tool_mode == ToolMode::Erase, "⌫")
                    .on_hover_text("Erase Tool")
                    .clicked()
                {
                    self.tool_mode = ToolMode::Erase;
                }

                if ui
                    .selectable_label(self.tool_mode == ToolMode::Split, "✂")
                    .on_hover_text("Split Tool")
                    .clicked()
                {
                    self.tool_mode = ToolMode::Split;
                }

                if ui
                    .selectable_label(self.tool_mode == ToolMode::Glue, "⊕")
                    .on_hover_text("Glue Tool")
                    .clicked()
                {
                    self.tool_mode = ToolMode::Glue;
                }

                if ui
                    .selectable_label(self.tool_mode == ToolMode::Velocity, "⇅")
                    .on_hover_text("Velocity Tool")
                    .clicked()
                {
                    self.tool_mode = ToolMode::Velocity;
                }
            });

            ui.separator();

            // Snap settings
            ui.label("Snap:");
            egui::ComboBox::from_id_source("piano_roll_snap")
                .selected_text(format!("1/{}", (1.0 / self.piano_roll.grid_snap) as i32))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 1.0, "1/1");
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 0.5, "1/2");
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 0.25, "1/4");
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 0.125, "1/8");
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 0.0625, "1/16");
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 0.03125, "1/32");
                    ui.selectable_value(&mut self.piano_roll.grid_snap, 0.0, "Off");
                });

            ui.separator();

            // View options
            ui.checkbox(&mut self.show_velocity_lane, "Velocity")
                .on_hover_text("Show/Hide Velocity Lane");

            ui.checkbox(&mut self.show_controller_lanes, "Controllers")
                .on_hover_text("Show/Hide Controller Lanes");

            ui.separator();

            // MIDI input
            ui.checkbox(&mut self.midi_input_enabled, "MIDI In")
                .on_hover_text("Enable MIDI Input");

            if self.midi_input_enabled {
                ui.label("Octave:");
                ui.add(
                    egui::DragValue::new(&mut self.midi_octave_offset)
                        .speed(1)
                        .range(-4..=4),
                );
            }

            ui.separator();

            // Zoom controls
            ui.label("Zoom:");
            if ui.button("−").clicked() {
                self.piano_roll.zoom_x = (self.piano_roll.zoom_x * 0.8).max(10.0);
                self.piano_roll.zoom_y = (self.piano_roll.zoom_y * 0.9).max(10.0);
            }
            if ui.button("╋").clicked() {
                self.piano_roll.zoom_x = (self.piano_roll.zoom_x * 1.25).min(500.0);
                self.piano_roll.zoom_y = (self.piano_roll.zoom_y * 1.1).min(50.0);
            }
            if ui.button("Reset").clicked() {
                self.piano_roll.zoom_x = 100.0;
                self.piano_roll.zoom_y = 20.0;
            }
        });
    }

    fn draw_piano_roll(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let mut pattern_actions = Vec::new();

        // Update current beat position for playhead
        {
            let state = app.state.lock().unwrap();
            if state.playing {
                let current_beat = state.position_to_beats(app.audio_state.get_position());
                let pattern_beat = current_beat
                    % state
                        .tracks
                        .get(app.selected_track)
                        .and_then(|t| t.patterns.get(self.selected_pattern))
                        .map(|p| p.length)
                        .unwrap_or(4.0);
                ui.ctx().memory_mut(|mem| {
                    mem.data
                        .insert_temp(egui::Id::new("current_beat"), pattern_beat);
                });
            }
        }

        // Draw the piano roll
        {
            let mut state = app.state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(app.selected_track) {
                if let Some(pattern) = track.patterns.get_mut(self.selected_pattern) {
                    // Apply tool mode to piano roll
                    match self.tool_mode {
                        ToolMode::Select => {
                            // Default behavior
                        }
                        ToolMode::Draw => {
                            // Enable drawing mode
                        }
                        ToolMode::Erase => {
                            // Enable erase mode
                        }
                        _ => {}
                    }

                    pattern_actions = self.piano_roll.ui(ui, pattern);
                }
            }
        }

        // Process piano roll actions
        for action in pattern_actions {
            match action {
                PianoRollAction::AddNote(note) => {
                    let _ = app.command_tx.send(AudioCommand::AddNote(
                        app.selected_track,
                        self.selected_pattern,
                        note,
                    ));
                }
                PianoRollAction::RemoveNote(index) => {
                    let _ = app.command_tx.send(AudioCommand::RemoveNote(
                        app.selected_track,
                        self.selected_pattern,
                        index,
                    ));
                }
                PianoRollAction::UpdateNote(index, note) => {
                    let _ = app.command_tx.send(AudioCommand::UpdateNote(
                        app.selected_track,
                        self.selected_pattern,
                        index,
                        note,
                    ));
                }
                PianoRollAction::PreviewNote(pitch) => {
                    let _ = app
                        .command_tx
                        .send(AudioCommand::PreviewNote(app.selected_track, pitch));
                }
                PianoRollAction::StopPreview => {
                    let _ = app.command_tx.send(AudioCommand::StopPreviewNote);
                }
            }
        }
    }

    fn draw_velocity_lane(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let state = app.state.lock().unwrap();

        if let Some(track) = state.tracks.get(app.selected_track) {
            if let Some(pattern) = track.patterns.get(self.selected_pattern) {
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(ui.available_width(), self.velocity_lane_height),
                    egui::Sense::click_and_drag(),
                );

                let rect = response.rect;

                // Background
                painter.rect_filled(rect, 0.0, egui::Color32::from_gray(15));

                // Grid lines
                for i in 0..=4 {
                    let y = rect.top() + (i as f32 / 4.0) * rect.height();
                    painter.line_segment(
                        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                    );
                }

                // Draw velocity bars for each note
                for (i, note) in pattern.notes.iter().enumerate() {
                    let x = rect.left()
                        + (note.start as f32 * self.piano_roll.zoom_x - self.piano_roll.scroll_x);
                    let width = (note.duration as f32 * self.piano_roll.zoom_x).max(2.0);
                    let height = (note.velocity as f32 / 127.0) * rect.height();

                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - height),
                        egui::vec2(width, height),
                    );

                    let is_selected = self.piano_roll.selected_notes.contains(&i);
                    let color = if is_selected {
                        egui::Color32::from_rgb(100, 150, 255)
                    } else {
                        egui::Color32::from_rgb(60, 90, 150)
                    };

                    painter.rect_filled(bar_rect, 0.0, color);

                    // Handle velocity editing
                    if bar_rect.contains(response.hover_pos().unwrap_or_default()) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);

                        if response.dragged() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                let new_velocity = ((rect.bottom() - pos.y) / rect.height() * 127.0)
                                    .round()
                                    .clamp(0.0, 127.0)
                                    as u8;

                                if new_velocity != note.velocity {
                                    let mut new_note = *note;
                                    new_note.velocity = new_velocity;

                                    let _ = app.command_tx.send(AudioCommand::UpdateNote(
                                        app.selected_track,
                                        self.selected_pattern,
                                        i,
                                        new_note,
                                    ));
                                }
                            }
                        }
                    }
                }

                // Draw velocity value on hover
                if let Some(pos) = response.hover_pos() {
                    let velocity = ((rect.bottom() - pos.y) / rect.height() * 127.0)
                        .round()
                        .clamp(0.0, 127.0) as u8;

                    painter.text(
                        pos + egui::vec2(10.0, -10.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("Vel: {}", velocity),
                        egui::FontId::default(),
                        egui::Color32::WHITE,
                    );
                }
            }
        }
    }

    fn draw_controller_lanes(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.group(|ui| {
            ui.set_min_height(100.0);

            ui.horizontal(|ui| {
                ui.label("Controller:");

                egui::ComboBox::from_id_source("controller_select")
                    .selected_text("Modulation")
                    .show_ui(ui, |ui| {
                        ui.selectable_label(false, "Modulation (CC1)");
                        ui.selectable_label(false, "Expression (CC11)");
                        ui.selectable_label(false, "Pan (CC10)");
                        ui.selectable_label(false, "Volume (CC7)");
                        ui.separator();
                        ui.selectable_label(false, "Pitch Bend");
                        ui.selectable_label(false, "Aftertouch");
                    });

                if ui
                    .button("➕")
                    .on_hover_text("Add Controller Lane")
                    .clicked()
                {
                    // Add new controller lane
                }
            });

            ui.separator();

            // Draw controller data
            let (response, painter) = ui.allocate_painter(
                egui::vec2(ui.available_width(), 80.0),
                egui::Sense::click_and_drag(),
            );

            let rect = response.rect;

            // Background
            painter.rect_filled(rect, 0.0, egui::Color32::from_gray(20));

            // Grid
            for i in 0..=4 {
                let y = rect.top() + (i as f32 / 4.0) * rect.height();
                painter.line_segment(
                    [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(35)),
                );
            }

            // Placeholder text
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Controller data will be displayed here",
                egui::FontId::default(),
                egui::Color32::from_gray(100),
            );
        });
    }
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self::new()
    }
}
