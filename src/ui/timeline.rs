use std::sync::atomic::Ordering;

use super::*;
use crate::constants::{DEFAULT_MIDI_CLIP_LEN, DEFAULT_MIN_PROJECT_BEATS};
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

    snap_enabled: bool,
    snap_to_grid: bool,
    snap_to_clips: bool,
    snap_to_loop: bool,
    snap_px_threshold: f32, // in pixels, default ~10

    // marquee
    selection_box: Option<(egui::Pos2, egui::Pos2)>,

    auto_crossfade_on_overlap: bool,

    snap_preview_beat: Option<f64>,

    // for drag commit and zoom
    last_pointer_pos: Option<egui::Pos2>,

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

    drag_target_track: Option<u64>,
}

#[derive(Clone)]
enum TimelineInteraction {
    DragClip {
        clip_ids_and_starts: Vec<(u64, f64)>,
        start_drag_beat: f64,
        duplicate_on_drop: bool,
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

            snap_enabled: true,
            snap_to_grid: true,
            snap_to_clips: true,
            snap_to_loop: true,
            snap_px_threshold: 10.0,

            selection_box: None,
            auto_crossfade_on_overlap: false,
            snap_preview_beat: None,
            last_pointer_pos: None,

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
            drag_target_track: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.heading("Timeline");
        self.draw_toolbar(ui, app);
        ui.separator();

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .scroll_source(ScrollSource::ALL)
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

                    ui.toggle_value(&mut self.snap_enabled, "On");
                    ui.toggle_value(&mut self.snap_to_grid, "Grid");
                    ui.toggle_value(&mut self.snap_to_clips, "Clips");
                    ui.toggle_value(&mut self.snap_to_loop, "Loop");
                    ui.add(
                        egui::Slider::new(&mut self.snap_px_threshold, 4.0..=24.0)
                            .text("Thresh px"),
                    );

                    ui.separator();
                    ui.checkbox(
                        &mut self.auto_crossfade_on_overlap,
                        "Auto crossfade on overlap",
                    );
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

        // wheel zoom (Ctrl/Cmd + wheel zooms around cursor)
        if response.hovered() {
            let modifiers = ui.input(|i| i.modifiers);
            let scroll = ui.input(|i| i.raw_scroll_delta);
            if (modifiers.ctrl || modifiers.command) && scroll.y.abs() > 0.0 {
                let anchor_x = response
                    .hover_pos()
                    .map(|p| p.x)
                    .unwrap_or(response.rect.center().x);
                let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                self.zoom_horiz_around(response.rect, anchor_x, factor);
            }
        }

        // Hand-pan with spacebar
        if response.dragged() && ui.input(|i| i.key_down(egui::Key::Space)) {
            if let Some(pos) = response.hover_pos() {
                if let Some(last) = self.last_pointer_pos {
                    let dx = pos.x - last.x;
                    self.scroll_x = (self.scroll_x - dx).max(0.0);
                }
            }
            // While space is down, cancel other interactions
            self.timeline_interaction = None;
        }
        self.last_pointer_pos = response.hover_pos().or(self.last_pointer_pos);
        if ui.ctx().input(|i| i.pointer.any_released()) {
            self.last_pointer_pos = None;
        }

        // Draw the grid and horizontal ruler
        let rect = response.rect;
        self.draw_grid(&painter, rect, app.state.lock().unwrap().bpm);

        // loop/seek
        let ruler_h = 18.0;
        let ruler_rect =
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + ruler_h));
        let ruler_resp = ui.interact(
            ruler_rect,
            ui.id().with("timeline_ruler"),
            egui::Sense::click_and_drag(),
        );

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
        self.draw_drag_ghosts(ui, app, rect);
        self.handle_keyboard_nudge(ui, app);

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

        if let Some(b) = self.snap_preview_beat {
            let x = self.beat_to_x(rect, b);
            ui.ctx().debug_painter().line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
            );
        }

        if self.pending_clip_undo {
            app.push_undo();
            self.pending_clip_undo = false;
        }

        // Parent-level pointer handling (seek, loop bars, drag clips).
        // If the pointer is over any lane rect, the automation widgets own it and we return early.
        self.handle_timeline_interaction(&response, &ruler_resp, ui, app);
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

        // Fade handles (visual + drag)
        let handle_w = 10.0;
        let left_handle = egui::Rect::from_min_size(
            egui::pos2(clip_rect.left(), clip_rect.bottom() - 14.0),
            egui::vec2(handle_w, 12.0),
        );
        let right_handle = egui::Rect::from_min_size(
            egui::pos2(clip_rect.right() - handle_w, clip_rect.bottom() - 14.0),
            egui::vec2(handle_w, 12.0),
        );
        ui.painter()
            .rect_filled(left_handle, 2.0, egui::Color32::from_gray(70));
        ui.painter()
            .rect_filled(right_handle, 2.0, egui::Color32::from_gray(70));

        let left_id = ui.id().with(("fade_in", clip.id));
        let right_id = ui.id().with(("fade_out", clip.id));
        let left_resp = ui.interact(left_handle, left_id, egui::Sense::click_and_drag());
        let right_resp = ui.interact(right_handle, right_id, egui::Sense::click_and_drag());

        if left_resp.dragged() {
            if let Some(pos) = left_resp.interact_pointer_pos() {
                let beat_at_cursor = self.x_to_beat(track_rect, pos.x);
                let mut new_len = (beat_at_cursor - clip.start_beat).clamp(0.0, clip.length_beats);
                let (snapped, _) =
                    self.snap_beat(ui, track_rect, clip.start_beat + new_len, app, None);
                new_len = (snapped - clip.start_beat).clamp(0.0, clip.length_beats);
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetAudioClipFadeIn(clip.id, Some(new_len)));
            }
        }
        if right_resp.dragged() {
            if let Some(pos) = right_resp.interact_pointer_pos() {
                let beat_at_cursor = self.x_to_beat(track_rect, pos.x);
                let end_beat = clip.start_beat + clip.length_beats;
                let mut new_len = (end_beat - beat_at_cursor).clamp(0.0, clip.length_beats);
                let (snapped, _) = self.snap_beat(ui, track_rect, end_beat - new_len, app, None);
                new_len = (end_beat - snapped).clamp(0.0, clip.length_beats);
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetAudioClipFadeOut(clip.id, Some(new_len)));
            }
        }
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
        if ui.input(|i| i.key_down(egui::Key::Space)) {
            return;
        }
        // Select on click
        if (response.clicked() || response.drag_started()) && !app.selected_clips.contains(&clip_id)
        {
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

        if response.dragged() && !hover_left && !hover_right {
            response.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
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

                let duplicate_on_drop = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                self.timeline_interaction = Some(TimelineInteraction::DragClip {
                    clip_ids_and_starts: clips_and_starts,
                    start_drag_beat: start_beat_under_mouse,
                    duplicate_on_drop,
                });
            }
        }
    }

    fn handle_timeline_interaction(
        &mut self,
        response: &egui::Response,
        ruler_resp: &egui::Response,
        ui: &mut egui::Ui,
        app: &mut super::app::YadawApp,
    ) {
        if self.timeline_interaction.is_none() {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                if self.automation_hit_regions.iter().any(|r| r.contains(pos)) {
                    return;
                }
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

        // Start marquee selection when dragging over clip area (not ruler/automation)
        if response.drag_started() && self.timeline_interaction.is_none() {
            if let Some(pos) = response.interact_pointer_pos() {
                if pos.y > rect.top() + ruler_h
                    && !self.automation_hit_regions.iter().any(|r| r.contains(pos))
                {
                    self.timeline_interaction = Some(TimelineInteraction::SelectionBox {
                        start_pos: pos,
                        current_pos: pos,
                    });
                }
            }
        }

        // Start drag
        if ruler_resp.drag_started() && self.timeline_interaction.is_none() {
            if let Some(pos) = ruler_resp.interact_pointer_pos() {
                let lb = app.audio_state.loop_start.load();
                let le = app.audio_state.loop_end.load();
                let target = {
                    let rel = (pos.x - response.rect.left()) + self.scroll_x;
                    (rel / self.zoom_x) as f64
                };
                if app.audio_state.loop_enabled.load(Ordering::Relaxed) && (le > lb) {
                    let start_x = response.rect.left() + (lb as f32 * self.zoom_x - self.scroll_x);
                    let end_x = response.rect.left() + (le as f32 * self.zoom_x - self.scroll_x);
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
            // let automation lanes handle themselves unless we’re already in a clip interaction
            if self.timeline_interaction.is_none()
                && self.automation_hit_regions.iter().any(|r| r.contains(pos))
            {
                return;
            }

            // Commands cause stuttering
            let _current_beat = beat_at(pos.x);

            // Update marquee selection box visuals
            if let Some(TimelineInteraction::SelectionBox {
                start_pos,
                current_pos,
            }) = &mut self.timeline_interaction
            {
                *current_pos = pos;
                let r = egui::Rect::from_two_pos(*start_pos, *current_pos);
                let layer =
                    egui::LayerId::new(egui::Order::Foreground, ui.id().with("tl_select_box"));
                let painter = ui.ctx().layer_painter(layer);
                painter.rect_filled(
                    r,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(100, 150, 255, 24),
                );
                painter.rect_stroke(
                    r,
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
                    egui::StrokeKind::Inside,
                );
            }

            match self.timeline_interaction {
                Some(TimelineInteraction::DragClip { .. })
                | Some(TimelineInteraction::ResizeClipLeft { .. })
                | Some(TimelineInteraction::ResizeClipRight { .. })
                | Some(TimelineInteraction::SlipContent { .. }) => {
                    self.auto_pan_during_drag(response.rect, pos.x);
                }
                _ => {}
            }

            // For cross-track moves
            if matches!(
                self.timeline_interaction,
                Some(TimelineInteraction::DragClip { .. })
            ) {
                self.drag_target_track = self
                    .last_track_blocks
                    .iter()
                    .find(|(_, r)| r.contains(pos))
                    .map(|(id, _)| *id);
                // Visual highlight
                if let Some(tid) = self.drag_target_track {
                    if let Some((_, block)) =
                        self.last_track_blocks.iter().find(|(id, _)| *id == tid)
                    {
                        let clip_area = egui::Rect::from_min_size(
                            block.min,
                            egui::vec2(block.width(), self.track_height),
                        );
                        let layer = egui::LayerId::new(
                            egui::Order::Foreground,
                            ui.id().with("drag_track_hi"),
                        );
                        let p = ui.ctx().layer_painter(layer);
                        p.rect_filled(
                            clip_area,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(100, 150, 255, 24),
                        );
                    }
                }
            }

            // Update snap guide preview for the current cursor
            let candidate = self.x_to_beat(response.rect, pos.x).max(0.0);
            let (snapped, snap_src) = self.snap_beat(ui, response.rect, candidate, app, None);
            self.snap_preview_beat = snap_src.or(Some(snapped));
        }

        // END DRAG
        let pointer_released = ui.ctx().input(|i| i.pointer.any_released());
        if pointer_released {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos())
                && let Some(interaction) = &self.timeline_interaction
            {
                match interaction {
                    TimelineInteraction::DragClip {
                        clip_ids_and_starts,
                        start_drag_beat,
                        duplicate_on_drop,
                    } => {
                        let current = self.x_to_beat(response.rect, pos.x);
                        let mut delta = current - *start_drag_beat;

                        // Use earliest selected clip start as reference for snapping
                        let ref_original_start = clip_ids_and_starts
                            .iter()
                            .map(|(_, s)| *s)
                            .fold(f64::INFINITY, f64::min);
                        let ref_original_start = if ref_original_start.is_finite() {
                            ref_original_start
                        } else {
                            0.0
                        };

                        // Snap reference
                        let (snapped, _snap_src) = self.snap_beat(
                            ui,
                            response.rect,
                            ref_original_start + delta,
                            app,
                            None,
                        );
                        delta = snapped - ref_original_start;

                        // Determine target track (if pointer over some track during drag)
                        let target_track_id = self.drag_target_track;

                        let mut allow_move = true;
                        app.push_undo();
                        let state = app.state.lock().unwrap();

                        'outer: for (clip_id, original_start) in clip_ids_and_starts.iter().copied()
                        {
                            let new_start = (original_start + delta).max(0.0);
                            // Resolve target track: either same as source, or hovered one if compatible
                            let (source_track, loc) = match state.find_clip(clip_id) {
                                Some(v) => v,
                                None => continue,
                            };
                            let (track_for_check, is_midi) = match loc {
                                ClipLocation::Midi(idx) => {
                                    let src_is_midi = true;
                                    let tgt = if let Some(tid) = target_track_id {
                                        if let Some(t) = state.tracks.get(&tid) {
                                            if t.is_midi { t } else { source_track }
                                        } else {
                                            source_track
                                        }
                                    } else {
                                        source_track
                                    };
                                    (tgt, src_is_midi)
                                }
                                ClipLocation::Audio(idx) => {
                                    let src_is_midi = false;
                                    let tgt = if let Some(tid) = target_track_id {
                                        if let Some(t) = state.tracks.get(&tid) {
                                            if !t.is_midi { t } else { source_track }
                                        } else {
                                            source_track
                                        }
                                    } else {
                                        source_track
                                    };
                                    (tgt, src_is_midi)
                                }
                            };

                            let new_length = match loc {
                                ClipLocation::Midi(idx) => {
                                    source_track.midi_clips[idx].length_beats
                                }
                                ClipLocation::Audio(idx) => {
                                    source_track.audio_clips[idx].length_beats
                                }
                            };
                            let new_end = new_start + new_length;

                            // Overlap check on the destination track
                            for other in &track_for_check.midi_clips {
                                if other.id == clip_id
                                    || clip_ids_and_starts.iter().any(|(id, _)| *id == other.id)
                                {
                                    continue; // ignore moving set
                                }
                                let other_end = other.start_beat + other.length_beats;
                                if new_start < other_end && new_end > other.start_beat {
                                    allow_move = false;
                                    break 'outer;
                                }
                            }
                            for other in &track_for_check.audio_clips {
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

                        let mut to_send: Vec<AudioCommand> = Vec::new();
                        if allow_move {
                            for (clip_id, original_start) in clip_ids_and_starts.iter().copied() {
                                let new_start = (original_start + delta).max(0.0);
                                let is_midi = state
                                    .clips_by_id
                                    .get(&clip_id)
                                    .map(|r| r.is_midi)
                                    .unwrap_or(false);
                                // Decide whether we’re moving within track, across tracks, or duplicating+moving
                                let current_track_id = state
                                    .clips_by_id
                                    .get(&clip_id)
                                    .map(|r| r.track_id)
                                    .unwrap_or_default();
                                let dest_track_id = if let Some(tid) = target_track_id {
                                    tid
                                } else {
                                    current_track_id
                                };
                                let cross_track = dest_track_id != current_track_id;

                                if *duplicate_on_drop {
                                    if is_midi {
                                        to_send.push(AudioCommand::DuplicateAndMoveMidiClip {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        });
                                    } else {
                                        to_send.push(AudioCommand::DuplicateAndMoveAudioClip {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        });
                                    }
                                } else if cross_track {
                                    if is_midi {
                                        to_send.push(AudioCommand::MoveMidiClipToTrack {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        });
                                    } else {
                                        to_send.push(AudioCommand::MoveAudioClipToTrack {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        });
                                    }
                                } else {
                                    // same-track move as before
                                    if is_midi {
                                        to_send.push(AudioCommand::MoveMidiClip {
                                            clip_id,
                                            new_start,
                                        });
                                    } else {
                                        to_send.push(AudioCommand::MoveAudioClip {
                                            clip_id,
                                            new_start,
                                        });
                                    }
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
                        let candidate = self.x_to_beat(response.rect, pos.x).max(0.0);
                        let (snapped, _) = self.snap_beat(ui, response.rect, candidate, app, None);
                        let new_start = snapped.min(*original_end_beat - min_len);
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
                        let candidate = self.x_to_beat(response.rect, pos.x).max(0.0);
                        let (snapped, _) = self.snap_beat(ui, response.rect, candidate, app, None);
                        let new_end = snapped.max(*original_start_beat + min_len);
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
                    TimelineInteraction::LoopCreate { anchor_beat } => {
                        let cur = self.x_to_beat(response.rect, pos.x).max(0.0);
                        let (s0, _) = self.snap_beat(ui, response.rect, *anchor_beat, app, None);
                        let (s1, _) = self.snap_beat(ui, response.rect, cur, app, None);
                        let (start, end) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
                        app.audio_state.loop_start.store(start);
                        app.audio_state.loop_end.store(end);
                        app.audio_state.loop_enabled.store(true, Ordering::Relaxed);
                    }

                    TimelineInteraction::LoopDragStart { offset_beats } => {
                        let cur = self.x_to_beat(response.rect, pos.x).max(0.0) - *offset_beats;
                        let (snapped, _) = self.snap_beat(ui, response.rect, cur, app, None);
                        let end = app.audio_state.loop_end.load();
                        let start = snapped.min(end - min_len);
                        app.audio_state.loop_start.store(start);
                    }

                    TimelineInteraction::LoopDragEnd { offset_beats } => {
                        let cur = self.x_to_beat(response.rect, pos.x).max(0.0) - *offset_beats;
                        let (snapped, _) = self.snap_beat(ui, response.rect, cur, app, None);
                        let start = app.audio_state.loop_start.load();
                        let end = snapped.max(start + min_len);
                        app.audio_state.loop_end.store(end);
                    }

                    _ => {}
                }
            }

            if let Some(TimelineInteraction::SelectionBox {
                start_pos,
                current_pos,
            }) = self.timeline_interaction.clone()
            {
                let sel_rect = egui::Rect::from_two_pos(start_pos, current_pos);
                let mut selected_ids: Vec<u64> = Vec::new();

                let st = app.state.lock().unwrap();
                for (track_id, track_block) in self.last_track_blocks.iter().copied() {
                    // Only the clip area height
                    let clip_area = egui::Rect::from_min_size(
                        track_block.min,
                        egui::vec2(track_block.width(), self.track_height),
                    );
                    if !sel_rect.intersects(clip_area) {
                        continue;
                    }
                    if let Some(t) = st.tracks.get(&track_id) {
                        for c in &t.audio_clips {
                            if self.clip_rect_for_audio(clip_area, c).intersects(sel_rect) {
                                selected_ids.push(c.id);
                            }
                        }
                        for c in &t.midi_clips {
                            if self.clip_rect_for_midi(clip_area, c).intersects(sel_rect) {
                                selected_ids.push(c.id);
                            }
                        }
                    }
                }
                drop(st);

                if ui.input(|i| i.modifiers.ctrl) {
                    // Union with existing selection
                    for id in selected_ids {
                        if !app.selected_clips.contains(&id) {
                            app.selected_clips.push(id);
                        }
                    }
                } else {
                    app.selected_clips = selected_ids;
                }
            }

            self.timeline_interaction = None;
            self.drag_target_track = None;
        }

        // Click on ruler to set playhead
        if ruler_resp.clicked() {
            if let Some(pos) = ruler_resp.interact_pointer_pos() {
                let rel = (pos.x - response.rect.left()) + self.scroll_x;
                let mut beat = (rel / self.zoom_x) as f64;
                beat = if self.grid_snap > 0.0 {
                    (beat / self.grid_snap as f64).round() * self.grid_snap as f64
                } else {
                    beat
                }
                .max(0.0);
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

    fn x_to_beat(&self, rect: egui::Rect, x: f32) -> f64 {
        let rel = (x - rect.left()) + self.scroll_x;
        (rel / self.zoom_x) as f64
    }
    fn beat_to_x(&self, rect: egui::Rect, beat: f64) -> f32 {
        rect.left() + (beat as f32 * self.zoom_x - self.scroll_x)
    }
    fn zoom_horiz_around(&mut self, rect: egui::Rect, anchor_x: f32, factor: f32) {
        // keep the beat at anchor_x stable while changing zoom_x
        let anchor_beat = self.x_to_beat(rect, anchor_x);
        let prev_zoom = self.zoom_x;
        self.zoom_x = (self.zoom_x * factor).clamp(10.0, 500.0);
        if (self.zoom_x - prev_zoom).abs() > f32::EPSILON {
            let new_x = (anchor_beat as f32) * self.zoom_x;
            // scroll_x so that beat maps back to anchor_x
            let desired_scroll_x = (new_x - (anchor_x - rect.left())).max(0.0);
            self.scroll_x = desired_scroll_x;
        }
    }
    fn snap_beat(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        beat: f64,
        app: &super::app::YadawApp,
        // Limit checks to the current track to avoid accidental snaps across far content
        track_filter: Option<u64>,
    ) -> (f64, Option<f64>) {
        // Shift disables snapping
        if !self.snap_enabled || ui.input(|i| i.modifiers.shift) {
            return (beat, None);
        }

        let mut candidates: Vec<f64> = Vec::with_capacity(64);

        // Grid
        if self.snap_to_grid && self.grid_snap > 0.0 {
            // nearest grid tick around beat: floor and ceil
            let g = self.grid_snap as f64;
            let base = (beat / g).round() * g;
            candidates.push(base);
            // add neighbors for threshold check
            candidates.push(base + g);
            candidates.push(base - g);
        }

        // Clip edges (starts/ends)
        if self.snap_to_clips {
            let state = app.state.lock().unwrap();
            let ids: Vec<u64> = state.track_order.clone();
            for &tid in &ids {
                if track_filter.map_or(false, |tf| tf != tid) {
                    continue;
                }
                if let Some(t) = state.tracks.get(&tid) {
                    for c in &t.audio_clips {
                        candidates.push(c.start_beat);
                        candidates.push(c.start_beat + c.length_beats);
                    }
                    for c in &t.midi_clips {
                        candidates.push(c.start_beat);
                        candidates.push(c.start_beat + c.length_beats);
                    }
                }
            }
        }

        // Loop boundaries
        if self.snap_to_loop {
            candidates.push(app.audio_state.loop_start.load());
            candidates.push(app.audio_state.loop_end.load());
        }

        // Find nearest candidate within pixel threshold
        let thresh_beats = (self.snap_px_threshold / self.zoom_x) as f64;
        let mut best: Option<(f64, f64)> = None; // (candidate, abs_diff)
        for &cand in &candidates {
            let d = (cand - beat).abs();
            if d <= thresh_beats {
                match best {
                    None => best = Some((cand, d)),
                    Some((_, best_d)) if d < best_d => best = Some((cand, d)),
                    _ => {}
                }
            }
        }

        if let Some((cand, _)) = best {
            (cand, Some(cand))
        } else {
            (beat, None)
        }
    }
    fn clip_rect_for_audio(&self, track_rect: egui::Rect, clip: &AudioClip) -> egui::Rect {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_w = clip.length_beats as f32 * self.zoom_x;
        egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 20.0),
            egui::vec2(clip_w, self.track_height - 25.0),
        )
    }
    fn clip_rect_for_midi(&self, track_rect: egui::Rect, clip: &MidiClip) -> egui::Rect {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_w = clip.length_beats as f32 * self.zoom_x;
        egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 5.0),
            egui::vec2(clip_w, self.track_height - 10.0),
        )
    }
    fn auto_pan_during_drag(&mut self, rect: egui::Rect, pointer_x: f32) {
        // pixels from edge where we start panning
        let margin = 48.0;
        // scroll step in px per frame (scaled with zoom so higher zoom pans faster)
        let step_base = (self.zoom_x * 0.25).clamp(4.0, 40.0);

        if pointer_x > rect.right() - margin {
            // pan right
            let t = ((pointer_x - (rect.right() - margin)) / margin).clamp(0.0, 1.0);
            self.scroll_x += step_base * (0.25 + 0.75 * t);
        } else if pointer_x < rect.left() + margin {
            // pan left
            let t = (((rect.left() + margin) - pointer_x) / margin).clamp(0.0, 1.0);
            self.scroll_x = (self.scroll_x - step_base * (0.25 + 0.75 * t)).max(0.0);
        }
    }

    fn draw_drag_ghosts(&self, ui: &mut egui::Ui, app: &super::app::YadawApp, rect: egui::Rect) {
        if let Some(TimelineInteraction::DragClip {
            clip_ids_and_starts,
            start_drag_beat,
            ..
        }) = &self.timeline_interaction
        {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                let current = self.x_to_beat(rect, pos.x);
                let mut delta = current - *start_drag_beat;
                let ref_original_start = clip_ids_and_starts
                    .iter()
                    .map(|(_, s)| *s)
                    .fold(f64::INFINITY, f64::min);
                let (snapped, _) = self.snap_beat(ui, rect, ref_original_start + delta, app, None);
                delta = snapped - ref_original_start;

                // pick track rect
                let tid = self.drag_target_track.or_else(|| {
                    self.last_track_blocks
                        .iter()
                        .find(|(_, r)| r.contains(pos))
                        .map(|(id, _)| *id)
                });
                let (target_clip_area, target_track_id) = if let Some(tid) = tid {
                    if let Some((_, block)) =
                        self.last_track_blocks.iter().find(|(id, _)| *id == tid)
                    {
                        (
                            egui::Rect::from_min_size(
                                block.min,
                                egui::vec2(block.width(), self.track_height),
                            ),
                            tid,
                        )
                    } else {
                        (egui::Rect::NOTHING, 0)
                    }
                } else {
                    (egui::Rect::NOTHING, 0)
                };

                if !target_clip_area.is_negative() {
                    let p = ui.painter();
                    let st = app.state.lock().unwrap();
                    for (clip_id, orig_start) in clip_ids_and_starts {
                        if let Some((track, loc)) = st.find_clip(*clip_id) {
                            let (length, is_midi, name) = match loc {
                                ClipLocation::Midi(idx) => {
                                    let c = &track.midi_clips[idx];
                                    (c.length_beats, true, c.name.as_str())
                                }
                                ClipLocation::Audio(idx) => {
                                    let c = &track.audio_clips[idx];
                                    (c.length_beats, false, c.name.as_str())
                                }
                            };
                            let new_start = (*orig_start + delta).max(0.0);
                            let x = target_clip_area.left()
                                + (new_start as f32 * self.zoom_x - self.scroll_x);
                            let w = (length as f32 * self.zoom_x).max(2.0);
                            let ghost = egui::Rect::from_min_size(
                                egui::pos2(
                                    x,
                                    target_clip_area.top() + if is_midi { 5.0 } else { 20.0 },
                                ),
                                egui::vec2(
                                    w,
                                    self.track_height - if is_midi { 10.0 } else { 25.0 },
                                ),
                            );
                            p.rect_filled(
                                ghost,
                                4.0,
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 28),
                            );
                            p.rect_stroke(
                                ghost,
                                4.0,
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
                                egui::StrokeKind::Outside,
                            );
                            p.text(
                                ghost.min + egui::vec2(6.0, 6.0),
                                egui::Align2::LEFT_TOP,
                                name,
                                egui::FontId::default(),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                }
            }
        }
    }
    fn handle_keyboard_nudge(&mut self, ui: &egui::Ui, app: &mut super::app::YadawApp) {
        let pressed = |k: egui::Key| ui.input(|i| i.key_pressed(k));
        if app.selected_clips.is_empty() {
            return;
        }

        let mods = ui.input(|i| i.modifiers);
        let grid = self.grid_snap.max(0.0001) as f64;
        let small = grid;
        let big = 1.0; // 1 beat when Shift for big steps

        // Helper to move with overlap check on each affected track
        let mut move_clips = |delta: f64| {
            if delta.abs() < f64::EPSILON {
                return;
            }
            let st = app.state.lock().unwrap();
            // group by track and by type
            let mut plan: Vec<(
                bool, /*is_midi*/
                u64,  /*clip_id*/
                u64,  /*track_id*/
                f64,  /*new_start*/
                f64,  /*len*/
            )> = vec![];
            for cid in &app.selected_clips {
                if let Some((track, loc)) = st.find_clip(*cid) {
                    let tid = st
                        .clips_by_id
                        .get(cid)
                        .map(|r| r.track_id)
                        .unwrap_or_default();
                    match loc {
                        ClipLocation::Midi(idx) => {
                            let c = &track.midi_clips[idx];
                            plan.push((
                                true,
                                *cid,
                                tid,
                                (c.start_beat + delta).max(0.0),
                                c.length_beats,
                            ));
                        }
                        ClipLocation::Audio(idx) => {
                            let c = &track.audio_clips[idx];
                            plan.push((
                                false,
                                *cid,
                                tid,
                                (c.start_beat + delta).max(0.0),
                                c.length_beats,
                            ));
                        }
                    }
                }
            }
            // overlap check per track
            'outer: for (is_midi, cid, tid, ns, len) in plan.iter().copied() {
                if let Some(t) = st.tracks.get(&tid) {
                    let ne = ns + len;
                    for o in &t.midi_clips {
                        if o.id == cid {
                            continue;
                        }
                        let oe = o.start_beat + o.length_beats;
                        if ns < oe && ne > o.start_beat {
                            return;
                        }
                    }
                    for o in &t.audio_clips {
                        if o.id == cid {
                            continue;
                        }
                        let oe = o.start_beat + o.length_beats;
                        if ns < oe && ne > o.start_beat {
                            return;
                        }
                    }
                }
            }
            drop(st);
            app.push_undo();
            for (is_midi, cid, _tid, ns, _len) in plan {
                let _ = if is_midi {
                    app.command_tx.send(AudioCommand::MoveMidiClip {
                        clip_id: cid,
                        new_start: ns,
                    })
                } else {
                    app.command_tx.send(AudioCommand::MoveAudioClip {
                        clip_id: cid,
                        new_start: ns,
                    })
                };
            }
        };

        // Nudge left/right
        if pressed(egui::Key::ArrowLeft) {
            move_clips(if mods.shift { -big } else { -small });
        }
        if pressed(egui::Key::ArrowRight) {
            move_clips(if mods.shift { big } else { small });
        }

        // Resize with Cmd/Ctrl (+Shift for left edge)
        let resize_step = if mods.shift { big } else { small };
        if (mods.command || mods.ctrl) && pressed(egui::Key::ArrowRight) {
            app.push_undo();
            for &cid in &app.selected_clips {
                let st = app.state.lock().unwrap();
                let is_midi = st.clips_by_id.get(&cid).map(|r| r.is_midi).unwrap_or(false);
                drop(st);
                let _ = if is_midi {
                    app.command_tx.send(AudioCommand::ResizeMidiClip {
                    clip_id: cid, new_start: /* unchanged */ {
                        let st = app.state.lock().unwrap();
                        let (t, loc) = st.find_clip(cid).unwrap();
                        match loc { ClipLocation::Midi(i) => t.midi_clips[i].start_beat, _ => 0.0 }
                    },
                    new_length: {
                        let st = app.state.lock().unwrap();
                        let (t, loc) = st.find_clip(cid).unwrap();
                        match loc { ClipLocation::Midi(i) => t.midi_clips[i].length_beats + resize_step, _ => 0.0 }
                    },
                })
                } else {
                    app.command_tx.send(AudioCommand::ResizeAudioClip {
                        clip_id: cid,
                        new_start: {
                            let st = app.state.lock().unwrap();
                            let (t, loc) = st.find_clip(cid).unwrap();
                            match loc {
                                ClipLocation::Audio(i) => t.audio_clips[i].start_beat,
                                _ => 0.0,
                            }
                        },
                        new_length: {
                            let st = app.state.lock().unwrap();
                            let (t, loc) = st.find_clip(cid).unwrap();
                            match loc {
                                ClipLocation::Audio(i) => {
                                    t.audio_clips[i].length_beats + resize_step
                                }
                                _ => 0.0,
                            }
                        },
                    })
                };
            }
        }
        if (mods.command || mods.ctrl) && pressed(egui::Key::ArrowLeft) {
            // shrink right edge
            app.push_undo();
            for &cid in &app.selected_clips {
                let st = app.state.lock().unwrap();
                let is_midi = st.clips_by_id.get(&cid).map(|r| r.is_midi).unwrap_or(false);
                let (start, len) = {
                    let (t, loc) = st.find_clip(cid).unwrap();
                    match loc {
                        ClipLocation::Midi(i) => {
                            (t.midi_clips[i].start_beat, t.midi_clips[i].length_beats)
                        }
                        ClipLocation::Audio(i) => {
                            (t.audio_clips[i].start_beat, t.audio_clips[i].length_beats)
                        }
                    }
                };
                drop(st);
                let new_len = (len - resize_step).max(self.grid_snap.max(0.03125) as f64);
                let _ = if is_midi {
                    app.command_tx.send(AudioCommand::ResizeMidiClip {
                        clip_id: cid,
                        new_start: start,
                        new_length: new_len,
                    })
                } else {
                    app.command_tx.send(AudioCommand::ResizeAudioClip {
                        clip_id: cid,
                        new_start: start,
                        new_length: new_len,
                    })
                };
            }
        }

        // Alt+Arrows: slip MIDI content
        if mods.alt && (pressed(egui::Key::ArrowLeft) || pressed(egui::Key::ArrowRight)) {
            let dir = if pressed(egui::Key::ArrowLeft) {
                -1.0
            } else {
                1.0
            };
            let step = dir * if mods.shift { big } else { small };
            app.push_undo();
            for &cid in &app.selected_clips {
                let _ = app.command_tx.send(AudioCommand::SetClipContentOffset {
                    clip_id: cid,
                    new_offset: {
                        let st = app.state.lock().unwrap();
                        if let Some((t, loc)) = st.find_clip(cid) {
                            if let ClipLocation::Midi(i) = loc {
                                t.midi_clips[i].content_offset_beats + step
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        }
                    },
                });
            }
        }

        // Up/Down: move to prev/next compatible track
        if pressed(egui::Key::ArrowUp) || pressed(egui::Key::ArrowDown) {
            let dir = if pressed(egui::Key::ArrowUp) {
                -1isize
            } else {
                1isize
            };
            let st = app.state.lock().unwrap();
            let order = &st.track_order;
            // pick anchor track (of first selected)
            if let Some(&first) = app.selected_clips.first() {
                if let Some((track, _loc)) = st.find_clip(first) {
                    let cur_tid = st
                        .clips_by_id
                        .get(&first)
                        .map(|r| r.track_id)
                        .unwrap_or_default();
                    let cur_ix = order.iter().position(|t| *t == cur_tid).unwrap_or(0) as isize;
                    let target_ix = (cur_ix + dir).clamp(0, order.len() as isize - 1) as usize;
                    let target_tid = order[target_ix];
                    let target_is_midi = st
                        .tracks
                        .get(&target_tid)
                        .map(|t| t.is_midi)
                        .unwrap_or(false);
                    drop(st);
                    app.push_undo();
                    for &cid in &app.selected_clips {
                        // keep same start, move to neighbor if compatible
                        let st2 = app.state.lock().unwrap();
                        let (src_t, loc) = match st2.find_clip(cid) {
                            Some(v) => v,
                            None => continue,
                        };
                        let (start, is_midi) = match loc {
                            ClipLocation::Midi(i) => (src_t.midi_clips[i].start_beat, true),
                            ClipLocation::Audio(i) => (src_t.audio_clips[i].start_beat, false),
                        };
                        drop(st2);
                        if is_midi == target_is_midi {
                            let _ = if is_midi {
                                app.command_tx.send(AudioCommand::MoveMidiClipToTrack {
                                    clip_id: cid,
                                    dest_track_id: target_tid,
                                    new_start: start,
                                })
                            } else {
                                app.command_tx.send(AudioCommand::MoveAudioClipToTrack {
                                    clip_id: cid,
                                    dest_track_id: target_tid,
                                    new_start: start,
                                })
                            };
                        }
                    }
                }
            }
        }
    }
}

impl Default for TimelineView {
    fn default() -> Self {
        Self::new()
    }
}
