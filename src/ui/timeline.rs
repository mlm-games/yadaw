use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::*;
use crate::constants::{DEFAULT_MIDI_CLIP_LEN, DEFAULT_MIN_PROJECT_BEATS, MAGNETIC_SNAP_THRESHOLD};
use crate::messages::AudioCommand;
use crate::model::{AudioClip, AutomationTarget, MidiClip, Track};
use crate::project::ClipLocation;
use crate::ui::automation_lane::{AutomationAction, AutomationLaneWidget};
use egui::scroll_area::ScrollSource;

pub struct TimelineView {
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub grid_snap: f32,
    pub show_automation: bool,
    pub auto_scroll: bool,

    timeline_interaction: Option<TimelineInteraction>,
    automation_widgets: Vec<AutomationLaneWidget>,

    show_clip_menu: bool,
    clip_menu_pos: egui::Pos2,

    track_height: f32,
    min_track_height: f32,
    max_track_height: f32,

    last_view_width: f32,
    pending_clip_undo: bool,

    automation_hit_regions: Vec<egui::Rect>,
    last_track_blocks: Vec<(u64, egui::Rect)>,
}

#[derive(Clone)]
enum TimelineInteraction {
    DragClip {
        clip_ids_and_starts: Vec<(u64, f64)>,
        start_drag_beat: f64,
    },
    ResizeClipLeft {
        clip_id: u64,
        original_end_beat: f64,
    },
    ResizeClipRight {
        clip_id: u64,
        original_start_beat: f64,
    },
    SelectionBox {
        start_pos: egui::Pos2,
        current_pos: egui::Pos2,
    },
    LoopDragStart {
        offset_beats: f64,
    },
    LoopDragEnd {
        offset_beats: f64,
    },
    LoopCreate {
        anchor_beat: f64,
    },
    SlipContent {
        clip_id: u64,
        start_offset: f64,
        start_mouse_beat: f64,
    },
}

impl TimelineView {
    pub fn new() -> Self {
        Self {
            zoom_x: 100.0,
            zoom_y: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            grid_snap: 0.25,
            show_automation: false,
            auto_scroll: true,

            timeline_interaction: None,
            automation_widgets: Vec::new(),

            show_clip_menu: false,
            clip_menu_pos: egui::Pos2::ZERO,

            track_height: 80.0,
            min_track_height: 40.0,
            max_track_height: 200.0,
            last_view_width: 800.0,
            pending_clip_undo: false,
            automation_hit_regions: Vec::new(),
            last_track_blocks: Vec::new(),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.heading("Timeline");
        self.draw_toolbar(ui, app);
        ui.separator();

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_timeline(ui, app);
            });

        self.draw_context_menus(ui, app);

        if self.auto_scroll && app.audio_state.playing.load(Ordering::Relaxed) {
            self.update_auto_scroll(app);
        }
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, app: &super::app::YadawApp) {
        egui::ScrollArea::horizontal()
            .id_salt("tl_tool_strip")
            .scroll_source(ScrollSource::ALL)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Zoom:");
                    if ui.button("−").clicked() {
                        self.zoom_x = (self.zoom_x * 0.8).max(10.0);
                    }
                    if ui.button("╋").clicked() {
                        self.zoom_x = (self.zoom_x * 1.25).min(500.0);
                    }
                    ui.label(format!("{:.0}px/beat", self.zoom_x));

                    ui.separator();

                    ui.label("Track Height:");
                    ui.add(
                        egui::Slider::new(
                            &mut self.track_height,
                            self.min_track_height..=self.max_track_height,
                        )
                        .show_value(false),
                    );

                    ui.separator();

                    ui.label("Snap:");
                    egui::ComboBox::from_label("")
                        .selected_text(format!("1/{}", (1.0 / self.grid_snap) as i32))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.grid_snap, 1.0, "1/1");
                            ui.selectable_value(&mut self.grid_snap, 0.5, "1/2");
                            ui.selectable_value(&mut self.grid_snap, 0.25, "1/4");
                            ui.selectable_value(&mut self.grid_snap, 0.125, "1/8");
                            ui.selectable_value(&mut self.grid_snap, 0.0625, "1/16");
                            ui.selectable_value(&mut self.grid_snap, 0.03125, "1/32");
                        });

                    ui.separator();
                    ui.checkbox(&mut self.show_automation, "Show Automation");
                    ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                });
            });
    }

    fn draw_timeline(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        self.automation_hit_regions.clear();
        self.last_track_blocks.clear();

        if self.pending_clip_undo {
            self.pending_clip_undo = false;
        }

        let track_data: Vec<(u64, Track)> = {
            let state = app.state.lock().unwrap();
            state
                .track_order
                .iter()
                .filter_map(|&tid| state.tracks.get(&tid).cloned().map(|t| (tid, t)))
                .collect()
        };

        // Compute end of project in beats (for horizontal size)
        let project_end_beats = self
            .compute_project_end_beats(app)
            .max(DEFAULT_MIN_PROJECT_BEATS);
        let margin_beats = 8.0;
        let content_beats = project_end_beats + margin_beats;

        // Visible width and full content width
        let view_w = ui.available_width();
        self.last_view_width = view_w;
        let total_width = view_w.max(content_beats as f32 * self.zoom_x);

        // Compute dynamic per-track height: base track + visible lanes (if show_automation)
        let track_heights: Vec<f32> = track_data
            .iter()
            .map(|(_, t)| {
                if self.show_automation {
                    let extra: f32 = t
                        .automation_lanes
                        .iter()
                        .filter(|l| l.visible)
                        .map(|l| l.height.max(20.0))
                        .sum();
                    self.track_height + extra
                } else {
                    self.track_height
                }
            })
            .collect();

        let total_height: f32 = track_heights.iter().copied().sum::<f32>().max(1.0);

        // Allocate the full drawing surface now that we know height
        let (response, painter) = ui.allocate_painter(
            egui::vec2(total_width, total_height),
            egui::Sense::click_and_drag(),
        );

        // Let the clip editor own the “active edit target” when hovered
        if response.hovered() || response.is_pointer_button_down_on() {
            app.active_edit_target = super::app::ActiveEditTarget::Clips;
        }

        // Draw the grid and horizontal ruler
        let rect = response.rect;
        self.draw_grid(&painter, rect, app.state.lock().unwrap().bpm);

        // Place each track block at cumulative Y positions
        let mut y_cursor = rect.top();

        for ((track_id, track), block_h) in track_data.iter().zip(track_heights.iter()) {
            // The main clip area is the top self.track_height of the block
            let clip_area = egui::Rect::from_min_size(
                egui::pos2(rect.left(), y_cursor),
                egui::vec2(rect.width(), self.track_height),
            );

            // Track background and name
            self.draw_track(&painter, ui, clip_area, &track, *track_id, app);

            let sep_color = ui.visuals().widgets.noninteractive.fg_stroke.color;
            painter.line_segment(
                [
                    egui::pos2(rect.left(), y_cursor),
                    egui::pos2(rect.right(), y_cursor),
                ],
                egui::Stroke::new(1.0, sep_color),
            );

            let block_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left(), y_cursor),
                egui::pos2(rect.right(), y_cursor + *block_h),
            );

            // Cache it for hit mapping (Ctrl+click etc.)
            self.last_track_blocks.push((*track_id, block_rect));

            if self.show_automation {
                let mut track_clone = track.clone();
                self.draw_automation_lanes(ui, block_rect, &mut track_clone, *track_id, app);
            }

            // Draw separator at the BOTTOM of the block (not the top)
            let sep_y = y_cursor + *block_h;
            let sep_color = ui.visuals().widgets.noninteractive.fg_stroke.color;
            painter.line_segment(
                [
                    egui::pos2(rect.left(), sep_y),
                    egui::pos2(rect.right(), sep_y),
                ],
                egui::Stroke::new(1.0, sep_color),
            );

            y_cursor += *block_h;
        }

        // Draw loop region overlay
        self.draw_loop_region(&painter, rect, app);

        // Draw playhead
        let position = app.audio_state.get_position();
        let sample_rate = app.audio_state.sample_rate.load();
        let bpm = app.audio_state.bpm.load();
        if sample_rate > 0.0 && bpm > 0.0 {
            let current_beat = (position / sample_rate as f64) * (bpm as f64 / 60.0);
            let x = rect.left() + (current_beat as f32 * self.zoom_x - self.scroll_x);
            if x >= rect.left() && x <= rect.right() {
                ui.ctx().debug_painter().line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(2.0, crate::constants::COLOR_PLAYHEAD),
                );
            }
        }

        if self.pending_clip_undo {
            app.push_undo();
            self.pending_clip_undo = false;
        }

        // Parent-level pointer handling (seek, loop bars, drag clips).
        // If the pointer is over any lane rect, the automation widgets own it and we return early.
        self.handle_timeline_interaction(&response, ui, app);
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: egui::Rect, _bpm: f32) {
        let ruler_h = 18.0;
        let visuals = painter.ctx().style().visuals.clone();
        let bg = visuals.widgets.noninteractive.bg_fill;
        let grid_fg = visuals.widgets.noninteractive.fg_stroke.color;
        let bar_fg =
            egui::Color32::from_rgba_premultiplied(grid_fg.r(), grid_fg.g(), grid_fg.b(), 220);

        // Top ruler background
        painter.rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + ruler_h)),
            0.0,
            bg,
        );

        // Vertical beat lines and bar markers
        let beats_visible = (rect.width() / self.zoom_x) as i32 + 2;
        let start_beat = (self.scroll_x / self.zoom_x) as i32;

        for beat in start_beat..(start_beat + beats_visible) {
            let x = rect.left() + (beat as f32 * self.zoom_x - self.scroll_x);
            if x < rect.left() || x > rect.right() {
                continue;
            }
            let is_bar = beat % 4 == 0;
            let color = if is_bar { bar_fg } else { grid_fg };
            painter.line_segment(
                [
                    egui::pos2(x, rect.top()),
                    egui::pos2(x, rect.top() + ruler_h),
                ],
                egui::Stroke::new(if is_bar { 1.5 } else { 1.0 }, color),
            );
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + ruler_h),
                    egui::pos2(x, rect.bottom()),
                ],
                egui::Stroke::new(1.0, grid_fg),
            );
        }
    }

    fn draw_track(
        &mut self,
        painter: &egui::Painter,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        track: &Track,
        track_id: u64,
        app: &mut super::app::YadawApp,
    ) {
        let vis = ui.visuals();
        let base = vis.extreme_bg_color;
        let bg_color = if rand::random_bool(0.5) {
            // TODO: make all of them colorful like audacity 4
            base
        } else {
            egui::Color32::from_rgba_premultiplied(
                ((base.r() as f32) * 0.93) as u8,
                ((base.g() as f32) * 0.93) as u8,
                ((base.b() as f32) * 0.93) as u8,
                base.a(),
            )
        };
        painter.rect_filled(rect, 0.0, bg_color);

        painter.text(
            rect.min + egui::vec2(5.0, 5.0),
            egui::Align2::LEFT_TOP,
            &track.name,
            egui::FontId::default(),
            egui::Color32::WHITE,
        );

        if track.is_midi {
            for clip in &track.midi_clips {
                self.draw_midi_clip(painter, ui, rect, clip, track_id, app);
            }
        } else {
            for clip in &track.audio_clips {
                self.draw_audio_clip(painter, ui, rect, clip, track_id, app);
            }
        }
    }

    fn draw_audio_clip(
        &mut self,
        painter: &egui::Painter,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        clip: &AudioClip,
        _track_id: u64,
        app: &mut super::app::YadawApp,
    ) {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_width = clip.length_beats as f32 * self.zoom_x;

        let clip_rect = egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 20.0),
            egui::vec2(clip_width, self.track_height - 25.0),
        );

        if clip_rect.right() < track_rect.left() || clip_rect.left() > track_rect.right() {
            return;
        }

        crate::waveform::draw_waveform(painter, clip_rect, clip, self.zoom_x, self.scroll_x);

        if app.selected_clips.contains(&clip.id) {
            painter.rect_stroke(
                clip_rect,
                2.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }

        let response = ui.interact(
            clip_rect,
            ui.id().with(("audio_clip", clip.id)),
            egui::Sense::click_and_drag(),
        );

        self.handle_clip_interaction(response, clip.id, ui, clip_rect, app);
    }

    fn draw_loop_region(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        app: &super::app::YadawApp,
    ) {
        if !app.audio_state.loop_enabled.load(Ordering::Relaxed) {
            return;
        }
        let loop_start = app.audio_state.loop_start.load();
        let loop_end = app.audio_state.loop_end.load();
        if !(loop_end > loop_start) {
            return;
        }

        let start_x = rect.left() + (loop_start as f32 * self.zoom_x - self.scroll_x);
        let end_x = rect.left() + (loop_end as f32 * self.zoom_x - self.scroll_x);

        if end_x <= rect.left() || start_x >= rect.right() {
            return;
        }

        painter.line_segment(
            [
                egui::pos2(start_x, rect.top()),
                egui::pos2(start_x, rect.bottom()),
            ],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
        );
        painter.line_segment(
            [
                egui::pos2(end_x, rect.top()),
                egui::pos2(end_x, rect.bottom()),
            ],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
        );
    }

    fn draw_midi_clip(
        &mut self,
        painter: &egui::Painter,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        clip: &MidiClip,
        track_id: u64,
        app: &mut super::app::YadawApp,
    ) {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_width = clip.length_beats as f32 * self.zoom_x;

        let clip_rect = egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 5.0),
            egui::vec2(clip_width, self.track_height - 10.0),
        );

        if clip_rect.right() < track_rect.left() || clip_rect.left() > track_rect.right() {
            return;
        }

        let clip_fill = if let Some((r, g, b)) = clip.color {
            egui::Color32::from_rgba_premultiplied(r, g, b, 196)
        } else {
            egui::Color32::from_rgba_premultiplied(100, 150, 200, 196)
        };
        painter.rect_filled(clip_rect, 4.0, clip_fill);

        painter.rect_stroke(
            clip_rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(180)),
            egui::StrokeKind::Middle,
        );

        // Draw MIDI notes preview (using content loop logic)
        let content_len = clip.content_len_beats.max(0.000001);
        let inst_len = clip.length_beats.max(0.0);
        let clip_left = clip_rect.left();

        let vis_start_rel: f64 =
            ((track_rect.left() - clip_left) as f64 + self.scroll_x as f64) / self.zoom_x as f64;
        let vis_end_rel: f64 =
            ((track_rect.right() - clip_left) as f64 + self.scroll_x as f64) / self.zoom_x as f64;

        let first_rep: i32 = if clip.loop_enabled {
            (vis_start_rel / content_len).floor().max(0.0) as i32
        } else {
            0
        };
        let last_rep: i32 = if clip.loop_enabled {
            (vis_end_rel / content_len).ceil().max(0.0) as i32
        } else {
            0
        };

        if clip.loop_enabled {
            let reps = (inst_len / content_len).ceil() as i32;
            for k in 1..reps {
                let x = clip_rect.left() + (k as f32 * content_len as f32 * self.zoom_x);
                if x >= track_rect.left() && x <= track_rect.right() {
                    painter.line_segment(
                        [
                            egui::pos2(x, clip_rect.top()),
                            egui::pos2(x, clip_rect.bottom()),
                        ],
                        egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_premultiplied(255, 255, 255, 40),
                        ),
                    );
                }
            }
        }

        let offset = clip
            .content_offset_beats
            .rem_euclid(clip.content_len_beats.max(0.000001));

        for k in first_rep..=last_rep {
            let rep_start = k as f64 * content_len;
            if rep_start >= inst_len {
                break;
            }

            for note in &clip.notes {
                let s_loc = (note.start + offset).rem_euclid(content_len);
                let e_loc_raw = s_loc + note.duration;
                let mut segs: smallvec::SmallVec<[(f64, f64); 2]> = smallvec::smallvec![];
                if e_loc_raw <= content_len {
                    segs.push((s_loc, e_loc_raw));
                } else {
                    segs.push((s_loc, content_len));
                    segs.push((0.0, e_loc_raw - content_len));
                }

                for (s_local, e_local) in segs {
                    let s = rep_start + s_local;
                    if s >= inst_len {
                        continue;
                    }
                    let e = (rep_start + e_local).min(inst_len);
                    let seg_left = clip_rect.left() + (s as f32 * self.zoom_x);
                    let seg_right = clip_rect.left() + (e as f32 * self.zoom_x);
                    if seg_right < track_rect.left() || seg_left > track_rect.right() {
                        continue;
                    }

                    let note_y =
                        clip_rect.bottom() - ((note.pitch as f32 / 127.0) * clip_rect.height());
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(seg_left, note_y - 2.0),
                            egui::vec2((seg_right - seg_left).max(2.0), 2.0),
                        ),
                        0.0,
                        egui::Color32::from_rgba_premultiplied(255, 255, 255, 100),
                    );
                }
            }
        }

        painter.text(
            clip_rect.min + egui::vec2(5.0, 5.0),
            egui::Align2::LEFT_TOP,
            &clip.name,
            egui::FontId::default(),
            egui::Color32::WHITE,
        );

        let response = ui.interact(
            clip_rect,
            ui.id().with(("midi_clip", clip.id)),
            egui::Sense::click_and_drag(),
        );

        if response.double_clicked() {
            app.selected_track = track_id;
            app.open_midi_clip_in_piano_roll(clip.id);
        }

        self.handle_clip_interaction(response, clip.id, ui, clip_rect, app);
    }

    fn handle_clip_interaction(
        &mut self,
        response: egui::Response,
        clip_id: u64,
        ui: &mut egui::Ui,
        clip_rect: egui::Rect,
        app: &mut super::app::YadawApp,
    ) {
        // Select on click
        if response.clicked() && !app.selected_clips.contains(&clip_id) {
            if !response.ctx.input(|i| i.modifiers.ctrl) {
                app.selected_clips.clear();
            }
            app.selected_clips.push(clip_id);
        }

        // Context menu
        if response.secondary_clicked() {
            self.show_clip_menu = true;
            self.clip_menu_pos = response.interact_pointer_pos().unwrap_or_default();
            if !app.selected_clips.contains(&clip_id) {
                app.selected_clips.clear();
                app.selected_clips.push(clip_id);
            }
        }

        // Edge hover
        let edge_threshold = 5.0;
        let hover_left = response
            .hover_pos()
            .is_some_and(|p| (p.x - clip_rect.left()).abs() < edge_threshold);
        let hover_right = response
            .hover_pos()
            .is_some_and(|p| (clip_rect.right() - p.x).abs() < edge_threshold);

        if hover_left || hover_right {
            response
                .ctx
                .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        // Begin drag/resize
        if response.drag_started() && self.timeline_interaction.is_none() {
            let (clip_start, clip_len) = {
                let state = app.state.lock().unwrap();
                if let Some((track, loc)) = state.find_clip(clip_id) {
                    match loc {
                        crate::project::ClipLocation::Midi(idx) => {
                            if let Some(c) = track.midi_clips.get(idx) {
                                (c.start_beat, c.length_beats)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                        crate::project::ClipLocation::Audio(idx) => {
                            if let Some(c) = track.audio_clips.get(idx) {
                                (c.start_beat, c.length_beats)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                    }
                } else {
                    (0.0, 0.0)
                }
            };

            let beat_at = |x: f32, rect_left: f32| -> f64 {
                let rel = (x - rect_left) + self.scroll_x;
                (rel / self.zoom_x) as f64
            };

            let start_beat_under_mouse = response
                .interact_pointer_pos()
                .map(|pos| beat_at(pos.x, response.rect.left()))
                .unwrap_or(clip_start);

            let alt = ui.input(|i| i.modifiers.alt);

            // Slip content (Alt+drag)
            if alt && !hover_left && !hover_right {
                let (start_offset, content_len) = {
                    let state = app.state.lock().unwrap();
                    if let Some((track, loc)) = state.find_clip(clip_id) {
                        if let crate::project::ClipLocation::Midi(idx) = loc {
                            if let Some(c) = track.midi_clips.get(idx) {
                                (c.content_offset_beats, c.content_len_beats.max(0.000001))
                            } else {
                                (0.0, 1.0)
                            }
                        } else {
                            (0.0, 1.0)
                        }
                    } else {
                        (0.0, 1.0)
                    }
                };

                self.timeline_interaction = Some(TimelineInteraction::SlipContent {
                    clip_id,
                    start_offset: start_offset.rem_euclid(content_len),
                    start_mouse_beat: start_beat_under_mouse,
                });
                return;
            }

            // Resize edges
            if hover_left {
                self.timeline_interaction = Some(TimelineInteraction::ResizeClipLeft {
                    clip_id,
                    original_end_beat: clip_start + clip_len,
                });
                return;
            } else if hover_right {
                self.timeline_interaction = Some(TimelineInteraction::ResizeClipRight {
                    clip_id,
                    original_start_beat: clip_start,
                });
                return;
            } else {
                // Drag whole clip (or multi-selection)
                let mut clips_and_starts = Vec::new();
                let state = app.state.lock().unwrap();

                let selected = if app.selected_clips.contains(&clip_id) {
                    app.selected_clips.clone()
                } else {
                    vec![clip_id]
                };

                for cid in selected {
                    if let Some((track, loc)) = state.find_clip(cid) {
                        match loc {
                            crate::project::ClipLocation::Midi(idx) => {
                                if let Some(c) = track.midi_clips.get(idx) {
                                    clips_and_starts.push((cid, c.start_beat));
                                }
                            }
                            crate::project::ClipLocation::Audio(idx) => {
                                if let Some(c) = track.audio_clips.get(idx) {
                                    clips_and_starts.push((cid, c.start_beat));
                                }
                            }
                        }
                    }
                }
                drop(state);

                self.timeline_interaction = Some(TimelineInteraction::DragClip {
                    clip_ids_and_starts: clips_and_starts,
                    start_drag_beat: start_beat_under_mouse,
                });
            }
        }

        // End interaction
        if response.drag_stopped() {
            self.timeline_interaction = None;
        }
    }

    fn handle_timeline_interaction(
        &mut self,
        response: &egui::Response,
        ui: &mut egui::Ui,
        app: &mut super::app::YadawApp,
    ) {
        if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
            if self.automation_hit_regions.iter().any(|r| r.contains(pos)) {
                return;
            }
        }

        let rect = response.rect;
        let ruler_h = 18.0;

        let beat_at = |x: f32| -> f64 {
            let rel = (x - rect.left()) + self.scroll_x;
            (rel / self.zoom_x) as f64
        };

        let snap = |v: f64, grid: f32| -> f64 {
            if grid > 0.0 {
                let g = grid as f64;
                (v / g).round() * g
            } else {
                v
            }
        };

        let min_len = (self.grid_snap.max(0.03125)) as f64;

        // Start drag
        if response.drag_started()
            && self.timeline_interaction.is_none()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let on_ruler = pos.y >= rect.top() && pos.y <= rect.top() + ruler_h;
            if on_ruler {
                let lb = app.audio_state.loop_start.load();
                let le = app.audio_state.loop_end.load();
                let target = beat_at(pos.x);
                if app.audio_state.loop_enabled.load(Ordering::Relaxed) && (le > lb) {
                    let start_x = rect.left() + (lb as f32 * self.zoom_x - self.scroll_x);
                    let end_x = rect.left() + (le as f32 * self.zoom_x - self.scroll_x);
                    let near = 6.0;
                    if (pos.x - start_x).abs() <= near {
                        self.timeline_interaction = Some(TimelineInteraction::LoopDragStart {
                            offset_beats: target - lb,
                        });
                        return;
                    } else if (pos.x - end_x).abs() <= near {
                        self.timeline_interaction = Some(TimelineInteraction::LoopDragEnd {
                            offset_beats: target - le,
                        });
                        return;
                    }
                }
                self.timeline_interaction = Some(TimelineInteraction::LoopCreate {
                    anchor_beat: target,
                });
                return;
            }
        }

        // DON'T SEND COMMANDS
        if response.dragged()
            && let Some(pos) = response.hover_pos()
        {
            if self.automation_hit_regions.iter().any(|r| r.contains(pos)) {
                return; // for the automation widget handle it
            }
            // Commands cause stuttering
            let _current_beat = beat_at(pos.x);
        }

        // END DRAG
        if response.drag_stopped() {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos())
                && let Some(interaction) = &self.timeline_interaction
            {
                match interaction {
                    TimelineInteraction::DragClip {
                        clip_ids_and_starts,
                        start_drag_beat,
                    } => {
                        let current = beat_at(pos.x);
                        let mut delta = current - *start_drag_beat;
                        delta = snap(delta, self.grid_snap);

                        let mut allow_move = true;
                        app.push_undo();
                        let state = app.state.lock().unwrap();

                        'outer: for (clip_id, original_start) in clip_ids_and_starts.iter().copied()
                        {
                            let new_start = (original_start + delta).max(0.0);

                            if let Some((track, loc)) = state.find_clip(clip_id) {
                                let new_length = match loc {
                                    ClipLocation::Midi(idx) => track.midi_clips[idx].length_beats,
                                    ClipLocation::Audio(idx) => track.audio_clips[idx].length_beats,
                                };
                                let new_end = new_start + new_length;

                                for other in &track.midi_clips {
                                    if other.id == clip_id
                                        || clip_ids_and_starts.iter().any(|(id, _)| *id == other.id)
                                    {
                                        continue; // Don't check against self or other moving clips
                                    }
                                    let other_end = other.start_beat + other.length_beats;
                                    if new_start < other_end && new_end > other.start_beat {
                                        allow_move = false;
                                        break 'outer;
                                    }
                                }
                                // Check against other Audio clips on the same track
                                for other in &track.audio_clips {
                                    if other.id == clip_id
                                        || clip_ids_and_starts.iter().any(|(id, _)| *id == other.id)
                                    {
                                        continue;
                                    }
                                    let other_end = other.start_beat + other.length_beats;
                                    if new_start < other_end && new_end > other.start_beat {
                                        allow_move = false;
                                        break 'outer;
                                    }
                                }
                            }
                        }

                        let mut to_send: Vec<AudioCommand> = Vec::new();
                        if allow_move {
                            for (clip_id, original_start) in clip_ids_and_starts.iter().copied() {
                                let new_start = (original_start + delta).max(0.0);
                                let is_midi = state
                                    .clips_by_id
                                    .get(&clip_id)
                                    .map(|r| r.is_midi)
                                    .unwrap_or(false);

                                if is_midi {
                                    to_send.push(AudioCommand::MoveMidiClip { clip_id, new_start });
                                } else {
                                    to_send
                                        .push(AudioCommand::MoveAudioClip { clip_id, new_start });
                                }
                            }
                        }
                        drop(state);
                        if allow_move {
                            for cmd in to_send {
                                let _ = app.command_tx.send(cmd);
                            }
                        } else {
                            app.dialogs.show_message(
                                "Cannot move clip here: it would overlap with another clip.",
                            );
                        }
                    }

                    TimelineInteraction::ResizeClipLeft {
                        clip_id,
                        original_end_beat,
                    } => {
                        let drag_at = snap(beat_at(pos.x).max(0.0), self.grid_snap);
                        let new_start = drag_at.min(*original_end_beat - min_len);
                        let new_len = (*original_end_beat - new_start).max(min_len);

                        let state = app.state.lock().unwrap();
                        let is_midi = state
                            .clips_by_id
                            .get(clip_id)
                            .map(|r| r.is_midi)
                            .unwrap_or(false);
                        drop(state);

                        if is_midi {
                            let _ = app.command_tx.send(AudioCommand::ResizeMidiClip {
                                clip_id: *clip_id,
                                new_start,
                                new_length: new_len,
                            });
                        } else {
                            let _ = app.command_tx.send(AudioCommand::ResizeAudioClip {
                                clip_id: *clip_id,
                                new_start,
                                new_length: new_len,
                            });
                        }
                        app.push_undo();
                    }

                    TimelineInteraction::ResizeClipRight {
                        clip_id,
                        original_start_beat,
                    } => {
                        let drag_at = snap(beat_at(pos.x).max(0.0), self.grid_snap);
                        let new_end = drag_at.max(*original_start_beat + min_len);
                        let new_len = (new_end - *original_start_beat).max(min_len);

                        let state = app.state.lock().unwrap();
                        let is_midi = state
                            .clips_by_id
                            .get(clip_id)
                            .map(|r| r.is_midi)
                            .unwrap_or(false);
                        drop(state);

                        if is_midi {
                            let _ = app.command_tx.send(AudioCommand::ResizeMidiClip {
                                clip_id: *clip_id,
                                new_start: *original_start_beat,
                                new_length: new_len,
                            });
                        } else {
                            let _ = app.command_tx.send(AudioCommand::ResizeAudioClip {
                                clip_id: *clip_id,
                                new_start: *original_start_beat,
                                new_length: new_len,
                            });
                        }
                        app.push_undo();
                    }

                    TimelineInteraction::SlipContent {
                        clip_id,
                        start_offset,
                        start_mouse_beat,
                    } => {
                        let cur = ((pos.x - rect.left()) + self.scroll_x) / self.zoom_x;
                        let delta = (cur as f64) - *start_mouse_beat;
                        let new_off = *start_offset + delta;

                        let _ = app.command_tx.send(AudioCommand::SetClipContentOffset {
                            clip_id: *clip_id,
                            new_offset: new_off,
                        });
                        app.push_undo();
                    }

                    _ => {}
                }
            }

            self.timeline_interaction = None;
        }

        // Click on ruler to set playhead
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let on_ruler = pos.y >= rect.top() && pos.y <= rect.top() + ruler_h;
            if on_ruler {
                let mut beat = beat_at(pos.x).max(0.0);
                beat = snap(beat, self.grid_snap);
                let sr = app.audio_state.sample_rate.load() as f64;
                let bpm = app.audio_state.bpm.load() as f64;
                if bpm > 0.0 && sr > 0.0 {
                    let samples = beat * (60.0 / bpm) * sr;
                    let _ = app.command_tx.send(AudioCommand::SetPosition(samples));
                }
                return;
            }
        }

        // Ctrl+click to create MIDI clip
        if response.clicked()
            && self.timeline_interaction.is_none()
            && let Some(pos) = response.interact_pointer_pos()
        {
            // Map y position to track using the cached block rects
            let track_id_opt = self
                .last_track_blocks
                .iter()
                .find(|(_, r)| r.contains(pos))
                .map(|(id, _)| *id);

            if let Some(track_id) = track_id_opt
                && ui.input(|i| i.modifiers.ctrl)
            {
                // Only create for MIDI tracks
                let is_midi = {
                    let state = app.state.lock().unwrap();
                    state
                        .tracks
                        .get(&track_id)
                        .map(|t| t.is_midi)
                        .unwrap_or(false)
                };
                if is_midi {
                    let grid_pos = pos - response.rect.min;
                    let mut beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                    beat = if self.grid_snap > 0.0 {
                        ((beat as f64 / self.grid_snap as f64).round() * self.grid_snap as f64)
                            as f32
                    } else {
                        beat
                    };
                    let _ = app.command_tx.send(AudioCommand::CreateMidiClip {
                        track_id,
                        start_beat: beat as f64,
                        length_beats: DEFAULT_MIDI_CLIP_LEN,
                    });
                }
            }
        }
    }

    fn draw_automation_lanes(
        &mut self,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        track: &Track,
        track_id: u64,
        app: &mut super::app::YadawApp,
    ) {
        let visible_lanes: Vec<(usize, f32)> = track
            .automation_lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.visible)
            .map(|(i, l)| (i, l.height.max(20.0)))
            .collect();

        if visible_lanes.is_empty() {
            return;
        }

        let mut y = track_rect.bottom();
        for (lane_idx, h) in visible_lanes.iter().cloned() {
            y -= h;
            let lane_rect = egui::Rect::from_min_size(
                egui::pos2(track_rect.left(), y),
                egui::vec2(track_rect.width(), h),
            );

            while self.automation_widgets.len() <= lane_idx {
                self.automation_widgets.push(AutomationLaneWidget);
            }

            let label_w = 80.0_f32.min(track_rect.width() * 0.25);
            let label_rect = egui::Rect::from_min_size(lane_rect.min, egui::vec2(label_w, h));
            let lane_name = match &track.automation_lanes[lane_idx].parameter {
                AutomationTarget::TrackVolume => "Volume",
                AutomationTarget::TrackPan => "Pan",
                AutomationTarget::TrackSend(_) => "Send",
                AutomationTarget::PluginParam { param_name, .. } => param_name.as_str(),
            };
            ui.painter()
                .rect_filled(label_rect, 0.0, egui::Color32::from_gray(28));
            ui.painter().text(
                label_rect.center(),
                egui::Align2::CENTER_CENTER,
                lane_name,
                egui::FontId::default(),
                egui::Color32::from_gray(200),
            );

            let curve_rect = egui::Rect::from_min_max(
                egui::pos2(lane_rect.left() + label_w + 2.0, lane_rect.top()),
                lane_rect.max,
            );

            self.automation_hit_regions.push(curve_rect);

            let id_ns = ui.id().with(("lane", track_id, lane_idx as u64));

            let actions = self.automation_widgets[lane_idx].ui(
                ui,
                &track.automation_lanes[lane_idx],
                curve_rect,
                self.zoom_x,
                self.scroll_x,
                id_ns,
            );

            let mut pushed_undo_for_move = false;

            for action in actions {
                match action {
                    AutomationAction::AddPoint { beat, value } => {
                        app.push_undo();
                        let target = track.automation_lanes[lane_idx].parameter.clone();
                        let _ = app.command_tx.send(AudioCommand::AddAutomationPoint(
                            track_id, target, beat, value,
                        ));
                    }
                    AutomationAction::RemovePoint(beat) => {
                        app.push_undo();
                        let _ = app.command_tx.send(AudioCommand::RemoveAutomationPoint(
                            track_id, lane_idx, beat,
                        ));
                    }
                    AutomationAction::MovePoint {
                        old_beat,
                        new_beat,
                        new_value,
                    } => {
                        if !pushed_undo_for_move {
                            app.push_undo();
                            pushed_undo_for_move = true;
                        }
                        let _ = app.command_tx.send(AudioCommand::UpdateAutomationPoint {
                            track_id,
                            lane_idx,
                            old_beat,
                            new_beat,
                            new_value,
                        });
                    }
                }
            }
        }
    }

    fn update_auto_scroll(&mut self, app: &super::app::YadawApp) {
        let position = app.audio_state.get_position();
        let sample_rate = app.audio_state.sample_rate.load();
        let bpm = app.audio_state.bpm.load();
        if sample_rate <= 0.0 || bpm <= 0.0 {
            return;
        }

        let current_beat = (position / sample_rate as f64) * (bpm as f64 / 60.0);
        let playhead_x = current_beat as f32 * self.zoom_x;

        let view_w = self.last_view_width.max(200.0);
        let left_margin = view_w * 0.1;
        let right_margin = view_w * 0.2;

        if playhead_x < self.scroll_x + left_margin {
            self.scroll_x = (playhead_x - left_margin).max(0.0);
        } else if playhead_x > self.scroll_x + view_w - right_margin {
            self.scroll_x = playhead_x - (view_w - right_margin);
        }
    }

    fn draw_context_menus(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        if self.show_clip_menu {
            let mut close_menu = false;

            egui::Area::new(ui.id().with("clip_context_menu"))
                .fixed_pos(self.clip_menu_pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(150.0);

                        if ui.button("Cut").clicked() {
                            app.cut_selected();
                            close_menu = true;
                        }

                        if ui.button("Copy").clicked() {
                            app.copy_selected();
                            close_menu = true;
                        }

                        if ui.button("Paste").clicked() {
                            app.paste_at_playhead();
                            close_menu = true;
                        }

                        ui.separator();

                        if ui.button("Split at Playhead").clicked() {
                            app.split_selected_at_playhead();
                            close_menu = true;
                        }

                        if ui.button("Delete").clicked() {
                            app.delete_selected();
                            close_menu = true;
                        }

                        ui.separator();

                        if ui.button("Normalize").clicked() {
                            app.normalize_selected();
                            close_menu = true;
                        }

                        if ui.button("Reverse").clicked() {
                            app.reverse_selected();
                            close_menu = true;
                        }

                        if ui.button("Fade In").clicked() {
                            app.apply_fade_in();
                            close_menu = true;
                        }

                        if ui.button("Fade Out").clicked() {
                            app.apply_fade_out();
                            close_menu = true;
                        }

                        let primary = app.selected_clips.first().copied();

                        if ui.button("Toggle Loop").clicked() {
                            if let Some(clip_id) = primary {
                                let enabled = {
                                    let state = app.state.lock().unwrap();
                                    state
                                        .find_clip(clip_id)
                                        .and_then(|(track, loc)| {
                                            if let crate::project::ClipLocation::Midi(idx) = loc {
                                                track.midi_clips.get(idx).map(|c| !c.loop_enabled)
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(true)
                                };
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::ToggleClipLoop { clip_id, enabled });
                            }
                            close_menu = true;
                        }

                        ui.separator();

                        let (is_alias, clip_id_opt) = if let Some(clip_id) = primary {
                            let st = app.state.lock().unwrap();
                            let is_alias = st
                                .find_clip(clip_id)
                                .and_then(|(track, loc)| {
                                    if let crate::project::ClipLocation::Midi(idx) = loc {
                                        track.midi_clips.get(idx).and_then(|c| c.pattern_id)
                                    } else {
                                        None
                                    }
                                })
                                .is_some();
                            (is_alias, Some(clip_id))
                        } else {
                            (false, None)
                        };

                        if ui.button("Duplicate (independent)").clicked() {
                            if let Some(clip_id) = clip_id_opt {
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::DuplicateMidiClip { clip_id });
                            }
                            close_menu = true;
                        }

                        if ui.button("Duplicate as Alias").clicked() {
                            if let Some(clip_id) = clip_id_opt {
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::DuplicateMidiClipAsAlias { clip_id });
                            }
                            close_menu = true;
                        }

                        let make_unique_btn = egui::Button::new("Make Unique");
                        if ui.add_enabled(is_alias, make_unique_btn).clicked() {
                            if let Some(clip_id) = clip_id_opt {
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::MakeClipUnique { clip_id });
                            }
                            close_menu = true;
                        }

                        ui.separator();
                        ui.label("Quantize");

                        let (mut grid, mut strength, mut swing, mut enabled) = {
                            if let Some(clip_id) = clip_id_opt {
                                let st = app.state.lock().unwrap();
                                st.find_clip(clip_id)
                                    .and_then(|(track, loc)| {
                                        if let crate::project::ClipLocation::Midi(idx) = loc {
                                            track.midi_clips.get(idx).map(|c| {
                                                (
                                                    c.quantize_grid,
                                                    c.quantize_strength,
                                                    c.swing,
                                                    c.quantize_enabled,
                                                )
                                            })
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or((0.25, 1.0, 0.0, false))
                            } else {
                                (0.25, 1.0, 0.0, false)
                            }
                        };

                        static GRIDS: [(&str, f32); 6] = [
                            ("1/1", 1.0),
                            ("1/2", 0.5),
                            ("1/4", 0.25),
                            ("1/8", 0.125),
                            ("1/16", 0.0625),
                            ("1/32", 0.03125),
                        ];

                        for (label, g) in GRIDS {
                            if ui
                                .selectable_label((grid - g).abs() < 1e-6, label)
                                .clicked()
                            {
                                grid = g;
                            }
                        }

                        ui.add(egui::Slider::new(&mut strength, 0.0..=1.0).text("Strength"));
                        ui.add(egui::Slider::new(&mut swing, -0.5..=0.5).text("Swing"));

                        let mut en = enabled;
                        if ui.checkbox(&mut en, "Enabled").changed() {
                            enabled = en;
                        }

                        if ui.button("Apply Quantize").clicked() {
                            if let Some(clip_id) = clip_id_opt {
                                let _ = app.command_tx.send(AudioCommand::SetClipQuantize {
                                    clip_id,
                                    grid,
                                    strength,
                                    swing,
                                    enabled,
                                });
                            }
                            close_menu = true;
                        }
                    });
                });

            if close_menu || ui.ctx().input(|i| i.pointer.any_click()) {
                self.show_clip_menu = false;
            }
        }
    }

    pub fn compute_project_end_beats(&self, app: &super::app::YadawApp) -> f64 {
        let state = app.state.lock().unwrap();
        let mut max_beat: f64 = DEFAULT_MIN_PROJECT_BEATS;
        for t in state.tracks.values() {
            for c in &t.audio_clips {
                max_beat = max_beat.max(c.start_beat + c.length_beats);
            }
            for c in &t.midi_clips {
                max_beat = max_beat.max(c.start_beat + c.length_beats);
            }
        }
        max_beat
    }
}

impl Default for TimelineView {
    fn default() -> Self {
        Self::new()
    }
}
