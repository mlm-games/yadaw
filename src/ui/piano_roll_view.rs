use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use egui::scroll_area::ScrollSource;
use egui::{Sense, UiBuilder};

use super::*;
use crate::audio_state::AudioState;
use crate::constants::{DEFAULT_MIDI_CLIP_LEN, PIANO_KEY_WIDTH};
use crate::messages::AudioCommand;
use crate::model::{MidiClip, MidiNote};
use crate::project::AppState;
use crate::ui::piano_roll::{PianoRoll, PianoRollAction};

pub struct PianoRollView {
    pub piano_roll: PianoRoll,
    pub selected_clip: Option<u64>,

    // View settings
    show_velocity_lane: bool,
    velocity_lane_height: f32,

    // Tool modes
    tool_mode: ToolMode,

    // MIDI input
    midi_input_enabled: bool,
    midi_octave_offset: i32,

    // Interaction tracking
    drag_in_progress: bool,
    last_undo_snapshot: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolMode {
    Select,
    Draw,
}

impl PianoRollView {
    pub fn new() -> Self {
        Self {
            piano_roll: PianoRoll::default(),
            show_velocity_lane: false,
            velocity_lane_height: 100.0,
            tool_mode: ToolMode::Select,
            midi_input_enabled: false,
            midi_octave_offset: 0,
            selected_clip: None,
            drag_in_progress: false,
            last_undo_snapshot: None,
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

            let grid_left_roll = roll_rect.left() + crate::constants::PIANO_KEY_WIDTH;

            ui.scope_builder(
                UiBuilder::new().max_rect(roll_rect).sense(Sense::hover()),
                |ui| {
                    self.draw_piano_roll(ui, app);

                    // Draw playhead
                    if let Some(current_beat) = ui
                        .ctx()
                        .memory(|m| m.data.get_temp::<f64>(egui::Id::new("current_beat")))
                    {
                        let x = grid_left_roll
                            + (current_beat as f32 * self.piano_roll.zoom_x
                                - self.piano_roll.scroll_x);

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
                },
            );

            self.handle_touch_pan_zoom(ui.ctx(), roll_rect, "roll");

            // Velocity lane
            if self.show_velocity_lane {
                let (lane_resp, _) = ui.allocate_painter(
                    egui::vec2(total_w, self.velocity_lane_height),
                    egui::Sense::click_and_drag(),
                );
                let lane_top = roll_rect.bottom();
                let lane_rect = egui::Rect::from_min_size(
                    egui::pos2(roll_rect.left(), lane_top),
                    egui::vec2(total_w, self.velocity_lane_height),
                ); // Position velocity lane properly below piano roll

                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(lane_rect)
                        .sense(Sense::click_and_drag()),
                    |ui| {
                        self.draw_velocity_lane(ui, lane_rect, app);
                    },
                );

                if lane_resp.hovered() {
                    let scroll_delta = ui.input(|i| i.raw_scroll_delta);
                    if ui.input(|i| i.modifiers.ctrl) {
                        let old = self.piano_roll.zoom_x;
                        self.piano_roll.zoom_x = (self.piano_roll.zoom_x
                            * (1.0 + scroll_delta.y * 0.01))
                            .clamp(10.0, 500.0);
                        if (self.piano_roll.zoom_x - old).abs() > f32::EPSILON
                            && let Some(pos) = lane_resp.hover_pos()
                        {
                            let grid_left = lane_rect.left() + crate::constants::PIANO_KEY_WIDTH;
                            let cx = (pos.x - grid_left + self.piano_roll.scroll_x) / old;
                            self.piano_roll.scroll_x =
                                (cx * self.piano_roll.zoom_x - (pos.x - grid_left)).max(0.0);
                        }
                    } else {
                        self.piano_roll.scroll_x =
                            (self.piano_roll.scroll_x - scroll_delta.x).max(0.0);
                    }
                }

                self.handle_touch_pan_zoom(ui.ctx(), roll_rect, "roll");
            }
        });
    }

    pub fn set_editing_clip(&mut self, clip_id: u64) {
        self.selected_clip = Some(clip_id);
        self.piano_roll.selected_note_ids.clear();
        self.piano_roll.temp_selected_indices.clear();
    }

    fn draw_piano_roll(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        if self.selected_clip.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select or create a MIDI clip to edit");
            });
            return;
        }
        let clip_id = self.selected_clip.unwrap();

        // Resolve notes and clip length for the view (pattern-first)
        let (clip_length, current_notes) = {
            let state = app.state.lock().unwrap();
            let clip_opt = state.find_clip(clip_id);
            match clip_opt {
                Some((track, crate::project::ClipLocation::Midi(idx))) => {
                    if let Some(clip) = track.midi_clips.get(idx) {
                        let notes = if let Some(pid) = clip.pattern_id {
                            state
                                .patterns
                                .get(&pid)
                                .map(|p| p.notes.clone())
                                .unwrap_or_default()
                        } else {
                            clip.notes.clone()
                        };
                        (clip.length_beats, notes)
                    } else {
                        self.selected_clip = None;
                        self.piano_roll.selected_note_ids.clear();
                        self.piano_roll.temp_selected_indices.clear();
                        return;
                    }
                }
                _ => {
                    self.selected_clip = None;
                    self.piano_roll.selected_note_ids.clear();
                    self.piano_roll.temp_selected_indices.clear();
                    return;
                }
            }
        };

        // Draw and interact
        let actions = self.piano_roll.ui(
            ui,
            &crate::model::clip::MidiClip {
                length_beats: clip_length,
                notes: current_notes.clone(),
                ..Default::default()
            },
            self.tool_mode == super::piano_roll_view::ToolMode::Draw,
        );

        // Separate preview and mutations
        let mut preview_actions = Vec::new();
        let mut mutation_actions = Vec::new();
        for a in actions {
            match a {
                super::piano_roll::PianoRollAction::PreviewNote(_)
                | super::piano_roll::PianoRollAction::StopPreview => preview_actions.push(a),
                _ => mutation_actions.push(a),
            }
        }

        // Previews
        for action in preview_actions {
            match action {
                super::piano_roll::PianoRollAction::PreviewNote(pitch) => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::PreviewNote(
                            app.selected_track,
                            pitch,
                        ));
                }
                super::piano_roll::PianoRollAction::StopPreview => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::StopPreviewNote);
                }
                _ => {}
            }
        }

        if mutation_actions.is_empty() {
            return;
        }

        // Map an index shown in the piano roll back to the underlying note id
        let id_at_index =
            |i: usize| -> Option<u64> { current_notes.get(i).map(|n| n.id).filter(|&id| id != 0) };

        // Translate UI actions into ID-based commands
        for action in mutation_actions {
            match action {
                super::piano_roll::PianoRollAction::AddNote(mut note) => {
                    // New notes may come with id=0; processor will assign
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::AddNotesToClip {
                            clip_id,
                            notes: vec![note],
                        });
                }
                super::piano_roll::PianoRollAction::RemoveNote(idx) => {
                    if let Some(id) = id_at_index(idx) {
                        let _ =
                            app.command_tx
                                .send(crate::messages::AudioCommand::RemoveNotesById {
                                    clip_id,
                                    note_ids: vec![id],
                                });
                    }
                }
                super::piano_roll::PianoRollAction::UpdateNote(idx, mut updated) => {
                    // Use the original note's ID to update the correct pattern entry
                    if let Some(id) = id_at_index(idx) {
                        updated.id = id;
                        let _ =
                            app.command_tx
                                .send(crate::messages::AudioCommand::UpdateNotesById {
                                    clip_id,
                                    notes: vec![updated],
                                });
                    }
                }
                super::piano_roll::PianoRollAction::DuplicateNotesAndSelect {
                    original_notes: _,
                    drag_offset_beats,
                    drag_offset_semitones,
                } => {
                    // Use the current selection by id (preferred). If empty, derive from temp indices.
                    let mut ids = self.piano_roll.selected_note_ids.clone();
                    if ids.is_empty() {
                        for i in &self.piano_roll.temp_selected_indices {
                            if let Some(id) = id_at_index(*i) {
                                ids.push(id);
                            }
                        }
                    }
                    if !ids.is_empty() {
                        let _ = app.command_tx.send(
                            crate::messages::AudioCommand::DuplicateNotesWithOffset {
                                clip_id,
                                source_note_ids: ids,
                                delta_beats: drag_offset_beats,
                                delta_semitones: drag_offset_semitones,
                            },
                        );
                    }
                }
                super::piano_roll::PianoRollAction::AddNotes(notes) => {
                    let _ = app
                        .command_tx
                        .send(crate::messages::AudioCommand::AddNotesToClip { clip_id, notes });
                }
                super::piano_roll::PianoRollAction::RemoveNotes(indices) => {
                    let ids: Vec<u64> =
                        indices.into_iter().filter_map(|i| id_at_index(i)).collect();
                    if !ids.is_empty() {
                        let _ =
                            app.command_tx
                                .send(crate::messages::AudioCommand::RemoveNotesById {
                                    clip_id,
                                    note_ids: ids,
                                });
                    }
                }
                // Ignored here: PreviewNote/StopPreview handled above
                _ => {}
            }
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        egui::ScrollArea::horizontal()
            .id_salt("pr_tool_strip")
            .scroll_source(ScrollSource::ALL)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Piano Roll");
                    ui.separator();

                    ui.label("MIDI Clip:");

                    let (clip_names, selected_text, create_clip_data, can_delete) = {
                        let state = app.state.lock().unwrap();
                        if let Some(track) = state.tracks.get(&app.selected_track) {
                            let clip_names: Vec<String> =
                                track.midi_clips.iter().map(|c| c.name.clone()).collect();

                            let selected_text = if let Some(clip_id) = self.selected_clip {
                                track
                                    .midi_clips
                                    .iter()
                                    .find(|c| c.id == clip_id)
                                    .map(|c| c.name.clone())
                                    .unwrap_or_else(|| "No Clip".to_string())
                            } else {
                                "No Clip Selected".to_string()
                            };

                            let create_clip_data =
                                Some(state.position_to_beats(app.audio_state.get_position()));

                            let can_delete = self.selected_clip.is_some();

                            (clip_names, selected_text, create_clip_data, can_delete)
                        } else {
                            (vec![], "No Track".to_string(), None, false)
                        }
                    };

                    let (clip_list, selected_name) = {
                        let state = app.state.lock().unwrap();
                        if let Some(track) = state.tracks.get(&app.selected_track) {
                            let clips: Vec<(u64, String)> = track
                                .midi_clips
                                .iter()
                                .map(|c| (c.id, c.name.clone()))
                                .collect();

                            let selected = self
                                .selected_clip
                                .and_then(|id| clips.iter().find(|(cid, _)| *cid == id))
                                .map(|(_, name)| name.clone())
                                .unwrap_or_else(|| "No Clip".to_string());

                            (clips, selected)
                        } else {
                            println!("No track or clip selected...");
                            return;
                        }
                    };

                    egui::ComboBox::from_id_salt("clip_selector")
                        .selected_text(selected_name)
                        .show_ui(ui, |ui| {
                            for (clip_id, name) in clip_list {
                                if ui
                                    .selectable_value(&mut self.selected_clip, Some(clip_id), &name)
                                    .clicked()
                                {
                                    self.piano_roll.selected_note_ids.clear();
                                    self.piano_roll.temp_selected_indices.clear();
                                }
                            }
                        });

                    if ui
                        .button("➕")
                        .on_hover_text("Create New MIDI Clip")
                        .clicked()
                    {
                        let (playhead_beat, last_clip_end) = {
                            let state = app.state.lock().unwrap();
                            let playhead = state.position_to_beats(app.audio_state.get_position());

                            let last_end = state
                                .tracks
                                .get(&app.selected_track)
                                .map(|track| {
                                    track
                                        .midi_clips
                                        .iter()
                                        .map(|c| c.start_beat + c.length_beats)
                                        .fold(0.0f64, f64::max)
                                })
                                .unwrap_or(0.0);

                            (playhead, last_end)
                        };

                        let start_beat = if playhead_beat.round() >= last_clip_end {
                            playhead_beat.round()
                        } else {
                            last_clip_end
                        };

                        let _ = app.command_tx.send(AudioCommand::CreateMidiClip {
                            track_id: app.selected_track,
                            start_beat,
                            length_beats: DEFAULT_MIDI_CLIP_LEN,
                        });
                    }

                    // Duplicate clip
                    if ui.button("⎘").on_hover_text("Duplicate Clip").clicked()
                        && let Some(clip_id) = self.selected_clip
                    {
                        let _ = app
                            .command_tx
                            .send(AudioCommand::DuplicateMidiClip { clip_id });
                    }

                    // Delete clip
                    if can_delete
                        && ui.button("🗑").on_hover_text("Delete Clip").clicked()
                        && let Some(clip_id) = self.selected_clip
                    {
                        let _ = app
                            .command_tx
                            .send(AudioCommand::DeleteMidiClip { clip_id });
                        self.selected_clip = None;
                    }

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

    fn draw_velocity_lane(
        &mut self,
        ui: &mut egui::Ui,
        lane_rect: egui::Rect,
        app: &mut super::app::YadawApp,
    ) {
        let painter = ui.painter_at(lane_rect);
        painter.rect_filled(lane_rect, 0.0, egui::Color32::from_gray(15));

        let clip_id = match self.selected_clip {
            Some(id) => id,
            None => return,
        };

        // Resolve notes (pattern-first) and length once (read-only)
        let (notes, clip_len) = {
            let st = app.state.lock().unwrap();
            match st
                .tracks
                .get(&app.selected_track)
                .and_then(|t| t.midi_clips.iter().find(|c| c.id == clip_id))
            {
                Some(clip) => {
                    let ns = if let Some(pid) = clip.pattern_id {
                        st.patterns
                            .get(&pid)
                            .map(|p| p.notes.clone())
                            .unwrap_or_default()
                    } else {
                        clip.notes.clone()
                    };
                    (ns, clip.length_beats)
                }
                None => return,
            }
        };

        // Layout
        let grid_left = lane_rect.left() + crate::constants::PIANO_KEY_WIDTH;
        let gutter =
            egui::Rect::from_min_max(lane_rect.min, egui::pos2(grid_left, lane_rect.bottom()));
        painter.rect_filled(gutter, 0.0, egui::Color32::from_gray(10));

        // Horizontal guides
        for i in 0..=4 {
            let y = lane_rect.top() + (i as f32 / 4.0) * lane_rect.height();
            painter.line_segment(
                [egui::pos2(grid_left, y), egui::pos2(lane_rect.right(), y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
            );
        }

        // Utility: map screen x back to beats
        let beat_from_x = |x: f32| -> f64 {
            ((x - (lane_rect.left() + crate::constants::PIANO_KEY_WIDTH)) as f64
                + self.piano_roll.scroll_x as f64)
                / self.piano_roll.zoom_x as f64
        };

        // Build UpdateNotesById payload lazily (to avoid frequent sends)
        let mut pending_updates: Vec<crate::model::clip::MidiNote> = Vec::new();

        for (i, note) in notes.iter().enumerate() {
            // Compute bar rect for this note
            let x =
                grid_left + (note.start as f32 * self.piano_roll.zoom_x - self.piano_roll.scroll_x);
            let w = (note.duration as f32 * self.piano_roll.zoom_x).max(2.0);

            let left = x.max(grid_left);
            let right = (x + w).min(lane_rect.right());
            if right <= left {
                continue;
            }

            let h = (note.velocity as f32 / 127.0) * lane_rect.height();
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(left, lane_rect.bottom() - h),
                egui::vec2(right - left, h),
            );

            // Color: highlight if selected by ID
            let selected = self
                .piano_roll
                .selected_note_ids
                .iter()
                .any(|&id| id == note.id);
            let color = if selected {
                egui::Color32::from_rgb(100, 150, 255)
            } else {
                egui::Color32::from_rgb(60, 90, 150)
            };

            painter.rect_filled(bar_rect, 0.0, color);

            // Drag to change velocity
            let resp = ui.interact(
                bar_rect,
                ui.id().with(("velocity", clip_id, note.id)),
                egui::Sense::click_and_drag(),
            );

            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let mut new_vel = ((lane_rect.bottom() - pos.y) / lane_rect.height() * 127.0)
                        .round()
                        .clamp(1.0, 127.0) as u8;

                    if new_vel != note.velocity {
                        // Build an update note struct (only fields used will be applied)
                        let mut up = *note;
                        up.velocity = new_vel;
                        pending_updates.push(up);
                    }
                }
            }
        }

        if !pending_updates.is_empty() {
            let _ = app
                .command_tx
                .send(crate::messages::AudioCommand::UpdateNotesById {
                    clip_id,
                    notes: pending_updates,
                });
        }
    }

    fn handle_touch_pan_zoom(&mut self, ctx: &egui::Context, region: egui::Rect, id_salt: &str) {
        let id_centroid = egui::Id::new(("pr_gesture", id_salt, "centroid"));
        let id_dist = egui::Id::new(("pr_gesture", id_salt, "dist"));

        // Gather touch points inside region only
        let points: Vec<egui::Pos2> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Touch { pos, phase, .. } = e {
                        match phase {
                            egui::TouchPhase::Start | egui::TouchPhase::Move => {
                                if region.contains(*pos) {
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

        if points.len() < 2 {
            ctx.memory_mut(|m| {
                m.data.remove::<egui::Pos2>(id_centroid);
                m.data.remove::<f32>(id_dist);
            });
            return;
        }

        if points.len() >= 2 {
            let centroid = egui::pos2(
                (points[0].x + points[1].x) * 0.5,
                (points[0].y + points[1].y) * 0.5,
            );
            let dist = (points[0] - points[1]).length();

            let (prev_centroid, prev_dist) = ctx.memory(|m| {
                (
                    m.data.get_temp::<egui::Pos2>(id_centroid),
                    m.data.get_temp::<f32>(id_dist),
                )
            });

            if let (Some(pc), Some(pd)) = (prev_centroid, prev_dist) {
                // Pan by delta in region space
                let delta = centroid - pc;
                self.piano_roll.scroll_x = (self.piano_roll.scroll_x - delta.x).max(0.0);
                if id_salt == "roll" {
                    self.piano_roll.scroll_y = (self.piano_roll.scroll_y - delta.y).max(0.0);
                }

                // Pinch zoom horizontally around centroid
                if pd > 1.0 {
                    let scale = (dist / pd).clamp(0.5, 2.0);
                    let old_zoom_x = self.piano_roll.zoom_x;
                    self.piano_roll.zoom_x = (self.piano_roll.zoom_x * scale).clamp(10.0, 500.0);

                    if (self.piano_roll.zoom_x - old_zoom_x).abs() > f32::EPSILON {
                        let grid_left = region.left() + crate::constants::PIANO_KEY_WIDTH;
                        let cx = (centroid.x - grid_left + self.piano_roll.scroll_x) / old_zoom_x;
                        self.piano_roll.scroll_x =
                            (cx * self.piano_roll.zoom_x - (centroid.x - grid_left)).max(0.0);
                    }
                }
            }

            ctx.memory_mut(|m| {
                m.data.insert_temp(id_centroid, centroid);
                m.data.insert_temp(id_dist, dist);
            });
        } else {
            ctx.memory_mut(|m| {
                m.data.remove::<egui::Pos2>(id_centroid);
                m.data.remove::<f32>(id_dist);
            });
        }
    }

    pub fn copy_selected_notes(
        &self,
        state: &Arc<Mutex<AppState>>,
        selected_track: u64,
    ) -> Option<Vec<MidiNote>> {
        let clip_id = self.selected_clip?;

        let state_guard = state.lock().unwrap();
        let track = state_guard.tracks.get(&selected_track)?;
        let clip = track.midi_clips.iter().find(|c| c.id == clip_id)?;

        // Prefer pattern notes if this is an alias; fall back to clip-local notes.
        let base_notes: &[MidiNote] = if let Some(pid) = clip.pattern_id {
            state_guard
                .patterns
                .get(&pid)
                .map(|p| p.notes.as_slice())
                .unwrap_or_else(|| clip.notes.as_slice())
        } else {
            clip.notes.as_slice()
        };

        if base_notes.is_empty() {
            return None;
        }

        // Use the selected note IDs from the piano roll; this is stable for pattern clips.
        if self.piano_roll.selected_note_ids.is_empty() {
            return None;
        }

        let mut selected: Vec<MidiNote> = base_notes
            .iter()
            .filter(|n| n.id != 0 && self.piano_roll.selected_note_ids.contains(&n.id))
            .cloned()
            .collect();

        if selected.is_empty() {
            return None;
        }

        let min_start = selected
            .iter()
            .map(|n| n.start)
            .fold(f64::INFINITY, f64::min);

        for n in &mut selected {
            n.start = (n.start - min_start).max(0.0);
        }

        Some(selected)
    }

    pub fn cut_selected_notes(&mut self, command_tx: &Sender<AudioCommand>) {
        let clip_id = match self.selected_clip {
            Some(id) => id,
            None => return,
        };
        let note_ids = self.piano_roll.selected_note_ids.clone();
        if note_ids.is_empty() {
            return;
        }

        self.piano_roll.selected_note_ids.clear();
        self.piano_roll.temp_selected_indices.clear();

        let _ = command_tx.send(AudioCommand::CutSelectedNotes { clip_id, note_ids });
    }

    pub fn paste_notes(
        &self,
        audio_state: &Arc<AudioState>,
        command_tx: &Sender<AudioCommand>,
        clipboard: &[MidiNote],
    ) {
        let clip_id = match self.selected_clip {
            Some(id) => id,
            None => return,
        };
        if clipboard.is_empty() {
            return;
        }

        let target_beat = audio_state.get_position();
        let sample_rate = audio_state.sample_rate.load() as f64;
        let bpm = audio_state.bpm.load() as f64;
        let target = (target_beat / sample_rate) * (bpm / 60.0);

        let snap = self.piano_roll.grid_snap as f64;
        let snapped_target = if snap > 0.0 {
            ((target / snap).round() * snap).max(0.0)
        } else {
            target.max(0.0)
        };

        // Prepare notes with new start times but ID 0 (processor will assign)
        let notes_to_paste: Vec<MidiNote> = clipboard
            .iter()
            .map(|note| {
                let mut new_note = *note;
                new_note.id = 0; // The command processor will assign a fresh ID
                new_note.start = (snapped_target + new_note.start).max(0.0);
                new_note
            })
            .collect();

        let _ = command_tx.send(AudioCommand::PasteNotes {
            clip_id,
            notes: notes_to_paste,
        });
    }

    pub fn delete_selected_notes(&mut self, command_tx: &Sender<AudioCommand>) -> bool {
        let clip_id = match self.selected_clip {
            Some(id) => id,
            None => return false,
        };
        let note_ids = self.piano_roll.selected_note_ids.clone();
        if note_ids.is_empty() {
            return false;
        }

        self.piano_roll.selected_note_ids.clear();
        self.piano_roll.temp_selected_indices.clear();

        let _ = command_tx.send(AudioCommand::DeleteSelectedNotes { clip_id, note_ids });
        true
    }

    pub fn select_all_notes(&mut self, state: &Arc<Mutex<AppState>>, selected_track: u64) {
        let clip_id = match self.selected_clip {
            Some(id) => id,
            None => return,
        };

        let state_guard = state.lock().unwrap();
        if let Some(track) = state_guard.tracks.get(&selected_track)
            && let Some(clip) = track.midi_clips.iter().find(|c| c.id == clip_id)
        {
            self.piano_roll.selected_note_ids.clear();
            self.piano_roll.temp_selected_indices.clear();

            // pattern notes for aliases, then fallback
            let source_notes: &[MidiNote] = if let Some(pid) = clip.pattern_id {
                state_guard
                    .patterns
                    .get(&pid)
                    .map(|p| p.notes.as_slice())
                    .unwrap_or_else(|| clip.notes.as_slice())
            } else {
                clip.notes.as_slice()
            };

            for (i, note) in source_notes.iter().enumerate() {
                if note.id != 0 {
                    self.piano_roll.selected_note_ids.push(note.id);
                } else {
                    self.piano_roll.temp_selected_indices.push(i);
                }
            }
        }
    }
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self::new()
    }
}
