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

        // Make the piano roll take up the full available space
        ui.allocate_rect(available_rect, egui::Sense::click_and_drag());

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

        // Handle input - FIXED: Create response for the entire area
        let response = ui.interact(
            available_rect,
            ui.id().with("piano_roll"),
            egui::Sense::click_and_drag(),
        );

        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if pos.x > available_rect.min.x + piano_width {
                    // Click in grid area - add note
                    let grid_pos = pos - grid_rect.min;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let pitch = 127.0 - ((grid_pos.y + self.scroll_y) / self.zoom_y);
                    let pitch = pitch.clamp(0.0, 127.0) as u8;

                    // Snap to grid
                    let snapped_beat = (beat / self.grid_snap).round() * self.grid_snap;

                    // Only add if within pattern bounds
                    if snapped_beat >= 0.0 && (snapped_beat as f64) < pattern.length {
                        actions.push(PianoRollAction::AddNote(MidiNote {
                            pitch,
                            velocity: 100,
                            start: snapped_beat.into(),
                            duration: self.grid_snap.into(),
                        }));
                    }
                }
            }
        }

        // Right click to remove notes
        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if pos.x > available_rect.min.x + piano_width {
                    // Check if click is on a note
                    for (i, note) in pattern.notes.iter().enumerate() {
                        let note_rect = self.note_rect(note, grid_rect);
                        if note_rect.contains(pos) {
                            actions.push(PianoRollAction::RemoveNote(i));
                            break;
                        }
                    }
                }
            }
        }

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
                self.scroll_y = self.scroll_y.clamp(0.0, 127.0 * self.zoom_y);
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
                let y =
                    rect.max.y - ((pitch as f32 + 0.5 - self.scroll_y / self.zoom_y) * self.zoom_y);

                if y >= rect.min.y && y <= rect.max.y {
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
                let y =
                    rect.max.y - ((pitch as f32 + 0.5 - self.scroll_y / self.zoom_y) * self.zoom_y);

                if y >= rect.min.y && y <= rect.max.y {
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
            let y = rect.max.y - ((pitch as f32 + 0.5 - self.scroll_y / self.zoom_y) * self.zoom_y);

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
        let y = grid_rect.max.y
            - ((note.pitch as f32 + 1.0 - self.scroll_y / self.zoom_y) * self.zoom_y);
        let width = note.duration as f32 * self.zoom_x;
        let height = self.zoom_y * 0.8;

        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
    }
}

#[derive(Debug)]
pub enum PianoRollAction {
    AddNote(MidiNote),
    RemoveNote(usize),
    UpdateNote(usize, MidiNote),
}
