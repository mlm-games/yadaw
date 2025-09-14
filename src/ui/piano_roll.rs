use std::vec;

use crate::{
    constants::PIANO_KEY_WIDTH,
    model::{MidiClip, MidiNote},
};
use eframe::{egui, egui_glow::painter};
use egui::generate_loader_id;

pub struct PianoRoll {
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub selected_note_ids: Vec<u64>,
    pub temp_selected_indices: Vec<usize>,
    pub grid_snap: f32,
    interaction_state: InteractionState,
    hover_note: Option<usize>,
    hover_edge: Option<ResizeEdge>,
    preview_notes: Vec<(u8, bool)>,
    note_clipboard: Option<Vec<MidiNote>>,
}

impl Default for PianoRoll {
    fn default() -> Self {
        Self {
            zoom_x: 100.0,
            zoom_y: 20.0,
            scroll_x: 0.0,
            scroll_y: 60.0 * 20.0,
            grid_snap: 0.25,
            selected_note_ids: Vec::new(),
            temp_selected_indices: Vec::new(),
            hover_note: None,
            interaction_state: InteractionState::Idle,
            hover_edge: None,
            preview_notes: vec![],
            note_clipboard: None,
        }
    }
}

#[derive(Clone)]
enum InteractionState {
    Idle,
    DraggingNotes {
        initial_positions: Vec<(usize, MidiNote)>,
        click_offset: egui::Vec2,
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

#[derive(Clone, Copy, PartialEq)]
enum ResizeEdge {
    Left,
    Right,
}

impl PianoRoll {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        pattern: &mut MidiClip,
        allow_add_on_click: bool,
    ) -> Vec<PianoRollAction> {
        let mut actions = Vec::new();
        let available_rect = ui.available_rect_before_wrap();

        // Background
        ui.painter()
            .rect_filled(available_rect, 0.0, egui::Color32::from_gray(20));

        // Piano keys and grid
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
        self.draw_grid(&ui.painter(), grid_rect, pattern.length_beats);

        let response = ui.interact(
            grid_rect,
            ui.id().with("piano_roll_grid"),
            egui::Sense::click_and_drag(),
        );

        // Hover state
        self.hover_note = None;
        self.hover_edge = None;

        if matches!(self.interaction_state, InteractionState::Idle) {
            if let Some(pos) = response.hover_pos() {
                let edge_threshold = 8.0;

                for (i, note) in pattern.notes.iter().enumerate() {
                    let note_rect = self.note_rect(note, grid_rect);
                    if note_rect.contains(pos) {
                        self.hover_note = Some(i);
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

        // Begin drag (supports Alt+Drag duplicate)
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                match &self.interaction_state {
                    InteractionState::Idle => {
                        if let Some(hover_idx) = self.hover_note {
                            let alt_held = ui.input(|i| i.modifiers.alt);

                            if let Some(edge) = self.hover_edge {
                                // Resize path (selection by indices resolved from IDs)
                                let selected = self.selected_indices(pattern);
                                if selected.contains(&hover_idx) && selected.len() > 1 {
                                    let notes_to_resize: Vec<(usize, MidiNote, ResizeEdge)> =
                                        selected
                                            .into_iter()
                                            .filter_map(|idx| {
                                                pattern
                                                    .notes
                                                    .get(idx)
                                                    .copied()
                                                    .map(|note| (idx, note, edge))
                                            })
                                            .collect();
                                    self.interaction_state = InteractionState::ResizingNotes {
                                        notes: notes_to_resize,
                                        start_pos: pos,
                                    };
                                } else {
                                    self.interaction_state = InteractionState::ResizingNotes {
                                        notes: vec![(hover_idx, pattern.notes[hover_idx], edge)],
                                        start_pos: pos,
                                    };
                                }
                                return actions;
                            }

                            // keep multi-selection if hover is already in the current selection
                            let current_sel = self.selected_indices(pattern);
                            let hovered_in_selection = current_sel.contains(&hover_idx);
                            if !hovered_in_selection {
                                let additive = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
                                self.select_single(&pattern.notes[hover_idx], hover_idx, additive);
                            }
                            let current_sel = self.selected_indices(pattern);

                            // Alt+Drag duplicate: duplicate the resolved selection
                            let drag_indices: Vec<usize> = if alt_held {
                                let mut new_indices = Vec::with_capacity(current_sel.len());
                                for &idx in &current_sel {
                                    if let Some(n) = pattern.notes.get(idx).copied() {
                                        let mut dup = n;
                                        // duplicated notes must not reuse ids
                                        dup.id = 0;
                                        let base = pattern.notes.len();
                                        pattern.notes.push(dup);
                                        new_indices.push(base);
                                    }
                                }
                                // Selection now by indices for the new duplicates
                                self.clear_selection();
                                self.temp_selected_indices = new_indices.clone();
                                new_indices
                            } else {
                                current_sel
                            };

                            // Compute click offset from first dragged
                            let first_selected = drag_indices[0];
                            let first_rect =
                                self.note_rect(&pattern.notes[first_selected], grid_rect);
                            let click_offset = pos - first_rect.left_top();

                            let initial_positions: Vec<(usize, MidiNote)> = drag_indices
                                .iter()
                                .filter_map(|&idx| {
                                    pattern.notes.get(idx).copied().map(|note| (idx, note))
                                })
                                .collect();

                            self.interaction_state = InteractionState::DraggingNotes {
                                initial_positions,
                                click_offset,
                                last_beat_delta: 0.0,
                                last_pitch_delta: 0,
                            };
                        } else {
                            // Empty space
                            if ui.input(|i| i.modifiers.shift) {
                                self.interaction_state =
                                    InteractionState::SelectionBox { start_pos: pos };
                            } else if !ui.input(|i| i.modifiers.ctrl) {
                                self.clear_selection();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Drag move
        if response.dragged() {
            if let Some(current_pos) = response.hover_pos() {
                match &mut self.interaction_state {
                    InteractionState::DraggingNotes {
                        initial_positions,
                        click_offset,
                        last_beat_delta,
                        last_pitch_delta,
                    } => {
                        let grid_pos = current_pos - grid_rect.min;
                        let beat = (grid_pos.x - click_offset.x + self.scroll_x) / self.zoom_x;
                        let snapped_beat = (beat / self.grid_snap).round() * self.grid_snap;

                        let pitch_y = grid_pos.y - click_offset.y + self.scroll_y;
                        let pitch_float = 127.0 - (pitch_y / self.zoom_y);
                        let pitch = pitch_float.floor().clamp(0.0, 127.0) as i32;

                        if let Some((_, first_original)) = initial_positions.first() {
                            let beat_delta = snapped_beat as f64 - first_original.start;
                            let pitch_delta = pitch - first_original.pitch as i32;

                            if pitch_delta != *last_pitch_delta {
                                if let Some((_, first_note)) = initial_positions.first() {
                                    let preview_pitch = ((first_note.pitch as i32 + pitch_delta)
                                        .clamp(0, 127))
                                        as u8;
                                    actions.push(PianoRollAction::PreviewNote(preview_pitch));
                                }
                                *last_pitch_delta = pitch_delta;
                            }

                            *last_beat_delta = beat_delta;

                            for (idx, original_note) in initial_positions.iter() {
                                if let Some(note) = pattern.notes.get_mut(*idx) {
                                    let new_start = (original_note.start + beat_delta).max(0.0);
                                    let max_start =
                                        (pattern.length_beats - original_note.duration).max(0.0);
                                    note.start = new_start.min(max_start);
                                    note.pitch = ((original_note.pitch as i32 + pitch_delta)
                                        .clamp(0, 127))
                                        as u8;
                                }
                            }
                        }
                    }
                    InteractionState::ResizingNotes {
                        notes,
                        start_pos: _,
                    } => {
                        let grid_x =
                            (current_pos.x - grid_rect.left() + self.scroll_x) / self.zoom_x;
                        let snapped_beat =
                            ((grid_x / self.grid_snap).round() * self.grid_snap).max(0.0);

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
                            self.clear_selection();
                        }
                        for (i, note) in pattern.notes.iter().enumerate() {
                            let note_rect = self.note_rect(note, grid_rect);
                            if rect.intersects(note_rect) {
                                if note.id != 0 {
                                    if !self.selected_note_ids.contains(&note.id) {
                                        self.selected_note_ids.push(note.id);
                                    }
                                } else if !self.temp_selected_indices.contains(&i) {
                                    self.temp_selected_indices.push(i);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Mouse up
        if response.drag_stopped() {
            actions.push(PianoRollAction::StopPreview);
            self.interaction_state = InteractionState::Idle;
        }

        // Double-click
        if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                // delete-on-note or create-on-empty
                let mut deleted = false;
                for i in 0..pattern.notes.len() {
                    let note_rect = self.note_rect(&pattern.notes[i], grid_rect);
                    if note_rect.contains(pos) {
                        actions.push(PianoRollAction::RemoveNote(i));
                        let n = pattern.notes[i];
                        if n.id != 0 {
                            self.selected_note_ids.retain(|&id| id != n.id);
                        } else {
                            self.temp_selected_indices.retain(|&j| j != i);
                        }
                        deleted = true;
                        break;
                    }
                }
                if !deleted {
                    let grid_pos = pos - grid_rect.min;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let pitch_float = 127.0 - ((grid_pos.y + self.scroll_y) / self.zoom_y);
                    let pitch = pitch_float.floor().clamp(0.0, 127.0) as u8;
                    let snapped_beat = ((beat / self.grid_snap).round() * self.grid_snap).max(0.0);
                    if (snapped_beat as f64) < pattern.length_beats {
                        let duration =
                            (self.grid_snap as f64).min(pattern.length_beats - snapped_beat as f64);
                        actions.push(PianoRollAction::AddNote(MidiNote {
                            id: 0,
                            pitch,
                            velocity: 100,
                            start: snapped_beat as f64,
                            duration,
                        }));
                    }
                }
            }
        }
        // Single click (with Alt audition preserved)
        else if response.clicked() && !response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                // 1) Detect if the tap hit an existing note right now (not relying on hover)
                let clicked_idx = pattern
                    .notes
                    .iter()
                    .enumerate()
                    .rev() // top-most first if overlapping
                    .find(|(_, n)| self.note_rect(n, grid_rect).contains(pos))
                    .map(|(i, _)| i);

                if ui.input(|i| i.modifiers.alt) {
                    // Alt = preview
                    if let Some(idx) = clicked_idx {
                        if let Some(n) = pattern.notes.get(idx) {
                            actions.push(PianoRollAction::PreviewNote(n.pitch));
                        }
                    } else {
                        let pitch = {
                            let pf =
                                127.0 - ((pos.y - grid_rect.min.y + self.scroll_y) / self.zoom_y);
                            pf.floor().clamp(0.0, 127.0) as u8
                        };
                        actions.push(PianoRollAction::PreviewNote(pitch));
                    }
                    return actions;
                }

                if let Some(idx) = clicked_idx {
                    // Selection behavior (respect multi-select modifiers)
                    if ui.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                        self.toggle_single(&pattern.notes[idx], idx);
                    } else {
                        self.select_single(&pattern.notes[idx], idx, false);
                    }
                    // No AddNote action
                    return actions;
                }

                // 3) Otherwise it's empty space: only add if allowed (Draw tool), else do nothing
                if allow_add_on_click {
                    let grid_pos = pos - grid_rect.min;
                    let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    let snapped_beat = if self.grid_snap > 0.0 {
                        ((beat / self.grid_snap) as f64).round() * self.grid_snap as f64
                    } else {
                        beat as f64
                    }
                    .max(0.0);

                    // compute pitch with floor (so tap matches visual row)
                    let pitch = {
                        let pf = 127.0 - ((grid_pos.y + self.scroll_y) / self.zoom_y);
                        pf.floor().clamp(0.0, 127.0) as u8
                    };

                    // Keep duration at grid size (or your minimum)
                    let duration = if self.grid_snap > 0.0 {
                        self.grid_snap as f64
                    } else {
                        0.25f64
                    };

                    // Don’t add on top of an existing same-cell note; select it instead.
                    const EPS: f64 = 1e-6;
                    if let Some(existing_idx) = pattern
                        .notes
                        .iter()
                        .position(|n| n.pitch == pitch && (n.start - snapped_beat).abs() < EPS)
                    {
                        // Select existing
                        self.select_single(&pattern.notes[existing_idx], existing_idx, false);
                    } else {
                        actions.push(PianoRollAction::AddNote(MidiNote {
                            id: 0,
                            pitch,
                            velocity: 100,
                            start: snapped_beat,
                            duration,
                        }));
                    }
                }
                // If not allowed to add (Select tool): do nothing on empty space tap
            }
        }

        // Draw notes using selection by ID or temp index
        for (i, note) in pattern.notes.iter().enumerate() {
            let note_rect = self.note_rect(note, grid_rect);
            let is_selected = (note.id != 0 && self.selected_note_ids.contains(&note.id))
                || (note.id == 0 && self.temp_selected_indices.contains(&i));
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

            if is_selected {
                let handle_width = 4.0;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        note_rect.left_top(),
                        egui::vec2(handle_width, note_rect.height()),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 100),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(note_rect.right() - handle_width, note_rect.top()),
                        egui::vec2(handle_width, note_rect.height()),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 100),
                );
            }

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

        // Context menu delete (ID-aware removal from selection)
        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let mut delete_selected = false;
                for (i, note) in pattern.notes.iter().enumerate() {
                    if ((note.id != 0 && self.selected_note_ids.contains(&note.id))
                        || (note.id == 0 && self.temp_selected_indices.contains(&i)))
                        && self.note_rect(note, grid_rect).contains(pos)
                    {
                        delete_selected = true;
                        break;
                    }
                }

                if delete_selected {
                    let mut del = self.selected_indices(pattern);
                    del.sort_unstable_by(|a, b| b.cmp(a));
                    for idx in del {
                        if idx < pattern.notes.len() {
                            actions.push(PianoRollAction::RemoveNote(idx));
                        }
                    }
                    self.clear_selection();
                } else {
                    for (i, note) in pattern.notes.iter().enumerate() {
                        if self.note_rect(note, grid_rect).contains(pos) {
                            actions.push(PianoRollAction::RemoveNote(i));
                            if note.id != 0 {
                                self.selected_note_ids.retain(|&id| id != note.id);
                            } else {
                                self.temp_selected_indices.retain(|&j| j != i);
                            }
                            break;
                        }
                    }
                }
            }
        }

        // in-roll playhead
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

        // Shortcuts (ID-aware)
        self.handle_editor_shortcuts(ui, pattern, grid_rect, &response, &mut actions);

        // Scroll/zoom
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

    fn handle_editor_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        pattern: &mut MidiClip,
        _grid_rect: egui::Rect,
        response: &egui::Response,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let editor_hot = response.hovered()
            || response.is_pointer_button_down_on()
            || !matches!(self.interaction_state, InteractionState::Idle);

        let allow = editor_hot && !ui.ctx().wants_keyboard_input();

        let cmd = egui::Modifiers::COMMAND;
        let sc_copy = egui::KeyboardShortcut::new(cmd, egui::Key::C);
        let sc_cut = egui::KeyboardShortcut::new(cmd, egui::Key::X);
        let sc_paste = egui::KeyboardShortcut::new(cmd, egui::Key::V);
        let sc_selecta = egui::KeyboardShortcut::new(cmd, egui::Key::A);
        let sc_v_up = egui::KeyboardShortcut::new(cmd, egui::Key::ArrowUp);
        let sc_v_down = egui::KeyboardShortcut::new(cmd, egui::Key::ArrowDown);

        let CLIP_ID: egui::Id = egui::Id::new("piano_roll_clipboard");

        let grid = if self.grid_snap > 0.0 {
            self.grid_snap as f64
        } else {
            0.25
        };
        let fine = (grid / 4.0).max(1e-6);
        let coarse = 1.0_f64;

        // Resolve current selection to indices once per frame
        let sel_idx = self.selected_indices(pattern);

        // Select all
        if allow && ui.input_mut(|i| i.consume_shortcut(&sc_selecta)) {
            self.clear_selection();
            for (i, n) in pattern.notes.iter().enumerate() {
                if n.id != 0 {
                    self.selected_note_ids.push(n.id);
                } else {
                    self.temp_selected_indices.push(i);
                }
            }
        }

        // Delete
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            if !sel_idx.is_empty() {
                let mut sel = sel_idx.clone();
                sel.sort_unstable_by(|a, b| b.cmp(a));
                for idx in sel {
                    if idx < pattern.notes.len() {
                        pattern.notes.remove(idx);
                    }
                }
                self.clear_selection();
            }
        }

        // Velocity with COMMAND
        if allow && ui.input_mut(|i| i.consume_shortcut(&sc_v_up)) {
            for &idx in &sel_idx {
                if idx < pattern.notes.len() {
                    let mut n = pattern.notes[idx];
                    n.velocity = (n.velocity.saturating_add(10)).min(127);
                    actions.push(PianoRollAction::UpdateNote(idx, n));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_shortcut(&sc_v_down)) {
            for &idx in &sel_idx {
                if idx < pattern.notes.len() {
                    let mut n = pattern.notes[idx];
                    n.velocity = n.velocity.saturating_sub(10);
                    actions.push(PianoRollAction::UpdateNote(idx, n));
                }
            }
        }

        // Transpose (no modifier / SHIFT)
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    nn.pitch = (nn.pitch.saturating_add(1)).min(127);
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    nn.pitch = nn.pitch.saturating_sub(1);
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowUp)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    nn.pitch = (nn.pitch as i32 + 12).clamp(0, 127) as u8;
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowDown)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    nn.pitch = (nn.pitch as i32 - 12).clamp(0, 127) as u8;
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }

        // Nudge (grid / fine / coarse)
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    let new_start = (nn.start - grid).max(0.0);
                    nn.start = new_start.min((pattern.length_beats - nn.duration).max(0.0));
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    let max_start = (pattern.length_beats - nn.duration).max(0.0);
                    nn.start = (nn.start + grid).min(max_start);
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    let new_start = (nn.start - fine).max(0.0);
                    nn.start = new_start.min((pattern.length_beats - nn.duration).max(0.0));
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    let max_start = (pattern.length_beats - nn.duration).max(0.0);
                    nn.start = (nn.start + fine).min(max_start);
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowLeft)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    let new_start = (nn.start - coarse).max(0.0);
                    nn.start = new_start.min((pattern.length_beats - nn.duration).max(0.0));
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }
        if allow && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowRight)) {
            for &idx in &sel_idx {
                if let Some(n) = pattern.notes.get(idx).copied() {
                    let mut nn = n;
                    let max_start = (pattern.length_beats - nn.duration).max(0.0);
                    nn.start = (nn.start + coarse).min(max_start);
                    actions.push(PianoRollAction::UpdateNote(idx, nn));
                }
            }
        }

        // COPY
        if allow && ui.input_mut(|i| i.consume_shortcut(&sc_copy)) {
            if !sel_idx.is_empty() {
                let mut to_copy = Vec::with_capacity(sel_idx.len());
                let mut min_start = f64::INFINITY;
                for &idx in &sel_idx {
                    if let Some(n) = pattern.notes.get(idx) {
                        min_start = min_start.min(n.start);
                    }
                }
                for &idx in &sel_idx {
                    if let Some(n) = pattern.notes.get(idx) {
                        let mut nn = *n;
                        nn.start = (nn.start - min_start).max(0.0);
                        to_copy.push(nn);
                    }
                }
                self.note_clipboard = Some(to_copy.clone());
                ui.ctx()
                    .memory_mut(|m| m.data.insert_persisted(CLIP_ID, to_copy));
            }
        }

        // CUT
        if allow && ui.input_mut(|i| i.consume_shortcut(&sc_cut)) {
            if !sel_idx.is_empty() {
                // Copy
                let mut to_copy = Vec::with_capacity(sel_idx.len());
                let mut min_start = f64::INFINITY;
                for &idx in &sel_idx {
                    if let Some(n) = pattern.notes.get(idx) {
                        min_start = min_start.min(n.start);
                    }
                }
                for &idx in &sel_idx {
                    if let Some(n) = pattern.notes.get(idx) {
                        let mut nn = *n;
                        nn.start = (nn.start - min_start).max(0.0);
                        to_copy.push(nn);
                    }
                }
                self.note_clipboard = Some(to_copy.clone());
                ui.ctx()
                    .memory_mut(|m| m.data.insert_persisted(CLIP_ID, to_copy));

                // Delete selected notes by index (reverse order)
                let mut del = sel_idx.clone();
                del.sort_unstable_by(|a, b| b.cmp(a));
                for idx in del {
                    if idx < pattern.notes.len() {
                        pattern.notes.remove(idx);
                    }
                }
                self.clear_selection();
            }
        }

        // PASTE
        if allow && ui.input_mut(|i| i.consume_shortcut(&sc_paste)) {
            let buf_opt = self.note_clipboard.clone().or_else(|| {
                ui.ctx()
                    .memory_mut(|m| m.data.get_persisted::<Vec<MidiNote>>(CLIP_ID))
            });

            if let Some(mut buf) = buf_opt {
                if buf.is_empty() {
                    return;
                }
                let span = buf
                    .iter()
                    .map(|n| n.start + n.duration)
                    .fold(0.0_f64, f64::max);

                let mut target = ui
                    .ctx()
                    .memory(|m| m.data.get_temp::<f64>(egui::Id::new("current_beat")))
                    .unwrap_or(0.0);
                let snap = self.grid_snap as f64;
                target = if snap > 0.0 {
                    ((target / snap).round() * snap).max(0.0)
                } else {
                    target.max(0.0)
                };

                let clip_len = pattern.length_beats.max(0.0);
                if target + span > clip_len {
                    target = (clip_len - span).max(0.0);
                }

                // Append and select pasted (indices; ids not assigned yet)
                let mut new_indices = Vec::with_capacity(buf.len());
                let mut base = pattern.notes.len();

                for mut n in buf.drain(..) {
                    n.start = (target + n.start).max(0.0);
                    if n.start >= clip_len {
                        continue;
                    }
                    let max_dur = (clip_len - n.start).max(0.0);
                    if max_dur <= 0.0 {
                        continue;
                    }
                    let min_dur = if self.grid_snap > 0.0 {
                        self.grid_snap as f64
                    } else {
                        (clip_len * 0.001).max(1e-6)
                    };
                    n.duration = n.duration.min(max_dur).max(min_dur);

                    pattern.notes.push(n);
                    new_indices.push(base);
                    base += 1;
                }

                if !new_indices.is_empty() {
                    // Selection after paste by indices (ids may be 0 for new notes)
                    self.clear_selection();
                    self.temp_selected_indices = new_indices;
                }
            }
        }
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

    pub fn selected_indices(&self, pattern: &MidiClip) -> Vec<usize> {
        use std::collections::HashMap;
        // Map id -> index for fast lookup
        let mut id_to_idx = HashMap::new();
        for (i, n) in pattern.notes.iter().enumerate() {
            if n.id != 0 {
                id_to_idx.insert(n.id, i);
            }
        }
        // Collect indices for id-based selection
        let mut out: Vec<usize> = self
            .selected_note_ids
            .iter()
            .filter_map(|id| id_to_idx.get(id).copied())
            .collect();
        // Add any fallback indices that still exist
        for &idx in &self.temp_selected_indices {
            if idx < pattern.notes.len() {
                out.push(idx);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn clear_selection(&mut self) {
        self.selected_note_ids.clear();
        self.temp_selected_indices.clear();
    }

    fn select_single(&mut self, note: &MidiNote, idx: usize, additive: bool) {
        if !additive {
            self.clear_selection();
        }
        if note.id != 0 {
            if !self.selected_note_ids.contains(&note.id) {
                self.selected_note_ids.push(note.id);
            }
            // If this id was previously selected via index, clear it
            self.temp_selected_indices.retain(|&i| i != idx);
        } else {
            if !self.temp_selected_indices.contains(&idx) {
                self.temp_selected_indices.push(idx);
            }
        }
    }

    fn toggle_single(&mut self, note: &MidiNote, idx: usize) {
        if note.id != 0 {
            if let Some(pos) = self.selected_note_ids.iter().position(|&id| id == note.id) {
                self.selected_note_ids.remove(pos);
            } else {
                self.selected_note_ids.push(note.id);
            }
            self.temp_selected_indices.retain(|&i| i != idx);
        } else {
            if let Some(pos) = self.temp_selected_indices.iter().position(|&i| i == idx) {
                self.temp_selected_indices.remove(pos);
            } else {
                self.temp_selected_indices.push(idx);
            }
        }
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

pub fn y_to_pitch(y_pixels: f32, scroll_y: f32, zoom_y: f32) -> u8 {
    let pf = 127.0 - ((y_pixels + scroll_y) / zoom_y);
    pf.floor().clamp(0.0, 127.0) as u8
}
