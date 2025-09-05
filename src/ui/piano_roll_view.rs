use std::fs::read_to_string;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rayon::vec;

use super::*;
use crate::constants::{DEFAULT_MIDI_CLIP_LEN, PIANO_KEY_WIDTH};
use crate::model::{MidiClip, MidiNote};
use crate::piano_roll::{PianoRoll, PianoRollAction};

pub struct PianoRollView {
    piano_roll: PianoRoll,
    selected_clip: Option<usize>,
    editing_notes: Vec<MidiNote>,

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
            show_velocity_lane: true,
            show_controller_lanes: false,
            velocity_lane_height: 100.0,

            tool_mode: ToolMode::Select,

            midi_input_enabled: false,
            midi_octave_offset: 0,
            selected_clip: None,
            editing_notes: vec![],
            undo_armed: false,
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
                (total_h - self.velocity_lane_height - 6.0).max(0.0) // 6px spacing budget
            } else {
                total_h
            };

            // 1) Allocate rect for the piano roll area and draw inside a child UI
            let (roll_resp, _roll_painter) =
                ui.allocate_painter(egui::vec2(total_w, piano_roll_height), egui::Sense::hover());
            let roll_rect = roll_resp.rect;

            let mut roll_ui =
                ui.child_ui(roll_rect, egui::Layout::top_down(egui::Align::Min), None);
            self.draw_piano_roll(&mut roll_ui, app);

            // Always-on-top playhead overlay for the piano roll area
            if let Some(current_beat) = ui
                .ctx()
                .memory(|m| m.data.get_temp::<f64>(egui::Id::new("current_beat")))
            {
                let grid_left = roll_rect.left() + crate::constants::PIANO_KEY_WIDTH;
                let x = grid_left
                    + (current_beat as f32 * self.piano_roll.zoom_x - self.piano_roll.scroll_x);

                if x >= roll_rect.left() && x <= roll_rect.right() {
                    ui.ctx().debug_painter().line_segment(
                        [
                            egui::pos2(x, roll_rect.top()),
                            egui::pos2(x, roll_rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, crate::constants::COLOR_PLAYHEAD),
                    );
                }
            }

            // 2) Velocity lane below
            if self.show_velocity_lane {
                ui.add_space(2.0);
                let (lane_resp, _lane_painter) = ui.allocate_painter(
                    egui::vec2(total_w, self.velocity_lane_height),
                    egui::Sense::click_and_drag(),
                );
                let lane_rect = lane_resp.rect;

                let mut lane_ui =
                    ui.child_ui(lane_rect, egui::Layout::top_down(egui::Align::Min), None);
                self.draw_velocity_lane(&mut lane_ui, app);
            }

            // Controller lanes (if any)
            if self.show_controller_lanes {
                ui.separator();
                self.draw_controller_lanes(ui, app);
            }
        });
    }

    pub fn set_editing_clip(&mut self, clip_idx: usize) {
        self.selected_clip = Some(clip_idx);
    }

    fn draw_header(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
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
                                .selectable_value(&mut self.selected_clip, Some(i), &clip.name)
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
                    let current_beat = state.position_to_beats(app.audio_state.get_position());
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
                            let _ = app.command_tx.send(AudioCommand::CreateMidiClipWithData(
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
                            let _ = app
                                .command_tx
                                .send(AudioCommand::DeleteMidiClip(app.selected_track, clip_idx));
                            self.selected_clip = None;
                            self.editing_notes.clear();
                        }
                    }
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
    }

    fn draw_piano_roll(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        if self.selected_clip.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select or create a MIDI clip to edit");
            });
            return;
        }

        // Get current clip length to bound the grid
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

        // Prepare a temp clip for the UI (start at 0, we edit relative to clip)
        let old_notes = self.editing_notes.clone();
        let mut temp_clip = crate::model::MidiClip {
            name: "temp".to_string(),
            start_beat: 0.0,
            length_beats: clip_length,
            notes: old_notes.clone(),
            color: Some((1, 1, 1)),
            ..Default::default()
        };

        // Run the piano roll UI; it may mutate temp_clip.notes for drag/resize
        let actions = self.piano_roll.ui(ui, &mut temp_clip);

        // Throttle state (egui temp memory)
        let mem_root = egui::Id::new(("pr_tx", app.selected_track, self.selected_clip));
        let mut last_send = ui
            .ctx()
            .memory(|m| m.data.get_temp::<Instant>(mem_root.with("last_send")));
        let mut dirty = ui
            .ctx()
            .memory(|m| m.data.get_temp::<bool>(mem_root.with("dirty")))
            .unwrap_or(false);

        // Apply action-based edits (tap-to-add, delete, one-off updates)
        // Note: PianoRoll::ui does NOT change notes for Add/Remove/Update; we must apply them.
        let mut action_changed = false;
        for a in &actions {
            match a {
                crate::piano_roll::PianoRollAction::AddNote(n) => {
                    self.editing_notes.push(*n);
                    action_changed = true;
                }
                crate::piano_roll::PianoRollAction::RemoveNote(idx) => {
                    if *idx < self.editing_notes.len() {
                        self.editing_notes.remove(*idx);
                        action_changed = true;
                    }
                }
                crate::piano_roll::PianoRollAction::UpdateNote(idx, n) => {
                    if *idx < self.editing_notes.len() {
                        self.editing_notes[*idx] = *n;
                        action_changed = true;
                    }
                }
                crate::piano_roll::PianoRollAction::PreviewNote(pitch) => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::PreviewNote(
                            app.selected_track,
                            *pitch,
                        ));
                }
                crate::piano_roll::PianoRollAction::StopPreview => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::StopPreviewNote);
                }
            }
        }

        // Merge drag/resize edits coming from temp_clip.notes (UI mutated that)
        // If actions already wrote into editing_notes, take that as leading source; otherwise adopt temp UI edits.
        if !action_changed {
            self.editing_notes = temp_clip.notes.clone();
        } else {
            // keep UI’s grid-mutated ordering consistent if actions happened too
            // (optional: you can sort by start if desired)
        }

        // Detect if notes changed (either via actions or UI drag/resize)
        let changed = self.editing_notes != old_notes;

        // Arm undo once per gesture when the first change is detected
        if changed && !self.undo_armed {
            app.push_undo();
            self.undo_armed = true;
        }

        // Throttled send during drags
        if changed {
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
                    dirty = false; // just flushed
                }
            }
        }

        // Final flush on pointer release if anything is pending
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

        // Persist throttle flags
        ui.ctx().memory_mut(|m| {
            if let Some(t) = last_send {
                m.data.insert_temp(mem_root.with("last_send"), t);
            }
            m.data.insert_temp(mem_root.with("dirty"), dirty);
        });
    }

    fn draw_velocity_lane(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let state = app.state.lock().unwrap();

        if let Some(track) = state.tracks.get(app.selected_track) {
            if let Some(clip_idx) = self.selected_clip {
                if let Some(pattern) = track.midi_clips.get(clip_idx) {
                    let rect = ui.max_rect(); // this is the lane rect
                    let painter = ui.painter();

                    // Backgrounds
                    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(15));

                    // Keyboard gutter at left
                    let grid_left = rect.left() + PIANO_KEY_WIDTH;
                    let gutter_rect =
                        egui::Rect::from_min_max(rect.min, egui::pos2(grid_left, rect.bottom()));
                    painter.rect_filled(gutter_rect, 0.0, egui::Color32::from_gray(10));

                    // Horizontal guides
                    for i in 0..=4 {
                        let y = rect.top() + (i as f32 / 4.0) * rect.height();
                        painter.line_segment(
                            [egui::pos2(grid_left, y), egui::pos2(rect.right(), y)],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                        );
                    }

                    // Bars
                    for (i, note) in pattern.notes.iter().enumerate() {
                        let x = grid_left
                            + (note.start as f32 * self.piano_roll.zoom_x
                                - self.piano_roll.scroll_x);
                        let width = (note.duration as f32 * self.piano_roll.zoom_x).max(2.0);
                        let height = (note.velocity as f32 / 127.0) * rect.height();

                        let left = x.max(grid_left);
                        let right = (x + width).min(rect.right());
                        if right <= left {
                            continue;
                        }

                        let bar_rect = egui::Rect::from_min_size(
                            egui::pos2(left, rect.bottom() - height),
                            egui::vec2(right - left, height),
                        );

                        let is_selected = self.piano_roll.selected_notes.contains(&i);
                        let color = if is_selected {
                            egui::Color32::from_rgb(100, 150, 255)
                        } else {
                            egui::Color32::from_rgb(60, 90, 150)
                        };

                        painter.rect_filled(bar_rect, 0.0, color);

                        // Drag to change velocity
                        let resp = ui.interact(
                            bar_rect,
                            ui.id().with(("velocity", i)),
                            egui::Sense::click_and_drag(),
                        );
                        if resp.dragged() {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                let new_velocity = ((rect.bottom() - pos.y) / rect.height() * 127.0)
                                    .round()
                                    .clamp(0.0, 127.0)
                                    as u8;

                                if new_velocity != note.velocity {
                                    let mut new_note = *note;
                                    new_note.velocity = new_velocity;

                                    let _ = app.command_tx.send(AudioCommand::UpdateNote(
                                        app.selected_track,
                                        clip_idx,
                                        i,
                                        new_note,
                                    ));
                                }
                            }
                        }
                    }

                    // Hover readout
                    if let Some(pos) = ui
                        .interact(rect, ui.id().with("velocity_lane"), egui::Sense::hover())
                        .hover_pos()
                    {
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
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self::new()
    }
}
