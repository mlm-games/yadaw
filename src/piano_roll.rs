use crate::state::{MidiNote, Pattern};
use eframe::{egui, egui_glow::painter};

pub struct PianoRoll {
    pub zoom_x: f32, // Pixels per beat
    pub zoom_y: f32, // Pixels per semitone
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub selected_notes: Vec<usize>,
    pub grid_snap: f32, // Snap to grid in beats (0.25 = 16th notes)
    dragging_note: Option<DragState>,
    resizing_note: Option<ResizeState>,
    hover_note: Option<usize>,
}

#[derive(Clone)]
struct DragState {
    note_index: usize,
    start_offset: egui::Vec2, // Offset from note start to mouse
    original_note: MidiNote,
}

#[derive(Clone)]
struct ResizeState {
    note_index: usize,
    edge: ResizeEdge,
    original_note: MidiNote,
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
            dragging_note: None,
            resizing_note: None,
            hover_note: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ResizeEdge {
    Left,
    Right,
}

impl PianoRoll {
    pub fn ui(&mut self, ui: &mut egui::Ui, pattern: &mut Pattern) -> Vec<PianoRollAction> {
        let mut actions = Vec::new();
        let available_rect = ui.available_rect_before_wrap();

        // Background
        ui.painter()
            .rect_filled(available_rect, 0.0, egui::Color32::from_gray(20));

        // Draw piano keys and grid
        let piano_width = 60.0;
        let piano_rect = egui::Rect::from_min_size(
            available_rect.min,
            egui::vec2(piano_width, available_rect.height()),
        );
        self.draw_piano_keys(&ui.painter(), piano_rect);

        let grid_rect = egui::Rect::from_min_size(
            available_rect.min + egui::vec2(piano_width, 0.0),
            egui::vec2(
                available_rect.width() - piano_width,
                available_rect.height(),
            ),
        );
        self.draw_grid(&ui.painter(), grid_rect, pattern.length);

        // Handle input - use drag sense for better drag detection
        let response = ui.interact(
            grid_rect,
            ui.id().with("piano_roll_grid"),
            egui::Sense::click_and_drag(),
        );

        let mut clicked_on_note = false;
        let mut note_to_remove = None;

        // Update hover state and cursor
        self.hover_note = None;
        if let Some(pos) = response.hover_pos() {
            for (i, note) in pattern.notes.iter().enumerate() {
                let note_rect = self.note_rect(note, grid_rect);
                if note_rect.contains(pos) {
                    self.hover_note = Some(i);

                    // Check if near edges for resize cursor
                    let edge_threshold = 5.0;
                    if (pos.x - note_rect.left()).abs() < edge_threshold {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    } else if (note_rect.right() - pos.x).abs() < edge_threshold {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    break;
                }
            }
        }

        // Handle mouse button down - start drag/resize
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                // Check if we're starting a drag or resize
                for (i, note) in pattern.notes.iter().enumerate() {
                    let note_rect = self.note_rect(note, grid_rect);
                    if note_rect.contains(pos) {
                        clicked_on_note = true;

                        let edge_threshold = 5.0;
                        if (pos.x - note_rect.left()).abs() < edge_threshold {
                            // Start resizing from left edge
                            self.resizing_note = Some(ResizeState {
                                note_index: i,
                                edge: ResizeEdge::Left,
                                original_note: *note,
                            });
                        } else if (note_rect.right() - pos.x).abs() < edge_threshold {
                            // Start resizing from right edge
                            self.resizing_note = Some(ResizeState {
                                note_index: i,
                                edge: ResizeEdge::Right,
                                original_note: *note,
                            });
                        } else {
                            // Start dragging
                            self.dragging_note = Some(DragState {
                                note_index: i,
                                start_offset: pos - note_rect.left_top(),
                                original_note: *note,
                            });

                            // Select the note if not already selected
                            if !self.selected_notes.contains(&i) {
                                if !ui.input(|i| i.modifiers.ctrl) {
                                    self.selected_notes.clear();
                                }
                                self.selected_notes.push(i);
                            }
                        }
                        break;
                    }
                }
            }
        }

        // Handle ongoing drag - don't modify pattern yet
        let mut preview_note = None;
        if response.dragged() {
            if let Some(drag_state) = &self.dragging_note {
                if let Some(pos) = response.hover_pos() {
                    let grid_pos = pos - grid_rect.min - drag_state.start_offset;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap).max(0.0);

                    let screen_y_from_top = grid_pos.y + drag_state.start_offset.y;
                    let scrolled_y = screen_y_from_top + self.scroll_y;
                    let pitch_from_top = scrolled_y / self.zoom_y;
                    let pitch = (127.0 - pitch_from_top).round().clamp(0.0, 127.0) as u8;

                    let mut new_note = drag_state.original_note;
                    new_note.start = snapped_beat as f64;
                    new_note.pitch = pitch;
                    preview_note = Some((drag_state.note_index, new_note));
                }
            }

            // Handle ongoing resize
            if let Some(resize_state) = &self.resizing_note {
                if let Some(pos) = response.hover_pos() {
                    let grid_pos = pos - grid_rect.min;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap).max(0.0);

                    let mut new_note = resize_state.original_note;

                    match resize_state.edge {
                        ResizeEdge::Left => {
                            let old_end = new_note.start + new_note.duration;
                            new_note.start =
                                snapped_beat.min(old_end as f32 - self.grid_snap) as f64;
                            new_note.duration = old_end - new_note.start;
                        }
                        ResizeEdge::Right => {
                            let new_end =
                                snapped_beat.max(new_note.start as f32 + self.grid_snap) as f64;
                            new_note.duration = new_end - new_note.start;
                        }
                    }

                    preview_note = Some((resize_state.note_index, new_note));
                }
            }
        }

        // When drag is released, apply the change
        if response.drag_stopped() {
            if let Some(drag_state) = &self.dragging_note {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let grid_pos = pos - grid_rect.min - drag_state.start_offset;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap).max(0.0);

                    let screen_y_from_top = grid_pos.y + drag_state.start_offset.y;
                    let scrolled_y = screen_y_from_top + self.scroll_y;
                    let pitch_from_top = scrolled_y / self.zoom_y;
                    let pitch = (127.0 - pitch_from_top).round().clamp(0.0, 127.0) as u8;

                    if drag_state.note_index < pattern.notes.len() {
                        let mut new_note = pattern.notes[drag_state.note_index];
                        new_note.start = snapped_beat as f64;
                        new_note.pitch = pitch;
                        actions.push(PianoRollAction::UpdateNote(drag_state.note_index, new_note));
                    }
                }
            }

            if let Some(resize_state) = &self.resizing_note {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let grid_pos = pos - grid_rect.min;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap).max(0.0);

                    if resize_state.note_index < pattern.notes.len() {
                        let mut new_note = pattern.notes[resize_state.note_index];

                        match resize_state.edge {
                            ResizeEdge::Left => {
                                let old_end = new_note.start + new_note.duration;
                                new_note.start =
                                    snapped_beat.min(old_end as f32 - self.grid_snap) as f64;
                                new_note.duration = old_end - new_note.start;
                            }
                            ResizeEdge::Right => {
                                let new_end =
                                    snapped_beat.max(new_note.start as f32 + self.grid_snap);
                                new_note.duration = new_end as f64 - new_note.start;
                            }
                        }

                        actions.push(PianoRollAction::UpdateNote(
                            resize_state.note_index,
                            new_note,
                        ));
                    }
                }
            }

            self.dragging_note = None;
            self.resizing_note = None;
        }

        // Handle clicks for selection and note creation
        if response.clicked()
            && !clicked_on_note
            && self.dragging_note.is_none()
            && self.resizing_note.is_none()
        {
            if let Some(pos) = response.interact_pointer_pos() {
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

                let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap);

                if snapped_beat >= 0.0 && snapped_beat < pattern.length as f32 {
                    actions.push(PianoRollAction::AddNote(MidiNote {
                        pitch,
                        velocity: 100,
                        start: snapped_beat as f64,
                        duration: self.grid_snap as f64,
                    }));
                }
            }
        }

        // Handle right-click for deletion
        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                for (i, note) in pattern.notes.iter().enumerate() {
                    let note_rect = self.note_rect(note, grid_rect);
                    if note_rect.contains(pos) {
                        note_to_remove = Some(i);
                        break;
                    }
                }
            }
        }

        if let Some(idx) = note_to_remove {
            actions.push(PianoRollAction::RemoveNote(idx));
        }

        // Draw notes
        for (i, note) in pattern.notes.iter().enumerate() {
            // Skip drawing the note being dragged/resized (we'll draw preview instead)
            if let Some((preview_idx, _)) = &preview_note {
                if i == *preview_idx {
                    continue;
                }
            }

            let note_rect = self.note_rect(note, grid_rect);

            // Color based on velocity
            let velocity_factor = note.velocity as f32 / 127.0;
            let base_color = if self.selected_notes.contains(&i) {
                egui::Color32::from_rgb(100, 150, 255)
            } else {
                egui::Color32::from_rgb(80, 120, 200)
            };

            // Adjust brightness based on velocity
            let color = egui::Color32::from_rgb(
                (base_color.r() as f32 * velocity_factor) as u8,
                (base_color.g() as f32 * velocity_factor) as u8,
                (base_color.b() as f32 * velocity_factor) as u8,
            );

            ui.painter().rect_filled(note_rect, 2.0, color);
            ui.painter().rect_stroke(
                note_rect,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::BLACK),
                egui::StrokeKind::Outside,
            );

            // Show velocity value when hovering
            if self.hover_note == Some(i)
                && self.dragging_note.is_none()
                && self.resizing_note.is_none()
            {
                ui.painter().text(
                    note_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", note.velocity),
                    egui::FontId::default(),
                    egui::Color32::WHITE,
                );
            }
        }

        // Draw preview of dragged/resized note
        if let Some((_, preview)) = preview_note {
            let preview_rect = self.note_rect(&preview, grid_rect);
            ui.painter().rect_filled(
                preview_rect,
                2.0,
                egui::Color32::from_rgba_premultiplied(100, 150, 255, 128),
            );
            ui.painter().rect_stroke(
                preview_rect,
                2.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(150, 200, 255)),
                egui::StrokeKind::Outside,
            );
        }

        // Draw playhead
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
                let mut indices = self.selected_notes.clone();
                indices.sort_by(|a, b| b.cmp(a));
                for idx in indices {
                    if idx < pattern.notes.len() {
                        actions.push(PianoRollAction::RemoveNote(idx));
                    }
                }
                self.selected_notes.clear();
            }

            // Velocity shortcuts
            if !self.selected_notes.is_empty() {
                if i.key_pressed(egui::Key::ArrowUp) && i.modifiers.ctrl {
                    // Increase velocity
                    for &idx in &self.selected_notes {
                        if idx < pattern.notes.len() {
                            let mut note = pattern.notes[idx];
                            note.velocity = (note.velocity + 10).min(127);
                            actions.push(PianoRollAction::UpdateNote(idx, note));
                        }
                    }
                } else if i.key_pressed(egui::Key::ArrowDown) && i.modifiers.ctrl {
                    // Decrease velocity
                    for &idx in &self.selected_notes {
                        if idx < pattern.notes.len() {
                            let mut note = pattern.notes[idx];
                            note.velocity = note.velocity.saturating_sub(10);
                            actions.push(PianoRollAction::UpdateNote(idx, note));
                        }
                    }
                }
            }
        });

        // Handle scroll and zoom (keep existing code)
        if response.hovered() {
            let scroll_delta = ui.input(|i| i.raw_scroll_delta);
            if ui.input(|i| i.modifiers.ctrl) {
                self.zoom_x *= 1.0 + scroll_delta.y * 0.01;
                self.zoom_x = self.zoom_x.clamp(10.0, 500.0);
            } else {
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

    fn draw_velocity_lane(
        &mut self,
        ui: &mut egui::Ui,
        pattern: &mut Pattern,
        rect: egui::Rect,
    ) -> Vec<PianoRollAction> {
        let mut actions = Vec::new();

        // Background
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_gray(15));

        // Draw velocity bars
        for (i, note) in pattern.notes.iter().enumerate() {
            let x = rect.min.x + (note.start as f32 * self.zoom_x - self.scroll_x);
            let width = note.duration as f32 * self.zoom_x;
            let height = (note.velocity as f32 / 127.0) * rect.height();

            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.max.y - height),
                egui::vec2(width, height),
            );

            let color = if self.selected_notes.contains(&i) {
                egui::Color32::from_rgb(100, 150, 255)
            } else {
                egui::Color32::from_rgb(60, 90, 150)
            };

            ui.painter().rect_filled(bar_rect, 0.0, color);

            // Handle velocity editing
            let response = ui.interact(
                bar_rect,
                ui.id().with(("velocity", i)),
                egui::Sense::click_and_drag(),
            );

            if response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let new_velocity = ((rect.max.y - pos.y) / rect.height() * 127.0)
                        .round()
                        .clamp(0.0, 127.0) as u8;

                    let mut new_note = *note;
                    new_note.velocity = new_velocity;
                    actions.push(PianoRollAction::UpdateNote(i, new_note));
                }
            }
        }

        actions
    }
}

#[derive(Debug)]
pub enum PianoRollAction {
    AddNote(MidiNote),
    RemoveNote(usize),
    UpdateNote(usize, MidiNote),
}
