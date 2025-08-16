use std::vec;

use crate::{
    constants::PIANO_KEY_WIDTH,
    state::{MidiNote, Pattern},
};
use eframe::{egui, egui_glow::painter};

pub struct PianoRoll {
    pub zoom_x: f32, // Pixels per beat
    pub zoom_y: f32, // Pixels per semitone
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub selected_notes: Vec<usize>,
    pub grid_snap: f32, // Snap to grid in beats (0.25 = 16th notes)
    interaction_state: InteractionState,
    hover_note: Option<usize>,
    hover_edge: Option<ResizeEdge>,
    preview_notes: Vec<(u8, bool)>,
}

#[derive(Clone)]
enum InteractionState {
    Idle,
    DraggingNotes {
        initial_positions: Vec<(usize, MidiNote)>,
        click_offset: egui::Vec2, // Offset from click point to first note
        last_beat_delta: f64,
        last_pitch_delta: i32,
    },
    ResizingNotes {
        notes: Vec<(usize, MidiNote, ResizeEdge)>,
        start_pos: egui::Pos2,
    },
    SelectionBox {
        start_pos: egui::Pos2,
    },
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
            hover_note: None,
            interaction_state: InteractionState::Idle,
            hover_edge: None,
            preview_notes: vec![],
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
        let piano_width = PIANO_KEY_WIDTH;
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

        let response = ui.interact(
            grid_rect,
            ui.id().with("piano_roll_grid"),
            egui::Sense::click_and_drag(),
        );

        // Update hover state
        self.hover_note = None;
        self.hover_edge = None;

        if matches!(self.interaction_state, InteractionState::Idle) {
            if let Some(pos) = response.hover_pos() {
                let edge_threshold = 8.0;

                for (i, note) in pattern.notes.iter().enumerate() {
                    let note_rect = self.note_rect(note, grid_rect);

                    if note_rect.contains(pos) {
                        self.hover_note = Some(i);

                        // Check edges for resize
                        if (pos.x - note_rect.left()).abs() < edge_threshold {
                            self.hover_edge = Some(ResizeEdge::Left);
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        } else if (note_rect.right() - pos.x).abs() < edge_threshold {
                            self.hover_edge = Some(ResizeEdge::Right);
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        } else {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                        }
                        break;
                    }
                }
            }
        }

        // Handle mouse down
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                match &self.interaction_state {
                    InteractionState::Idle => {
                        if let Some(hover_idx) = self.hover_note {
                            // Check if we should resize
                            if let Some(edge) = self.hover_edge {
                                // Handle multi-note resize if multiple selected
                                if self.selected_notes.contains(&hover_idx)
                                    && self.selected_notes.len() > 1
                                {
                                    // Resize all selected notes
                                    let notes_to_resize: Vec<(usize, MidiNote, ResizeEdge)> = self
                                        .selected_notes
                                        .iter()
                                        .filter_map(|&idx| {
                                            pattern.notes.get(idx).map(|&note| (idx, note, edge))
                                        })
                                        .collect();

                                    self.interaction_state = InteractionState::ResizingNotes {
                                        notes: notes_to_resize,
                                        start_pos: pos,
                                    };
                                } else {
                                    // Single note resize
                                    self.interaction_state = InteractionState::ResizingNotes {
                                        notes: vec![(hover_idx, pattern.notes[hover_idx], edge)],
                                        start_pos: pos,
                                    };
                                }
                                return actions;
                            }

                            // Handle selection and start drag
                            if !self.selected_notes.contains(&hover_idx) {
                                if !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl) {
                                    self.selected_notes.clear();
                                }
                                self.selected_notes.push(hover_idx);
                            }

                            // Calculate offset from click position to the first selected note
                            let first_selected = self.selected_notes[0];
                            let first_rect =
                                self.note_rect(&pattern.notes[first_selected], grid_rect);
                            let click_offset = pos - first_rect.left_top();

                            let initial_positions: Vec<(usize, MidiNote)> = self
                                .selected_notes
                                .iter()
                                .filter_map(|&idx| pattern.notes.get(idx).map(|&note| (idx, note)))
                                .collect();

                            self.interaction_state = InteractionState::DraggingNotes {
                                initial_positions,
                                click_offset,
                                last_beat_delta: 0.0,
                                last_pitch_delta: 0,
                            };
                        } else {
                            // Clicking on empty space
                            if ui.input(|i| i.modifiers.shift) {
                                self.interaction_state =
                                    InteractionState::SelectionBox { start_pos: pos };
                            } else if !ui.input(|i| i.modifiers.ctrl) {
                                self.selected_notes.clear();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Handle dragging
        if response.dragged() {
            if let Some(current_pos) = response.hover_pos() {
                match &mut self.interaction_state {
                    InteractionState::DraggingNotes {
                        initial_positions,
                        click_offset,
                        last_beat_delta,
                        last_pitch_delta,
                    } => {
                        // Calculate new position based on mouse position
                        let grid_pos = current_pos - grid_rect.min;
                        let beat = (grid_pos.x - click_offset.x + self.scroll_x) / self.zoom_x;
                        let snapped_beat = (beat / self.grid_snap).round() * self.grid_snap;

                        let pitch_y = grid_pos.y - click_offset.y + self.scroll_y;
                        let pitch_float = 127.0 - (pitch_y / self.zoom_y);
                        let pitch = pitch_float.round() as i32;

                        // Calculate delta from the first note's original position
                        if let Some((_, first_original)) = initial_positions.first() {
                            let beat_delta = snapped_beat as f64 - first_original.start;
                            let pitch_delta = pitch - first_original.pitch as i32;

                            // Check if pitch changed for preview sound
                            if pitch_delta != *last_pitch_delta {
                                // Trigger preview sound for the new pitch
                                if let Some((_, first_note)) = initial_positions.first() {
                                    let preview_pitch = ((first_note.pitch as i32 + pitch_delta)
                                        .clamp(0, 127))
                                        as u8;
                                    actions.push(PianoRollAction::PreviewNote(preview_pitch));
                                }
                                *last_pitch_delta = pitch_delta;
                            }

                            *last_beat_delta = beat_delta;

                            // Update all selected notes with the same delta
                            for (idx, original_note) in initial_positions.iter() {
                                if let Some(note) = pattern.notes.get_mut(*idx) {
                                    // Ensure note stays within pattern bounds (to remove later)
                                    let new_start = (original_note.start + beat_delta).max(0.0);
                                    let max_start = pattern.length - original_note.duration;
                                    note.start = new_start.min(max_start);
                                    note.pitch = ((original_note.pitch as i32 + pitch_delta)
                                        .clamp(0, 127))
                                        as u8;
                                }
                            }
                        }
                    }
                    InteractionState::ResizingNotes { notes, start_pos } => {
                        let grid_x =
                            (current_pos.x - grid_rect.left() + self.scroll_x) / self.zoom_x;
                        let snapped_beat =
                            ((grid_x / self.grid_snap).round() * self.grid_snap).max(0.0);

                        // Calculate resize amount based on first note
                        if let Some((_, first_original, edge)) = notes.first() {
                            let resize_amount = match edge {
                                ResizeEdge::Left => snapped_beat as f64 - first_original.start,
                                ResizeEdge::Right => {
                                    snapped_beat as f64
                                        - (first_original.start + first_original.duration)
                                }
                            };

                            for (idx, original_note, edge) in notes.iter() {
                                if let Some(note) = pattern.notes.get_mut(*idx) {
                                    match edge {
                                        ResizeEdge::Left => {
                                            let new_start =
                                                (original_note.start + resize_amount).max(0.0).min(
                                                    original_note.start + original_note.duration
                                                        - self.grid_snap as f64,
                                                );
                                            note.duration = (original_note.start
                                                + original_note.duration)
                                                - new_start;
                                            note.start = new_start;
                                        }
                                        ResizeEdge::Right => {
                                            let new_duration = (original_note.duration
                                                + resize_amount)
                                                .max(self.grid_snap as f64);
                                            note.duration = new_duration;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    InteractionState::SelectionBox { start_pos } => {
                        let rect = egui::Rect::from_two_pos(*start_pos, current_pos);
                        ui.painter().rect_filled(
                            rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(100, 150, 255, 20),
                        );
                        ui.painter().rect_stroke(
                            rect,
                            0.0,
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
                            egui::StrokeKind::Inside,
                        );

                        // Update selection
                        if !ui.input(|i| i.modifiers.ctrl) {
                            self.selected_notes.clear();
                        }

                        for (i, note) in pattern.notes.iter().enumerate() {
                            let note_rect = self.note_rect(note, grid_rect);
                            if rect.intersects(note_rect) && !self.selected_notes.contains(&i) {
                                self.selected_notes.push(i);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Handle mouse up
        if response.drag_stopped() {
            // Stop any preview sounds
            actions.push(PianoRollAction::StopPreview);
            self.interaction_state = InteractionState::Idle;
        }

        // Handle single click
        if response.clicked() && !response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(hover_idx) = self.hover_note {
                    // Toggle selection
                    if ui.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                        if self.selected_notes.contains(&hover_idx) {
                            self.selected_notes.retain(|&i| i != hover_idx);
                        } else {
                            self.selected_notes.push(hover_idx);
                        }
                    } else {
                        self.selected_notes.clear();
                        self.selected_notes.push(hover_idx);
                    }
                } else {
                    // Create new note
                    if !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl) {
                        self.selected_notes.clear();
                    }

                    let grid_pos = pos - grid_rect.min;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let pitch_float = 127.0 - ((grid_pos.y + self.scroll_y) / self.zoom_y);
                    let pitch = pitch_float.round().clamp(0.0, 127.0) as u8;
                    let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap).max(0.0);

                    // Ensure note doesn't exceed pattern length
                    if (snapped_beat as f64) < pattern.length {
                        let duration =
                            (self.grid_snap as f64).min(pattern.length - snapped_beat as f64);
                        actions.push(PianoRollAction::AddNote(MidiNote {
                            pitch,
                            velocity: 100,
                            start: snapped_beat as f64,
                            duration,
                        }));
                    }
                }
            }
        }

        // Draw notes with drag handles
        for (i, note) in pattern.notes.iter().enumerate() {
            let note_rect = self.note_rect(note, grid_rect);

            let is_selected = self.selected_notes.contains(&i);
            let velocity_factor = note.velocity as f32 / 127.0;

            let base_color = if is_selected {
                egui::Color32::from_rgb(120, 170, 255)
            } else {
                egui::Color32::from_rgb(80, 120, 200)
            };

            let color = egui::Color32::from_rgb(
                (base_color.r() as f32 * velocity_factor) as u8,
                (base_color.g() as f32 * velocity_factor) as u8,
                (base_color.b() as f32 * velocity_factor) as u8,
            );

            ui.painter().rect_filled(note_rect, 2.0, color);

            // Draw resize handles for selected notes
            if is_selected {
                let handle_width = 4.0;

                // Left handle
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        note_rect.left_top(),
                        egui::vec2(handle_width, note_rect.height()),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 100),
                );

                // Right handle
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(note_rect.right() - handle_width, note_rect.top()),
                        egui::vec2(handle_width, note_rect.height()),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 100),
                );
            }

            // Draw border
            ui.painter().rect_stroke(
                note_rect,
                2.0,
                egui::Stroke::new(
                    if is_selected { 2.0 } else { 1.0 },
                    if is_selected {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::BLACK
                    },
                ),
                egui::StrokeKind::Inside,
            );

            // Show velocity on hover
            if self.hover_note == Some(i)
                && matches!(self.interaction_state, InteractionState::Idle)
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

        // Handle right-click for deletion
        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                // Check if we right-clicked on a selected note
                let mut delete_selected = false;
                for &idx in &self.selected_notes {
                    if let Some(note) = pattern.notes.get(idx) {
                        let note_rect = self.note_rect(note, grid_rect);
                        if note_rect.contains(pos) {
                            delete_selected = true;
                            break;
                        }
                    }
                }

                if delete_selected {
                    // Delete all selected notes
                    let mut indices = self.selected_notes.clone();
                    indices.sort_by(|a, b| b.cmp(a)); // Sort in reverse order
                    for idx in indices {
                        if idx < pattern.notes.len() {
                            actions.push(PianoRollAction::RemoveNote(idx));
                        }
                    }
                    self.selected_notes.clear();
                } else {
                    // Delete only the clicked note
                    for (i, note) in pattern.notes.iter().enumerate() {
                        let note_rect = self.note_rect(note, grid_rect);
                        if note_rect.contains(pos) {
                            actions.push(PianoRollAction::RemoveNote(i));
                            self.selected_notes.retain(|&idx| idx != i);
                            break;
                        }
                    }
                }
            }
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

            // Select all
            if i.key_pressed(egui::Key::A) && i.modifiers.ctrl {
                self.selected_notes = (0..pattern.notes.len()).collect();
            }

            // Velocity shortcuts
            if !self.selected_notes.is_empty() {
                if i.key_pressed(egui::Key::ArrowUp) && i.modifiers.ctrl {
                    for &idx in &self.selected_notes {
                        if idx < pattern.notes.len() {
                            let mut note = pattern.notes[idx];
                            note.velocity = (note.velocity + 10).min(127);
                            actions.push(PianoRollAction::UpdateNote(idx, note));
                        }
                    }
                } else if i.key_pressed(egui::Key::ArrowDown) && i.modifiers.ctrl {
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

        // Handle scroll and zoom
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
}

#[derive(Debug)]
pub enum PianoRollAction {
    AddNote(MidiNote),
    RemoveNote(usize),
    UpdateNote(usize, MidiNote),
    PreviewNote(u8),
    StopPreview,
}
