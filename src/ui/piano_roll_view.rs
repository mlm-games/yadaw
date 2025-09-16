use std::collections::{HashMap, HashSet};
use std::fs::read_to_string;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::scroll_area::ScrollSource;
use rayon::vec;

use super::*;
use crate::constants::{DEFAULT_MIDI_CLIP_LEN, PIANO_KEY_WIDTH};
use crate::model::{MidiClip, MidiNote};
use crate::ui::piano_roll::{PianoRoll, PianoRollAction};

pub struct PianoRollView {
    pub(crate) piano_roll: PianoRoll,
    pub(crate) selected_clip: Option<usize>,
    pub(crate) editing_notes: Vec<MidiNote>,

    // View settings
    show_velocity_lane: bool,
    show_controller_lanes: bool,
    velocity_lane_height: f32,

    // Tool modes
    tool_mode: ToolMode,

    // MIDI input
    midi_input_enabled: bool,
    midi_octave_offset: i32,

    undo_armed: bool,
    write_to_all_selected: bool,
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
            show_velocity_lane: false,
            show_controller_lanes: false,
            velocity_lane_height: 100.0,

            tool_mode: ToolMode::Select,

            midi_input_enabled: false,
            midi_octave_offset: 0,
            selected_clip: None,
            editing_notes: vec![],
            undo_armed: false,
            write_to_all_selected: false,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.vertical(|ui| {
            // Header
            self.draw_header(ui, app);
            ui.separator();

            let total_w = ui.available_width();
            let total_h = ui.available_height();

            let piano_roll_height = if self.show_velocity_lane {
                (total_h - self.velocity_lane_height - 6.0).max(0.0)
            } else {
                total_h
            };

            // Piano roll area
            let (roll_resp, _) =
                ui.allocate_painter(egui::vec2(total_w, piano_roll_height), egui::Sense::hover());
            let roll_rect = roll_resp.rect;

            // // Mark notes as the active edit target when the roll is hot
            // if roll_resp.hovered() || roll_resp.is_pointer_button_down_on() {
            //     app.active_edit_target = super::app::ActiveEditTarget::Notes;
            // }

            ui.allocate_ui_at_rect(roll_rect, |ui| {
                ui.set_clip_rect(roll_rect);
                self.draw_piano_roll(ui, app);

                if let Some(current_beat) = ui
                    .ctx()
                    .memory(|m| m.data.get_temp::<f64>(egui::Id::new("current_beat")))
                {
                    let grid_left = roll_rect.left() + crate::constants::PIANO_KEY_WIDTH;
                    let x = grid_left
                        + (current_beat as f32 * self.piano_roll.zoom_x - self.piano_roll.scroll_x);

                    if x >= roll_rect.left() && x <= roll_rect.right() {
                        ui.painter().line_segment(
                            [
                                egui::pos2(x, roll_rect.top()),
                                egui::pos2(x, roll_rect.bottom()),
                            ],
                            egui::Stroke::new(2.0, crate::constants::COLOR_PLAYHEAD),
                        );
                    }
                }
            });

            self.handle_touch_pan_zoom(ui.ctx(), roll_rect);

            // Velocity lane
            if self.show_velocity_lane {
                ui.add_space(2.0);
                let (lane_resp, _) = ui.allocate_painter(
                    egui::vec2(total_w, self.velocity_lane_height),
                    egui::Sense::click_and_drag(),
                );
                let lane_rect = lane_resp.rect;

                ui.allocate_ui_at_rect(lane_rect, |ui| {
                    ui.set_clip_rect(lane_rect);
                    self.draw_velocity_lane(ui, lane_rect, app);
                });
            }

            if self.show_controller_lanes {
                ui.separator();
                self.draw_controller_lanes(ui, app);
            }
        });
    }

    pub fn menu_copy_notes(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) -> bool {
        if self.selected_clip.is_none() {
            return false;
        }
        // Resolve selection indices against current editing buffer
        let sel_idx = {
            // Build id->idx map
            use std::collections::HashMap;
            let mut id_to_idx = HashMap::new();
            for (i, n) in self.editing_notes.iter().enumerate() {
                if n.id != 0 {
                    id_to_idx.insert(n.id, i);
                }
            }
            let mut out: Vec<usize> = self
                .piano_roll
                .selected_note_ids
                .iter()
                .filter_map(|id| id_to_idx.get(id).copied())
                .collect();
            for &i in &self.piano_roll.temp_selected_indices {
                if i < self.editing_notes.len() {
                    out.push(i);
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };

        if sel_idx.is_empty() {
            return false;
        }

        let mut to_copy = Vec::with_capacity(sel_idx.len());
        let mut min_start = f64::INFINITY;
        for &idx in &sel_idx {
            if let Some(n) = self.editing_notes.get(idx) {
                min_start = min_start.min(n.start);
            }
        }
        if !min_start.is_finite() {
            return false;
        }
        for &idx in &sel_idx {
            if let Some(n) = self.editing_notes.get(idx).copied() {
                let mut nn = n;
                nn.start = (nn.start - min_start).max(0.0);
                to_copy.push(nn);
            }
        }

        app.note_clipboard = Some(to_copy.clone());
        let CLIP_ID: egui::Id = egui::Id::new("piano_roll_clipboard");
        ctx.memory_mut(|m| m.data.insert_persisted(CLIP_ID, to_copy));
        true
    }

    pub fn menu_cut_notes(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) -> bool {
        if !self.menu_copy_notes(ctx, app) {
            return false;
        }
        // Resolve indices again (defensive)
        let mut sel_idx = {
            use std::collections::HashMap;
            let mut id_to_idx = HashMap::new();
            for (i, n) in self.editing_notes.iter().enumerate() {
                if n.id != 0 {
                    id_to_idx.insert(n.id, i);
                }
            }
            let mut out: Vec<usize> = self
                .piano_roll
                .selected_note_ids
                .iter()
                .filter_map(|id| id_to_idx.get(id).copied())
                .collect();
            for &i in &self.piano_roll.temp_selected_indices {
                if i < self.editing_notes.len() {
                    out.push(i);
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };

        if sel_idx.is_empty() {
            return false;
        }

        sel_idx.sort_unstable_by(|a, b| b.cmp(a));
        for idx in sel_idx {
            if idx < self.editing_notes.len() {
                self.editing_notes.remove(idx);
            }
        }
        self.piano_roll.selected_note_ids.clear();
        self.piano_roll.temp_selected_indices.clear();

        if let Some(clip_idx) = self.selected_clip {
            let _ = app
                .command_tx
                .send(crate::messages::AudioCommand::UpdateMidiClip(
                    app.selected_track,
                    clip_idx,
                    self.editing_notes.clone(),
                ));
            return true;
        }
        false
    }

    pub fn menu_paste_notes(
        &mut self,
        ctx: &egui::Context,
        app: &mut super::app::YadawApp,
    ) -> bool {
        if self.selected_clip.is_none() {
            return false;
        }

        let clip_len = {
            let state = app.state.lock().unwrap();
            if let Some(track) = state.tracks.get(app.selected_track) {
                if let Some(clip_idx) = self.selected_clip {
                    track
                        .midi_clips
                        .get(clip_idx)
                        .map(|c| c.length_beats)
                        .unwrap_or(0.0)
                } else {
                    0.0
                }
            } else {
                0.0
            }
        };
        if clip_len <= 0.0 {
            return false;
        }

        let CLIP_ID: egui::Id = egui::Id::new("piano_roll_clipboard");
        let buf_opt = app
            .note_clipboard
            .clone()
            .or_else(|| ctx.memory_mut(|m| m.data.get_persisted::<Vec<MidiNote>>(CLIP_ID)));
        let mut buf = if let Some(b) = buf_opt {
            b
        } else {
            return false;
        };
        if buf.is_empty() {
            return false;
        }

        let mut target = ctx
            .memory(|m| m.data.get_temp::<f64>(egui::Id::new("current_beat")))
            .unwrap_or(0.0);
        let snap = self.piano_roll.grid_snap as f64;
        target = if snap > 0.0 {
            ((target / snap).round() * snap).max(0.0)
        } else {
            target.max(0.0)
        };

        let span = buf
            .iter()
            .map(|n| n.start + n.duration)
            .fold(0.0_f64, f64::max);
        if target + span > clip_len {
            target = (clip_len - span).max(0.0);
        }

        let mut new_indices = Vec::with_capacity(buf.len());
        let mut base = self.editing_notes.len();
        for mut n in buf.drain(..) {
            n.start = (target + n.start).max(0.0);
            if n.start >= clip_len {
                continue;
            }
            let max_dur = (clip_len - n.start).max(0.0);
            if max_dur <= 0.0 {
                continue;
            }
            let min_dur = if self.piano_roll.grid_snap > 0.0 {
                self.piano_roll.grid_snap as f64
            } else {
                (clip_len * 0.001).max(1e-6)
            };
            n.duration = n.duration.min(max_dur).max(min_dur);

            self.editing_notes.push(n);
            new_indices.push(base);
            base += 1;
        }
        if new_indices.is_empty() {
            return false;
        }
        self.piano_roll.selected_note_ids.clear();
        self.piano_roll.temp_selected_indices = new_indices;

        if let Some(clip_idx) = self.selected_clip {
            let _ = app
                .command_tx
                .send(crate::messages::AudioCommand::UpdateMidiClip(
                    app.selected_track,
                    clip_idx,
                    self.editing_notes.clone(),
                ));
            return true;
        }
        false
    }

    pub fn select_all_notes(&mut self) {
        self.piano_roll.selected_note_ids.clear();
        self.piano_roll.temp_selected_indices.clear();
        for (i, n) in self.editing_notes.iter().enumerate() {
            if n.id != 0 {
                self.piano_roll.selected_note_ids.push(n.id);
            } else {
                self.piano_roll.temp_selected_indices.push(i);
            }
        }
    }

    pub fn set_editing_clip(&mut self, clip_idx: usize) {
        self.selected_clip = Some(clip_idx);
    }

    fn draw_header(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        egui::ScrollArea::horizontal()
            .id_salt("pr_tool_strip")
            .scroll_source(ScrollSource::ALL)
            // .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Piano Roll");

                    ui.separator();

                    // Clip selector
                    ui.label("MIDI Clip:");
                    let state = app.state.lock().unwrap();
                    if let Some(track) = state.tracks.get(app.selected_track) {
                        let selected_text = if let Some(clip_idx) = self.selected_clip {
                            track
                                .midi_clips
                                .get(clip_idx)
                                .map(|c| c.name.as_str())
                                .unwrap_or("No Clip")
                        } else {
                            "No Clip Selected"
                        };

                        egui::ComboBox::from_id_salt("clip_selector")
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for (i, clip) in track.midi_clips.iter().enumerate() {
                                    if ui
                                        .selectable_value(
                                            &mut self.selected_clip,
                                            Some(i),
                                            &clip.name,
                                        )
                                        .clicked()
                                    {
                                        // Load clip notes for editing
                                        self.editing_notes = clip.notes.clone();
                                    }
                                }
                            });

                        // Create new clip button
                        if ui
                            .button("➕")
                            .on_hover_text("Create New MIDI Clip")
                            .clicked()
                        {
                            let current_beat =
                                state.position_to_beats(app.audio_state.get_position());
                            let _ = app.command_tx.send(AudioCommand::CreateMidiClip(
                                app.selected_track,
                                current_beat,
                                DEFAULT_MIDI_CLIP_LEN, // Default 1 bar length
                            ));
                        }

                        // Duplicate clip button
                        if ui.button("⎘").on_hover_text("Duplicate Clip").clicked() {
                            if let Some(clip_idx) = self.selected_clip {
                                if let Some(clip) = track.midi_clips.get(clip_idx) {
                                    let new_clip = MidiClip {
                                        name: format!("{} (copy)", clip.name),
                                        start_beat: clip.start_beat + clip.length_beats,
                                        length_beats: clip.length_beats,
                                        notes: clip.notes.clone(),
                                        color: clip.color,
                                        ..Default::default()
                                    };
                                    // Add the duplicated clip
                                    let _ =
                                        app.command_tx.send(AudioCommand::CreateMidiClipWithData(
                                            app.selected_track,
                                            new_clip,
                                        ));
                                }
                            }
                        }

                        // Delete clip button
                        if self.selected_clip.is_some() {
                            if ui.button("🗑").on_hover_text("Delete Clip").clicked() {
                                if let Some(clip_idx) = self.selected_clip {
                                    let _ = app.command_tx.send(AudioCommand::DeleteMidiClip(
                                        app.selected_track,
                                        clip_idx,
                                    ));
                                    self.selected_clip = None;
                                    self.editing_notes.clear();
                                }
                            }
                        }

                        ui.checkbox(
                            &mut self.write_to_all_selected,
                            "Write to all selected clips",
                        )
                        .on_hover_text(
                            "Apply edits to all selected MIDI clips (in addition to aliases).",
                        );
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
                    egui::ComboBox::from_id_salt("piano_roll_snap")
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
            });
    }

    fn draw_piano_roll(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        if self.selected_clip.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select or create a MIDI clip to edit");
            });
            return;
        }

        if let Some(clip_idx) = self.selected_clip {
            let model_notes = {
                let state = app.state.lock().unwrap();
                state
                    .tracks
                    .get(app.selected_track)
                    .and_then(|t| t.midi_clips.get(clip_idx))
                    .map(|c| c.notes.clone())
            };
            if let Some(model_notes) = model_notes {
                // Only overwrite if you're not currently pushing edits
                let mid_edit = self.undo_armed || ui.input(|i| i.pointer.any_down());
                if !mid_edit && model_notes != self.editing_notes {
                    self.editing_notes = model_notes;

                    // Clean up selection to only contain ids that still exist
                    {
                        // Build a quick map from (pitch,start,duration) to id for the refreshed notes
                        let mut sig_to_id: HashMap<(u8, i64, i64), u64> = HashMap::new();
                        for n in &self.editing_notes {
                            // Use integer signature to avoid fp jitter
                            let s = (
                                (n.start * 1_000_000.0).round() as i64,
                                (n.duration * 1_000_000.0).round() as i64,
                            );
                            sig_to_id.insert((n.pitch, s.0, s.1), n.id);
                        }

                        // Promote any temp-selected indices to ids
                        for &idx in &self.piano_roll.temp_selected_indices {
                            if let Some(n) = self.editing_notes.get(idx) {
                                let s = (
                                    (n.start * 1_000_000.0).round() as i64,
                                    (n.duration * 1_000_000.0).round() as i64,
                                );
                                if let Some(id) = sig_to_id.get(&(n.pitch, s.0, s.1)).copied() {
                                    if id != 0 && !self.piano_roll.selected_note_ids.contains(&id) {
                                        self.piano_roll.selected_note_ids.push(id);
                                    }
                                }
                            }
                        }

                        // Now prune ids that disappeared
                        let ids_now: std::collections::HashSet<u64> =
                            self.editing_notes.iter().map(|n| n.id).collect();
                        self.piano_roll
                            .selected_note_ids
                            .retain(|id| *id != 0 && ids_now.contains(id));

                        // Finally clear temp indices
                        self.piano_roll.temp_selected_indices.clear();
                    }
                }
            } else {
                self.selected_clip = None;
                self.editing_notes.clear();
                self.piano_roll.selected_note_ids.clear();
                self.piano_roll.temp_selected_indices.clear();
            }
        }

        // Prefetch a pool of note IDs for immediate assignment
        if app.reserved_note_ids.len() < 32 {
            let _ = app
                .command_tx
                .send(crate::messages::AudioCommand::ReserveNoteIds(64));
        }

        let clip_length = {
            let state = app.state.lock().unwrap();
            if let Some(track) = state.tracks.get(app.selected_track) {
                if let Some(clip_idx) = self.selected_clip {
                    track.midi_clips.get(clip_idx).map(|c| c.length_beats)
                } else {
                    None
                }
            } else {
                None
            }
        };
        let Some(clip_length) = clip_length else {
            return;
        };

        let old_notes = self.editing_notes.clone();
        let mut temp_clip = crate::model::MidiClip {
            name: "temp".to_string(),
            start_beat: 0.0,
            length_beats: clip_length,
            notes: old_notes.clone(),
            color: Some((1, 1, 1)),
            ..Default::default()
        };

        // Sync IN persisted clipboard
        {
            let CLIP_ID: egui::Id = egui::Id::new("piano_roll_clipboard");
            if let Some(buf) = app.note_clipboard.clone() {
                ui.ctx()
                    .memory_mut(|m| m.data.insert_persisted(CLIP_ID, buf));
            }
        }

        {
            let state = app.state.lock().unwrap();
            if let Some(primary_idx) = self.selected_clip {
                if let Some(track) = state.tracks.get(app.selected_track) {
                    // Collect “other” clips: either from selected_clips or those overlapping the editor range
                    let others: Vec<&crate::model::clip::MidiClip> = app
                        .selected_clips
                        .iter()
                        .filter(|(t, c)| *t == app.selected_track && *c != primary_idx)
                        .filter_map(|(_, c)| track.midi_clips.get(*c))
                        .collect();

                    // Draw
                    let grid_left = ui.min_rect().left() + crate::constants::PIANO_KEY_WIDTH;
                    for oc in others {
                        let color = egui::Color32::from_rgba_premultiplied(255, 255, 255, 40);
                        for n in &oc.notes {
                            let x = grid_left
                                + (n.start as f32 * self.piano_roll.zoom_x
                                    - self.piano_roll.scroll_x);
                            let w = (n.duration as f32 * self.piano_roll.zoom_x).max(2.0);
                            let y = ui.min_rect().bottom()
                                - ((n.pitch as f32 / 127.0) * (ui.min_rect().height() - 0.0));
                            let r = egui::Rect::from_min_size(
                                egui::pos2(x, y - 2.0),
                                egui::vec2(w, 2.0),
                            );
                            ui.painter().rect_filled(r, 1.0, color);
                        }
                    }
                }
            }
        }

        let actions = self
            .piano_roll
            .ui(ui, &mut temp_clip, self.tool_mode == ToolMode::Draw);

        // Apply actions (assign IDs on AddNote immediately if possible)
        let mut action_changed = false;
        for a in &actions {
            match a {
                PianoRollAction::AddNote(n) => {
                    let mut nn = *n;

                    // Snap epsilon to treat same-cell adds as duplicates
                    const EPS: f64 = 1e-6;

                    // Duplicate check: same pitch, same snapped start
                    if let Some(existing_idx) = self
                        .editing_notes
                        .iter()
                        .position(|x| x.pitch == nn.pitch && (x.start - nn.start).abs() < EPS)
                    {
                        // Select the existing note instead of adding a duplicate
                        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);

                        if !additive {
                            self.piano_roll.selected_note_ids.clear();
                            self.piano_roll.temp_selected_indices.clear();
                        }
                        let existing = self.editing_notes[existing_idx];
                        if existing.id != 0 {
                            if !self.piano_roll.selected_note_ids.contains(&existing.id) {
                                self.piano_roll.selected_note_ids.push(existing.id);
                            }
                        } else if !self
                            .piano_roll
                            .temp_selected_indices
                            .contains(&existing_idx)
                        {
                            self.piano_roll.temp_selected_indices.push(existing_idx);
                        }
                        // No change to notes; don’t mark action_changed
                        continue;
                    }

                    // Assign an id if we have a reserved one (helps keep selection stable later)
                    if nn.id == 0 {
                        if let Some(id) = app.reserved_note_ids.pop() {
                            nn.id = id;
                        }
                    }

                    // Add note
                    let new_index = self.editing_notes.len();
                    self.editing_notes.push(nn);
                    action_changed = true;

                    // Select the newly added note (additive with Shift/Ctrl)
                    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
                    if !additive {
                        self.piano_roll.selected_note_ids.clear();
                        self.piano_roll.temp_selected_indices.clear();
                    }
                    let just = self.editing_notes[new_index];
                    if just.id != 0 {
                        self.piano_roll.selected_note_ids.push(just.id);
                    } else {
                        self.piano_roll.temp_selected_indices.push(new_index);
                    }
                }
                PianoRollAction::RemoveNote(idx) => {
                    if *idx < self.editing_notes.len() {
                        self.editing_notes.remove(*idx);
                        action_changed = true;
                    }
                }
                PianoRollAction::UpdateNote(idx, n) => {
                    if *idx < self.editing_notes.len() {
                        let mut nn = *n;
                        if nn.id == 0 {
                            if let Some(id) = app.reserved_note_ids.pop() {
                                nn.id = id;
                            }
                        }
                        self.editing_notes[*idx] = nn;
                        action_changed = true;
                    }
                }
                PianoRollAction::PreviewNote(pitch) => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::PreviewNote(
                            app.selected_track,
                            *pitch,
                        ));
                }
                PianoRollAction::StopPreview => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::StopPreviewNote);
                }
            }
        }

        if !action_changed {
            // adopt drag/resize edits from temp clip
            self.editing_notes = temp_clip.notes.clone();
        }

        let changed = self.editing_notes != old_notes;

        if changed && !self.undo_armed {
            app.push_undo();
            self.undo_armed = true;
        }

        if changed {
            let mem_root = egui::Id::new(("pr_tx", app.selected_track, self.selected_clip));
            let mut last_send = ui
                .ctx()
                .memory(|m| m.data.get_temp::<Instant>(mem_root.with("last_send")));
            let mut dirty = ui
                .ctx()
                .memory(|m| m.data.get_temp::<bool>(mem_root.with("dirty")))
                .unwrap_or(false);

            dirty = true;
            let now = Instant::now();
            let due =
                last_send.map_or(true, |t| now.duration_since(t) >= Duration::from_millis(30));
            if due {
                if let Some(clip_idx) = self.selected_clip {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::UpdateMidiClip(
                            app.selected_track,
                            clip_idx,
                            self.editing_notes.clone(),
                        ));
                    last_send = Some(now);
                    dirty = false;
                }

                if self.write_to_all_selected {
                    // Build target list = selected clips excluding the primary (and excluding those that share pattern_id with primary to avoid redundant updates)
                    let targets: Vec<(usize, usize)> = {
                        let st = app.state.lock().unwrap();
                        let mut v = Vec::new();
                        let primary_pid = st
                            .tracks
                            .get(app.selected_track)
                            .and_then(|t| self.selected_clip.and_then(|cid| t.midi_clips.get(cid)))
                            .and_then(|c| c.pattern_id);
                        for (t, c) in app.selected_clips.iter().copied() {
                            if t == app.selected_track && Some(c) == self.selected_clip {
                                continue;
                            }
                            if let Some(track) = st.tracks.get(t) {
                                if track.is_midi && track.midi_clips.get(c).is_some() {
                                    // skip aliases of the primary to avoid redundant write (they’ll mirror anyway)
                                    let is_alias_of_primary = primary_pid.is_some()
                                        && track.midi_clips[c].pattern_id == primary_pid;
                                    if !is_alias_of_primary {
                                        v.push((t, c));
                                    }
                                }
                            }
                        }
                        v
                    };
                    if !targets.is_empty() {
                        let _ = app.command_tx.send(
                            crate::messages::AudioCommand::UpdateMidiClipsSameNotes {
                                targets,
                                notes: self.editing_notes.clone(),
                            },
                        );
                    }
                }
            }

            // flush on pointer release
            let released = ui.input(|i| i.pointer.any_released());
            if released && dirty {
                if let Some(clip_idx) = self.selected_clip {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::UpdateMidiClip(
                            app.selected_track,
                            clip_idx,
                            self.editing_notes.clone(),
                        ));
                }
                dirty = false;
            }
            if released {
                self.undo_armed = false;
            }

            ui.ctx().memory_mut(|m| {
                if let Some(t) = last_send {
                    m.data.insert_temp(mem_root.with("last_send"), t);
                }
                m.data.insert_temp(mem_root.with("dirty"), dirty);
            });
        }

        // Sync OUT clipboard
        {
            let CLIP_ID: egui::Id = egui::Id::new("piano_roll_clipboard");
            let persisted: Option<Vec<MidiNote>> =
                ui.ctx().memory_mut(|m| m.data.get_persisted(CLIP_ID));
            if persisted.is_some() {
                app.note_clipboard = persisted;
            }
        }
    }

    fn draw_velocity_lane(
        &mut self,
        ui: &mut egui::Ui,
        lane_rect: egui::Rect,
        app: &mut super::app::YadawApp,
    ) {
        // Prepare painter strictly clipped to lane_rect
        let painter = ui.painter_at(lane_rect);

        // Background
        painter.rect_filled(lane_rect, 0.0, egui::Color32::from_gray(15));

        // Get current clip (if any)
        let (clip_opt, track_is_midi) = {
            let st = app.state.lock().unwrap();
            let track_opt = st.tracks.get(app.selected_track);
            let is_midi = track_opt.map(|t| t.is_midi).unwrap_or(false);
            let clip_opt = match (track_opt, self.selected_clip) {
                (Some(t), Some(idx)) => t.midi_clips.get(idx),
                _ => None,
            };
            (clip_opt.cloned(), is_midi)
        };

        if !track_is_midi || clip_opt.is_none() {
            return; // nothing to draw
        }

        let clip = clip_opt.unwrap();

        // Left gutter aligned with the piano keyboard width
        let grid_left = lane_rect.left() + crate::constants::PIANO_KEY_WIDTH;
        let gutter =
            egui::Rect::from_min_max(lane_rect.min, egui::pos2(grid_left, lane_rect.bottom()));
        painter.rect_filled(gutter, 0.0, egui::Color32::from_gray(10));

        // Horizontal guide lines
        for i in 0..=4 {
            let y = lane_rect.top() + (i as f32 / 4.0) * lane_rect.height();
            painter.line_segment(
                [egui::pos2(grid_left, y), egui::pos2(lane_rect.right(), y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
            );
        }

        // Selection indices resolved against this clip
        let sel_idx = self.piano_roll.selected_indices(&clip);

        // Draw each velocity bar using same scroll_x/zoom_x as the piano roll grid
        for (i, note) in clip.notes.iter().enumerate() {
            let x =
                grid_left + (note.start as f32 * self.piano_roll.zoom_x - self.piano_roll.scroll_x);
            let w = (note.duration as f32 * self.piano_roll.zoom_x).max(2.0);
            let h = (note.velocity as f32 / 127.0) * lane_rect.height();

            let left = x.max(grid_left);
            let right = (x + w).min(lane_rect.right());
            if right <= left {
                continue;
            }

            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(left, lane_rect.bottom() - h),
                egui::vec2(right - left, h),
            );

            let color = if sel_idx.contains(&i) {
                egui::Color32::from_rgb(100, 150, 255)
            } else {
                egui::Color32::from_rgb(60, 90, 150)
            };

            // Paint bar (clipped to lane_rect)
            painter.rect_filled(bar_rect, 0.0, color);

            // Interact with the bar using Ui (for pointer + drag)
            let resp = ui.interact(
                bar_rect,
                ui.id().with(("velocity", i)),
                egui::Sense::click_and_drag(),
            );

            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let new_velocity = ((lane_rect.bottom() - pos.y) / lane_rect.height() * 127.0)
                        .round()
                        .clamp(0.0, 127.0) as u8;

                    if new_velocity != note.velocity {
                        let mut new_note = *note;
                        new_note.velocity = new_velocity;
                        if let Some(clip_idx) = self.selected_clip {
                            let _ = app
                                .command_tx
                                .send(crate::messages::AudioCommand::UpdateNote(
                                    app.selected_track,
                                    clip_idx,
                                    i,
                                    new_note,
                                ));
                        }
                    }
                }
            }
        }

        // Hover readout
        if let Some(pos) = ui
            .interact(
                lane_rect,
                ui.id().with("velocity_lane"),
                egui::Sense::hover(),
            )
            .hover_pos()
        {
            let velocity = ((lane_rect.bottom() - pos.y) / lane_rect.height() * 127.0)
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

    fn draw_controller_lanes(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.group(|ui| {
            ui.set_min_height(100.0);

            ui.horizontal(|ui| {
                ui.label("Controller:");

                egui::ComboBox::from_id_salt("controller_select")
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

    fn handle_touch_pan_zoom(&mut self, ctx: &egui::Context, roll_rect: egui::Rect) {
        // IDs for temp memory
        let id_centroid = egui::Id::new(("pr_gesture", "centroid"));
        let id_dist = egui::Id::new(("pr_gesture", "dist"));

        // 1) Gather touch points inside the roll rect (read-only; no memory calls here)
        let points: Vec<egui::Pos2> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Touch { pos, phase, .. } = e {
                        match phase {
                            egui::TouchPhase::Start | egui::TouchPhase::Move => {
                                if roll_rect.contains(*pos) {
                                    Some(*pos)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                })
                .collect()
        });

        // 2) If we have two touches, pan + pinch
        if points.len() >= 2 {
            let centroid = egui::pos2(
                (points[0].x + points[1].x) * 0.5,
                (points[0].y + points[1].y) * 0.5,
            );
            let dist = (points[0] - points[1]).length();

            // Read previous gesture state (outside ctx.input)
            let (prev_centroid, prev_dist) = ctx.memory(|m| {
                (
                    m.data.get_temp::<egui::Pos2>(id_centroid),
                    m.data.get_temp::<f32>(id_dist),
                )
            });

            if let (Some(pc), Some(pd)) = (prev_centroid, prev_dist) {
                // Pan by centroid delta (pixels)
                let delta = centroid - pc;
                self.piano_roll.scroll_x = (self.piano_roll.scroll_x - delta.x).max(0.0);
                self.piano_roll.scroll_y = (self.piano_roll.scroll_y - delta.y).max(0.0);

                // Pinch zoom on X, keep pinch center anchored
                if pd > 1.0 {
                    let scale = (dist / pd).clamp(0.5, 2.0);
                    let old_zoom_x = self.piano_roll.zoom_x;
                    self.piano_roll.zoom_x = (self.piano_roll.zoom_x * scale).clamp(10.0, 500.0);

                    if (self.piano_roll.zoom_x - old_zoom_x).abs() > f32::EPSILON {
                        let grid_left = roll_rect.left() + crate::constants::PIANO_KEY_WIDTH;
                        let cx = (centroid.x - grid_left + self.piano_roll.scroll_x) / old_zoom_x;
                        self.piano_roll.scroll_x =
                            (cx * self.piano_roll.zoom_x - (centroid.x - grid_left)).max(0.0);
                    }
                }
            }

            // Save gesture state for next frame (outside ctx.input)
            ctx.memory_mut(|m| {
                m.data.insert_temp(id_centroid, centroid);
                m.data.insert_temp(id_dist, dist);
            });
        } else {
            // No pinch -> clear temp state (outside ctx.input)
            ctx.memory_mut(|m| {
                m.data.remove::<egui::Pos2>(id_centroid);
                m.data.remove::<f32>(id_dist);
            });
        }
    }
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self::new()
    }
}
