use crate::state::{MidiNote, Pattern};
use eframe::{egui, egui_glow::painter};

pub struct PianoRoll {
    pub zoom_x: f32, // Pixels per beat
    pub zoom_y: f32, // Pixels per semitone
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub selected_notes: Vec<usize>,
    pub grid_snap: f32, // Snap to grid in beats (0.25 = 16th notes)
}

impl Default for PianoRoll {
    fn default() -> Self {
        Self {
            zoom_x: 100.0,
            zoom_y: 20.0,
            scroll_x: 0.0,
            scroll_y: 60.0 * 20.0, // Start at middle C
            grid_snap: 0.25,
            selected_notes: Vec::new(),
        }
    }
}

impl PianoRoll {
    pub fn ui(&mut self, ui: &mut egui::Ui, pattern: &mut Pattern) -> Vec<PianoRollAction> {
        let mut actions = Vec::new();

        let available_rect = ui.available_rect_before_wrap();

        // Background
        ui.painter()
            .rect_filled(available_rect, 0.0, egui::Color32::from_gray(20));

        // Draw piano keys on the left
        let piano_width = 60.0;
        let piano_rect = egui::Rect::from_min_size(
            available_rect.min,
            egui::vec2(piano_width, available_rect.height()),
        );

        self.draw_piano_keys(&ui.painter(), piano_rect);

        // Draw grid
        let grid_rect = egui::Rect::from_min_size(
            available_rect.min + egui::vec2(piano_width, 0.0),
            egui::vec2(
                available_rect.width() - piano_width,
                available_rect.height(),
            ),
        );

        self.draw_grid(&ui.painter(), grid_rect, pattern.length);

        // Handle input BEFORE drawing notes so we can update selection
        let response = ui.interact(
            grid_rect,
            ui.id().with("piano_roll_grid"),
            egui::Sense::click_and_drag(),
        );

        let mut clicked_on_note = false;
        let mut note_to_remove = None;

        // Check for clicks on existing notes
        if let Some(pos) = response.interact_pointer_pos() {
            for (i, note) in pattern.notes.iter().enumerate() {
                let note_rect = self.note_rect(note, grid_rect);
                if note_rect.contains(pos) {
                    if response.clicked() {
                        // Left click - select note
                        clicked_on_note = true;
                        if !ui.input(|i| i.modifiers.ctrl) {
                            self.selected_notes.clear();
                        }
                        if !self.selected_notes.contains(&i) {
                            self.selected_notes.push(i);
                        }
                    } else if response.secondary_clicked() {
                        // Right click - remove note
                        note_to_remove = Some(i);
                    }
                    break;
                }
            }
        }

        // If we didn't click on a note, handle empty space click
        if response.clicked() && !clicked_on_note {
            if let Some(pos) = response.interact_pointer_pos() {
                // Clear selection unless ctrl is held
                if !ui.input(|i| i.modifiers.ctrl) {
                    self.selected_notes.clear();
                }

                // Add new note
                let grid_pos = pos - grid_rect.min;
                let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                let screen_y_from_top = grid_pos.y;
                let scrolled_y = screen_y_from_top + self.scroll_y;
                let pitch_from_top = scrolled_y / self.zoom_y;
                let pitch = (127.0 - pitch_from_top).round().clamp(0.0, 127.0) as u8;

                let snapped_beat: f64 = ((beat / self.grid_snap).round() * self.grid_snap).into();

                if snapped_beat >= 0.0 && snapped_beat < pattern.length {
                    actions.push(PianoRollAction::AddNote(MidiNote {
                        pitch,
                        velocity: 100,
                        start: snapped_beat,
                        duration: self.grid_snap.into(),
                    }));
                }
            }
        }

        // Handle note removal
        if let Some(idx) = note_to_remove {
            actions.push(PianoRollAction::RemoveNote(idx));
        }

        // Draw notes
        for (i, note) in pattern.notes.iter().enumerate() {
            let note_rect = self.note_rect(note, grid_rect);
            let color = if self.selected_notes.contains(&i) {
                egui::Color32::from_rgb(100, 150, 255)
            } else {
                egui::Color32::from_rgb(80, 120, 200)
            };

            ui.painter().rect_filled(note_rect, 2.0, color);
            ui.painter().rect_stroke(
                note_rect,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::BLACK),
                egui::StrokeKind::Inside,
            );
        }

        // Draw playhead if playing
        if let Some(current_beat) = ui
            .ctx()
            .memory(|mem| mem.data.get_temp::<f64>(egui::Id::new("current_beat")))
        {
            let playhead_x = grid_rect.min.x + (current_beat as f32 * self.zoom_x - self.scroll_x);
            if playhead_x >= grid_rect.min.x && playhead_x <= grid_rect.max.x {
                ui.painter().line_segment(
                    [
                        egui::pos2(playhead_x, grid_rect.min.y),
                        egui::pos2(playhead_x, grid_rect.max.y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 100, 100)),
                );
            }
        }

        // Handle keyboard shortcuts
        ui.input(|i| {
            if i.key_pressed(egui::Key::Delete) && !self.selected_notes.is_empty() {
                // Remove selected notes in reverse order
                let mut indices = self.selected_notes.clone();
                indices.sort_by(|a, b| b.cmp(a));
                for idx in indices {
                    if idx < pattern.notes.len() {
                        actions.push(PianoRollAction::RemoveNote(idx));
                    }
                }
                self.selected_notes.clear();
            }
        });

        // Zoom with scroll
        if response.hovered() {
            let scroll_delta = ui.input(|i| i.raw_scroll_delta);
            if ui.input(|i| i.modifiers.ctrl) {
                // Zoom
                self.zoom_x *= 1.0 + scroll_delta.y * 0.01;
                self.zoom_x = self.zoom_x.clamp(10.0, 500.0);
            } else {
                // Scroll
                self.scroll_x -= scroll_delta.x;
                self.scroll_y -= scroll_delta.y;
                self.scroll_x = self.scroll_x.max(0.0);
                self.scroll_y = self
                    .scroll_y
                    .clamp(0.0, 127.0 * self.zoom_y - available_rect.height());
            }
        }

        actions
    }

    fn draw_piano_keys(&self, painter: &egui::Painter, rect: egui::Rect) {
        let white_keys = [0, 2, 4, 5, 7, 9, 11];
        let black_keys = [1, 3, 6, 8, 10];

        for octave in 0..11 {
            for &key in &white_keys {
                let pitch = octave * 12 + key;
                let y = self.pitch_to_y(pitch as f32, rect);

                if y >= rect.min.y - self.zoom_y && y <= rect.max.y + self.zoom_y {
                    let key_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x, y - self.zoom_y / 2.0),
                        egui::vec2(rect.width(), self.zoom_y),
                    );

                    painter.rect_filled(key_rect, 0.0, egui::Color32::WHITE);
                    painter.rect_stroke(
                        key_rect,
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::GRAY),
                        egui::StrokeKind::Outside,
                    );

                    // Label C notes
                    if key == 0 {
                        painter.text(
                            key_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("C{}", octave - 1),
                            egui::FontId::default(),
                            egui::Color32::BLACK,
                        );
                    }
                }
            }
        }

        // Draw black keys on top
        for octave in 0..11 {
            for &key in &black_keys {
                let pitch = octave * 12 + key;
                let y = self.pitch_to_y(pitch as f32, rect);

                if y >= rect.min.y - self.zoom_y && y <= rect.max.y + self.zoom_y {
                    let key_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x, y - self.zoom_y / 2.0),
                        egui::vec2(rect.width() * 0.7, self.zoom_y),
                    );

                    painter.rect_filled(key_rect, 0.0, egui::Color32::from_gray(40));
                    painter.rect_stroke(
                        key_rect,
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::BLACK),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: egui::Rect, pattern_length: f64) {
        // Vertical lines (beats)
        let visible_beats = (rect.width() / self.zoom_x) as i32 + 2;
        let start_beat = (self.scroll_x / self.zoom_x) as i32;

        for i in 0..visible_beats {
            let beat = start_beat + i;
            let x = rect.min.x + (beat as f32 * self.zoom_x - self.scroll_x);

            if x >= rect.min.x && x <= rect.max.x {
                let color = if beat % 4 == 0 {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(40)
                };

                painter.line_segment(
                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(1.0, color),
                );
            }
        }

        // Horizontal lines (notes)
        for pitch in 0..128 {
            let y = self.pitch_to_y(pitch as f32, rect);

            if y >= rect.min.y && y <= rect.max.y {
                let color = if pitch % 12 == 0 {
                    egui::Color32::from_gray(50)
                } else {
                    egui::Color32::from_gray(30)
                };

                painter.line_segment(
                    [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                    egui::Stroke::new(1.0, color),
                );
            }
        }
    }

    fn note_rect(&self, note: &MidiNote, grid_rect: egui::Rect) -> egui::Rect {
        let x = grid_rect.min.x + (note.start as f32 * self.zoom_x - self.scroll_x);
        let y = self.pitch_to_y(note.pitch as f32 + 0.5, grid_rect);
        let width = note.duration as f32 * self.zoom_x;
        let height = self.zoom_y * 0.8;

        egui::Rect::from_min_size(egui::pos2(x, y - height / 2.0), egui::vec2(width, height))
    }

    // Helper function to convert pitch to screen Y coordinate
    fn pitch_to_y(&self, pitch: f32, rect: egui::Rect) -> f32 {
        rect.min.y + (127.0 - pitch) * self.zoom_y - self.scroll_y
    }
}

#[derive(Debug)]
pub enum PianoRollAction {
    AddNote(MidiNote),
    RemoveNote(usize),
    UpdateNote(usize, MidiNote),
}
