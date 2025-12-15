use std::sync::atomic::Ordering;

use crate::constants::{DEFAULT_MIDI_CLIP_LEN, DEFAULT_MIN_PROJECT_BEATS};
use crate::messages::AudioCommand;
use crate::model::track::TrackType;
use crate::model::{AudioClip, AutomationTarget, MidiClip, MidiNote, Track};
use crate::project::ClipLocation;
use crate::ui::automation_lane::{AutomationAction, AutomationLaneWidget};
use crate::ui::waveform::draw_waveform;
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
    pub show_clip_menu: bool,
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

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, _app: &super::app::YadawApp) {
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
                    ui.checkbox(&mut self.show_automation, "Show Automation");
                    ui.checkbox(&mut self.auto_scroll, "Auto-scroll");

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

                    ui.label("Snap:");

                    ui.toggle_value(&mut self.snap_enabled, "On");
                    ui.toggle_value(&mut self.snap_to_grid, "Grid");
                    ui.toggle_value(&mut self.snap_to_clips, "Clips");
                    ui.toggle_value(&mut self.snap_to_loop, "Loop");
                    ui.add(
                        egui::Slider::new(&mut self.snap_px_threshold, 4.0..=24.0)
                            .text("Thresh px"),
                    );
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
        const LANE_HEADER_H: f32 = 22.0;

        let track_heights: Vec<f32> = track_data
            .iter()
            .map(|(_, t)| {
                if self.show_automation {
                    let extra: f32 = t
                        .automation_lanes
                        .iter()
                        .filter(|l| l.visible)
                        .map(|l| {
                            let curve_h = l.height.max(24.0);
                            curve_h + LANE_HEADER_H
                        })
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
                self.draw_automation_lanes(ui, block_rect, &track, *track_id, app);
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
        self.draw_resize_previews(ui, app, rect);
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
            let stroke = egui::Stroke::new(if is_bar { 1.5 } else { 1.0 }, color);
            painter.line_segment(
                [
                    egui::pos2(x, rect.top()),
                    egui::pos2(x, rect.top() + ruler_h),
                ],
                stroke,
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
        let track_color = track.color;

        let vis = ui.visuals();
        let base = vis.extreme_bg_color;
        let idx = app.track_id_to_index(track_id).unwrap_or(0);

        let bg_color = if idx % 2 == 0 {
            base
        } else {
            // Slightly lighter/different tint for alternate rows
            if vis.dark_mode {
                base.gamma_multiply(1.2)
            } else {
                base.gamma_multiply(0.95)
            }
        };

        painter.rect_filled(rect, 0.0, bg_color);

        if let Some((r, g, b)) = track_color {
            let strip_rect = egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height()));
            painter.rect_filled(strip_rect, 0.0, egui::Color32::from_rgb(r, g, b));
        }

        painter.text(
            rect.min + egui::vec2(8.0, 5.0),
            egui::Align2::LEFT_TOP,
            &track.name,
            egui::FontId::default(),
            vis.text_color().gamma_multiply(0.5),
        );

        if matches!(track.track_type, TrackType::Midi) {
            for clip in &track.midi_clips {
                self.draw_midi_clip(painter, ui, rect, clip, track_id, app, track_color);
            }
        } else {
            for clip in &track.audio_clips {
                self.draw_audio_clip(painter, ui, rect, clip, track_id, app, track_color);
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
        track_color: Option<(u8, u8, u8)>,
    ) {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;

        let bpm = app.audio_state.bpm.load();
        let audio_duration_seconds = clip.samples.len() as f64 / clip.sample_rate as f64;
        let audio_length_beats = audio_duration_seconds * (bpm as f64 / 60.0);

        let effective_length_beats = (audio_length_beats as f32).min(clip.length_beats as f32);
        let clip_width = effective_length_beats * self.zoom_x;

        let clip_rect = egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 20.0),
            egui::vec2(clip_width, self.track_height - 25.0),
        );

        if clip_rect.right() < track_rect.left() || clip_rect.left() > track_rect.right() {
            return;
        }

        // Color Resolution: Clip -> Track -> Default
        let default_color = if ui.visuals().dark_mode {
            (70, 75, 80)
        } else {
            (200, 200, 210)
        };
        let (r, g, b) = clip.color.or(track_color).unwrap_or(default_color);
        let base_color = egui::Color32::from_rgb(r, g, b);

        // Calculate brightness to determine text/waveform contrast
        let brightness = r as f32 * 0.299 + g as f32 * 0.587 + b as f32 * 0.114;
        let is_light = brightness > 140.0;

        let fg_color = if is_light {
            egui::Color32::BLACK.gamma_multiply(0.8)
        } else {
            egui::Color32::WHITE.gamma_multiply(0.9)
        };

        // Fill Background
        painter.rect_filled(clip_rect, 3.0, base_color);

        draw_waveform(
            painter,
            clip_rect,
            clip,
            self.zoom_x,
            self.scroll_x,
            fg_color.gamma_multiply(0.6),
        );

        // Audio Looping Indicators (Visual only)
        if clip.loop_enabled {
            let src_len_beats =
                (clip.samples.len() as f64 / clip.sample_rate as f64) * (bpm as f64 / 60.0);
            if src_len_beats > 0.0 && src_len_beats < clip.length_beats {
                let reps = (clip.length_beats / src_len_beats).ceil() as i32;
                for k in 1..reps {
                    let offset_x = (k as f64 * src_len_beats * self.zoom_x as f64) as f32;
                    let line_x = clip_rect.left() + offset_x;
                    if line_x < clip_rect.right() {
                        painter.line_segment(
                            [
                                egui::pos2(line_x, clip_rect.top()),
                                egui::pos2(line_x, clip_rect.bottom()),
                            ],
                            egui::Stroke::new(1.0, fg_color.gamma_multiply(0.3)),
                        );
                    }
                }
            }
        }

        let bar_color = if is_light {
            base_color.gamma_multiply(0.8)
        } else {
            base_color.gamma_multiply(1.3) // lighter for dark clips
        };
        let bar_rect = egui::Rect::from_min_size(clip_rect.min, egui::vec2(clip_rect.width(), 4.0));
        painter.rect_filled(bar_rect, 3.0, bar_color);

        painter.text(
            clip_rect.left_top() + egui::vec2(4.0, 5.0),
            egui::Align2::LEFT_TOP,
            &clip.name,
            egui::FontId::proportional(12.0),
            fg_color,
        );

        let response = ui.interact(
            clip_rect,
            ui.id().with(("audio_clip", clip.id)),
            egui::Sense::click_and_drag(),
        );

        let is_selected = app.selected_clips.contains(&clip.id);

        if is_selected {
            painter.rect_stroke(
                clip_rect,
                3.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        } else {
            // Subtle border to define edges against lane
            painter.rect_stroke(
                clip_rect,
                3.0,
                egui::Stroke::new(1.0, bar_color),
                egui::StrokeKind::Inside,
            );
        }

        self.handle_clip_interaction(response, clip.id, ui, clip_rect, app);

        // 6. Draw Fades (Visual feedback)
        let in_px =
            (clip.fade_in.unwrap_or(0.0) as f32 * self.zoom_x).clamp(0.0, clip_rect.width());
        let out_px =
            (clip.fade_out.unwrap_or(0.0) as f32 * self.zoom_x).clamp(0.0, clip_rect.width());

        if in_px > 1.0 {
            // Fade-in triangle slope visual
            let p1 = clip_rect.left_bottom();
            let p2 = clip_rect.left_top() + egui::vec2(in_px, 0.0);
            let p3 = clip_rect.left_top();

            // Draw a subtle darkening/masking triangle to represent volume attenuation
            let mut mesh = egui::Mesh::default();
            let base = mesh.vertices.len() as u32;

            mesh.add_triangle(base, base + 1, base + 2);
            mesh.colored_vertex(p1, egui::Color32::from_black_alpha(0));
            mesh.colored_vertex(p2, egui::Color32::from_black_alpha(0));
            mesh.colored_vertex(p3, egui::Color32::from_black_alpha(100)); // Darken top-left
            painter.add(mesh);

            painter.line_segment(
                [p1, p2],
                egui::Stroke::new(1.0, fg_color.gamma_multiply(0.5)),
            );
        }

        if out_px > 1.0 {
            // Fade-out line
            let p1 = clip_rect.right_top() - egui::vec2(out_px, 0.0);
            let p2 = clip_rect.right_bottom();
            painter.line_segment(
                [p1, p2],
                egui::Stroke::new(1.0, fg_color.gamma_multiply(0.5)),
            );
        }

        // Fade handles
        let dot_r = 5.0;
        let left_dot_center = egui::pos2(clip_rect.left() + in_px, clip_rect.top() + 6.0);
        let right_dot_center = egui::pos2(clip_rect.right() - out_px, clip_rect.top() + 6.0);

        {
            let dot_id = ui.id().with(("fade_in_dot", clip.id));
            let dot_rect = egui::Rect::from_center_size(left_dot_center, egui::vec2(14.0, 14.0));
            let resp = ui.interact(dot_rect, dot_id, egui::Sense::click_and_drag());
            if resp.hovered() || resp.dragged() {
                ui.painter()
                    .circle_filled(left_dot_center, dot_r, egui::Color32::from_gray(220));
                ui.painter().circle_stroke(
                    left_dot_center,
                    dot_r + 2.0,
                    egui::Stroke::new(1.0, egui::Color32::WHITE),
                );
            }
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let beat_at_cursor = self.x_to_beat(track_rect, pos.x);
                    let mut new_len =
                        (beat_at_cursor - clip.start_beat).clamp(0.0, clip.length_beats);
                    let (snapped, _) =
                        self.snap_beat(ui, track_rect, clip.start_beat + new_len, app, None);
                    new_len = (snapped - clip.start_beat).clamp(0.0, clip.length_beats);
                    let _ = app
                        .command_tx
                        .send(AudioCommand::SetAudioClipFadeIn(clip.id, Some(new_len)));
                }
            }
        }

        {
            let dot_id = ui.id().with(("fade_out_dot", clip.id));
            let dot_rect = egui::Rect::from_center_size(right_dot_center, egui::vec2(14.0, 14.0));
            let resp = ui.interact(dot_rect, dot_id, egui::Sense::click_and_drag());
            if resp.hovered() || resp.dragged() {
                ui.painter()
                    .circle_filled(right_dot_center, dot_r, egui::Color32::from_gray(220));
                ui.painter().circle_stroke(
                    right_dot_center,
                    dot_r + 2.0,
                    egui::Stroke::new(1.0, egui::Color32::WHITE),
                );
            }
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let beat_at_cursor = self.x_to_beat(track_rect, pos.x);
                    let end_beat = clip.start_beat + clip.length_beats;
                    let mut new_len = (end_beat - beat_at_cursor).clamp(0.0, clip.length_beats);
                    let (snapped, _) =
                        self.snap_beat(ui, track_rect, end_beat - new_len, app, None);
                    new_len = (end_beat - snapped).clamp(0.0, clip.length_beats);
                    let _ = app
                        .command_tx
                        .send(AudioCommand::SetAudioClipFadeOut(clip.id, Some(new_len)));
                }
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

        let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255));
        painter.line_segment(
            [
                egui::pos2(start_x, rect.top()),
                egui::pos2(start_x, rect.bottom()),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(end_x, rect.top()),
                egui::pos2(end_x, rect.bottom()),
            ],
            stroke,
        );
    }

    fn draw_midi_clip(
        &mut self,
        painter: &egui::Painter,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        clip: &crate::model::clip::MidiClip,
        track_id: u64,
        app: &mut super::app::YadawApp,
        track_color: Option<(u8, u8, u8)>,
    ) {
        // Compute clip rectangle
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_width = clip.length_beats as f32 * self.zoom_x;

        let clip_rect = egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 5.0),
            egui::vec2(clip_width, self.track_height - 10.0),
        );

        if clip_rect.right() < track_rect.left() || clip_rect.left() > track_rect.right() {
            return;
        }

        // Color Resolution
        let default_color = if ui.visuals().dark_mode {
            (50, 80, 120) // Dark blueish
        } else {
            (180, 200, 230) // Light blueish
        };
        let (r, g, b) = clip.color.or(track_color).unwrap_or(default_color);
        let base_color = egui::Color32::from_rgb(r, g, b);

        // Contrast Calculation
        let brightness = r as f32 * 0.299 + g as f32 * 0.587 + b as f32 * 0.114;
        let is_light = brightness > 140.0;
        let _fg_color = if is_light {
            egui::Color32::BLACK.gamma_multiply(0.7)
        } else {
            egui::Color32::WHITE.gamma_multiply(0.9)
        };
        let note_color = if is_light {
            egui::Color32::BLACK.gamma_multiply(0.5)
        } else {
            egui::Color32::WHITE.gamma_multiply(0.6)
        };

        painter.rect_filled(clip_rect, 4.0, base_color);

        let base_notes: Vec<MidiNote> = {
            let state = app.state.lock().unwrap();
            if let Some(pid) = clip.pattern_id {
                state
                    .patterns
                    .get(&pid)
                    .map(|p| p.notes.clone())
                    .unwrap_or_else(|| clip.notes.clone())
            } else {
                clip.notes.clone()
            }
        };

        // Content-loop preview
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
                            if is_light {
                                egui::Color32::from_rgba_premultiplied(0, 0, 0, 40)
                            } else {
                                egui::Color32::from_rgba_premultiplied(255, 255, 255, 40)
                            },
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

            for note in &base_notes {
                let s_loc = (note.start + offset).rem_euclid(content_len);
                let e_loc_raw = s_loc + note.duration;

                // A note that crosses the content boundary is split into two segments visually
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

                    // Project to screen space
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
                            egui::vec2((seg_right - seg_left).max(2.0), 3.0),
                        ),
                        1.0,
                        note_color,
                    );
                }
            }
        }

        // Clip name label
        painter.text(
            clip_rect.min + egui::vec2(5.0, 5.0),
            egui::Align2::LEFT_TOP,
            &clip.name,
            egui::FontId::default(),
            egui::Color32::WHITE,
        );

        // Interaction
        let response = ui.interact(
            clip_rect,
            ui.id().with(("midi_clip", clip.id)),
            egui::Sense::click_and_drag(),
        );

        let is_selected = app.selected_clips.contains(&clip.id);

        let stroke_color = if is_selected {
            egui::Color32::WHITE
        } else {
            // Darker/Lighter border of base color
            if is_light {
                base_color.gamma_multiply(0.7)
            } else {
                base_color.gamma_multiply(1.3)
            }
        };
        let stroke_width = if is_selected { 2.0 } else { 1.0 };

        painter.rect_stroke(
            clip_rect,
            4.0,
            egui::Stroke::new(stroke_width, stroke_color),
            egui::StrokeKind::Inside,
        );

        if response.double_clicked() {
            app.select_track(track_id);
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
            if !ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
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
                        ClipLocation::Midi(idx) => (
                            track.midi_clips[idx].start_beat,
                            track.midi_clips[idx].length_beats,
                        ),
                        ClipLocation::Audio(idx) => (
                            track.audio_clips[idx].start_beat,
                            track.audio_clips[idx].length_beats,
                        ),
                    }
                } else {
                    (0.0, 0.0)
                }
            };

            let start_beat_under_mouse = response
                .interact_pointer_pos()
                .map(|pos| self.x_to_beat(clip_rect, pos.x))
                .unwrap_or(clip_start);

            let alt = ui.input(|i| i.modifiers.alt);

            // Slip content (Alt+drag)
            if alt && !hover_left && !hover_right {
                let (start_offset, content_len, is_midi) = {
                    let state = app.state.lock().unwrap();
                    if let Some((track, loc)) = state.find_clip(clip_id) {
                        if let ClipLocation::Midi(idx) = loc {
                            let c = &track.midi_clips[idx];
                            (
                                c.content_offset_beats,
                                c.content_len_beats.max(0.000001),
                                true,
                            )
                        } else {
                            (0.0, 1.0, false)
                        }
                    } else {
                        (0.0, 1.0, false)
                    }
                };

                if is_midi {
                    self.timeline_interaction = Some(TimelineInteraction::SlipContent {
                        clip_id,
                        start_offset: start_offset.rem_euclid(content_len),
                        start_mouse_beat: start_beat_under_mouse,
                    });
                    return;
                }
                // only for midi
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
                            ClipLocation::Midi(idx) => {
                                clips_and_starts.push((cid, track.midi_clips[idx].start_beat));
                            }
                            ClipLocation::Audio(idx) => {
                                clips_and_starts.push((cid, track.audio_clips[idx].start_beat));
                            }
                        }
                    }
                }
                drop(state);

                let duplicate_on_drop = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

                let clicked_clip_start = clips_and_starts
                    .iter()
                    .find(|(cid, _)| *cid == clip_id)
                    .map(|(_, start)| *start)
                    .unwrap_or(start_beat_under_mouse);

                self.timeline_interaction = Some(TimelineInteraction::DragClip {
                    clip_ids_and_starts: clips_and_starts,
                    start_drag_beat: clicked_clip_start,
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
                let target = self.x_to_beat(response.rect, pos.x);

                if app.audio_state.loop_enabled.load(Ordering::Relaxed) && (le > lb) {
                    let start_x = self.beat_to_x(response.rect, lb);
                    let end_x = self.beat_to_x(response.rect, le);
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

            if let Some(TimelineInteraction::DragClip { .. })
            | Some(TimelineInteraction::ResizeClipLeft { .. })
            | Some(TimelineInteraction::ResizeClipRight { .. })
            | Some(TimelineInteraction::SlipContent { .. }) = self.timeline_interaction
            {
                self.auto_pan_during_drag(response.rect, pos.x);
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
            if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                if let Some(interaction) = self.timeline_interaction.clone() {
                    match interaction {
                        TimelineInteraction::DragClip {
                            clip_ids_and_starts,
                            start_drag_beat,
                            duplicate_on_drop,
                        } => {
                            // Compute snapped delta
                            let current = self.x_to_beat(response.rect, pos.x);
                            let mut delta = current - start_drag_beat;

                            // Snap relative to the earliest original start among dragged clips
                            let ref_original_start = clip_ids_and_starts
                                .iter()
                                .map(|(_, s)| *s)
                                .fold(f64::INFINITY, f64::min);

                            let (snapped, _) = self.snap_beat(
                                ui,
                                response.rect,
                                ref_original_start + delta,
                                app,
                                None,
                            );
                            delta = snapped - ref_original_start;

                            // Destination track under cursor (fallback: source track of first clip)
                            let dest_track_id = self
                                .last_track_blocks
                                .iter()
                                .find(|(_, r)| r.contains(pos))
                                .map(|(id, _)| *id)
                                .unwrap_or_else(|| {
                                    clip_ids_and_starts
                                        .first()
                                        .and_then(|(cid, _)| {
                                            let st = app.state.lock().unwrap();
                                            st.clips_by_id.get(cid).map(|r| r.track_id)
                                        })
                                        .unwrap_or(0)
                                });

                            // Cache destination track snapshot once
                            let (dest_is_midi, dest_clips_audio, dest_clips_midi) = {
                                let st = app.state.lock().unwrap();
                                if let Some(t) = st.tracks.get(&dest_track_id) {
                                    (
                                        matches!(
                                            t.track_type,
                                            crate::model::track::TrackType::Midi
                                        ),
                                        t.audio_clips.clone(),
                                        t.midi_clips.clone(),
                                    )
                                } else {
                                    (false, Vec::new(), Vec::new())
                                }
                            };

                            // Build a set of dragged clip IDs so we don’t punch-out ourselves
                            let sel_ids: std::collections::HashSet<u64> =
                                clip_ids_and_starts.iter().map(|(cid, _)| *cid).collect();

                            // Helper: punch-out (cut-in) overlapped regions on destination track
                            let punch_out = |start: f64, end: f64, is_midi: bool| {
                                if is_midi {
                                    for c in &dest_clips_midi {
                                        if sel_ids.contains(&c.id) {
                                            continue;
                                        }
                                        let c_start = c.start_beat;
                                        let c_end = c_start + c.length_beats;
                                        if start < c_end && end > c_start {
                                            let _ = app.command_tx.send(
                                                AudioCommand::PunchOutMidiClip {
                                                    clip_id: c.id,
                                                    start_beat: start.max(c_start).min(c_end),
                                                    end_beat: end.max(c_start).min(c_end),
                                                },
                                            );
                                        }
                                    }
                                } else {
                                    for c in &dest_clips_audio {
                                        if sel_ids.contains(&c.id) {
                                            continue;
                                        }
                                        let c_start = c.start_beat;
                                        let c_end = c_start + c.length_beats;
                                        if start < c_end && end > c_start {
                                            let _ = app.command_tx.send(
                                                AudioCommand::PunchOutAudioClip {
                                                    clip_id: c.id,
                                                    start_beat: start.max(c_start).min(c_end),
                                                    end_beat: end.max(c_start).min(c_end),
                                                },
                                            );
                                        }
                                    }
                                }
                            };

                            // For each dragged clip: compute new window, cut-in any overlapped region first, then move/duplicate
                            for (clip_id, original_start) in clip_ids_and_starts.iter().copied() {
                                // Resolve source type and length
                                let (src_track_id, is_midi, length_beats) = {
                                    let st = app.state.lock().unwrap();
                                    if let Some(clip_ref) = st.clips_by_id.get(&clip_id) {
                                        let is_midi = clip_ref.is_midi;
                                        let len = if is_midi {
                                            st.tracks
                                                .get(&clip_ref.track_id)
                                                .and_then(|t| {
                                                    t.midi_clips.iter().find(|c| c.id == clip_id)
                                                })
                                                .map(|c| c.length_beats)
                                                .unwrap_or(0.0)
                                        } else {
                                            st.tracks
                                                .get(&clip_ref.track_id)
                                                .and_then(|t| {
                                                    t.audio_clips.iter().find(|c| c.id == clip_id)
                                                })
                                                .map(|c| c.length_beats)
                                                .unwrap_or(0.0)
                                        };
                                        (clip_ref.track_id, is_midi, len)
                                    } else {
                                        continue;
                                    }
                                };

                                // Skip incompatible destination track
                                if dest_is_midi != is_midi {
                                    continue;
                                }

                                let new_start = (original_start + delta).max(0.0);
                                let new_end = new_start + length_beats.max(0.0);

                                // 1) Default UX: Punch-out overlapped regions on destination
                                punch_out(new_start, new_end, is_midi);

                                // 2) Move or duplicate onto destination
                                let cmd = if duplicate_on_drop {
                                    if is_midi {
                                        AudioCommand::DuplicateAndMoveMidiClip {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        }
                                    } else {
                                        AudioCommand::DuplicateAndMoveAudioClip {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        }
                                    }
                                } else if dest_track_id != src_track_id {
                                    if is_midi {
                                        AudioCommand::MoveMidiClipToTrack {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        }
                                    } else {
                                        AudioCommand::MoveAudioClipToTrack {
                                            clip_id,
                                            dest_track_id,
                                            new_start,
                                        }
                                    }
                                } else {
                                    if is_midi {
                                        AudioCommand::MoveMidiClip { clip_id, new_start }
                                    } else {
                                        AudioCommand::MoveAudioClip { clip_id, new_start }
                                    }
                                };
                                let _ = app.command_tx.send(cmd);

                                let bpm = app.audio_state.bpm.load();

                                if !is_midi && self.auto_crossfade_on_overlap {
                                    // ~20ms in beats (at current BPM). You can tune this.
                                    let fade_beats = 0.02f64 * (bpm as f64 / 60.0);
                                    let _ = app.command_tx.send(AudioCommand::SetAudioClipFadeIn(
                                        clip_id,
                                        Some(fade_beats),
                                    ));
                                    let _ = app.command_tx.send(AudioCommand::SetAudioClipFadeOut(
                                        clip_id,
                                        Some(fade_beats),
                                    ));
                                }
                            }

                            let _ = app.command_tx.send(AudioCommand::UpdateTracks);
                        }
                        TimelineInteraction::ResizeClipLeft {
                            clip_id,
                            original_end_beat,
                        } => {
                            let candidate = self.x_to_beat(response.rect, pos.x).max(0.0);
                            let (snapped, _) =
                                self.snap_beat(ui, response.rect, candidate, app, None);
                            let new_start = snapped.min(original_end_beat - min_len);
                            let new_len = (original_end_beat - new_start).max(min_len);

                            let is_midi = app
                                .state
                                .lock()
                                .unwrap()
                                .clips_by_id
                                .get(&clip_id)
                                .map_or(false, |r| r.is_midi);
                            let cmd = if is_midi {
                                AudioCommand::ResizeMidiClip {
                                    clip_id,
                                    new_start,
                                    new_length: new_len,
                                }
                            } else {
                                AudioCommand::ResizeAudioClip {
                                    clip_id,
                                    new_start,
                                    new_length: new_len,
                                }
                            };
                            let _ = app.command_tx.send(cmd);
                            app.push_undo();
                        }
                        TimelineInteraction::ResizeClipRight {
                            clip_id,
                            original_start_beat,
                        } => {
                            let candidate = self.x_to_beat(response.rect, pos.x).max(0.0);
                            let (snapped, _) =
                                self.snap_beat(ui, response.rect, candidate, app, None);
                            let new_end = snapped.max(original_start_beat + min_len);
                            let new_len = (new_end - original_start_beat).max(min_len);

                            let is_midi = app
                                .state
                                .lock()
                                .unwrap()
                                .clips_by_id
                                .get(&clip_id)
                                .map_or(false, |r| r.is_midi);
                            let cmd = if is_midi {
                                AudioCommand::ResizeMidiClip {
                                    clip_id,
                                    new_start: original_start_beat,
                                    new_length: new_len,
                                }
                            } else {
                                AudioCommand::ResizeAudioClip {
                                    clip_id,
                                    new_start: original_start_beat,
                                    new_length: new_len,
                                }
                            };
                            let _ = app.command_tx.send(cmd);
                            app.push_undo();
                        }
                        TimelineInteraction::SlipContent {
                            clip_id,
                            start_offset,
                            start_mouse_beat,
                        } => {
                            let delta = self.x_to_beat(rect, pos.x) - start_mouse_beat;
                            let new_off = start_offset + delta;
                            let _ = app.command_tx.send(AudioCommand::SetClipContentOffset {
                                clip_id,
                                new_offset: new_off,
                            });
                            app.push_undo();
                        }
                        TimelineInteraction::LoopCreate { anchor_beat } => {
                            let cur = self.x_to_beat(response.rect, pos.x).max(0.0);
                            let (s0, _) = self.snap_beat(ui, response.rect, anchor_beat, app, None);
                            let (s1, _) = self.snap_beat(ui, response.rect, cur, app, None);
                            let (start, end) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
                            app.audio_state.loop_start.store(start);
                            app.audio_state.loop_end.store(end);
                            app.audio_state.loop_enabled.store(true, Ordering::Relaxed);
                        }
                        TimelineInteraction::LoopDragStart { offset_beats } => {
                            let cur = self.x_to_beat(response.rect, pos.x).max(0.0) - offset_beats;
                            let (snapped, _) = self.snap_beat(ui, response.rect, cur, app, None);
                            let end = app.audio_state.loop_end.load();
                            let start = snapped.min(end - min_len);
                            app.audio_state.loop_start.store(start);
                        }
                        TimelineInteraction::LoopDragEnd { offset_beats } => {
                            let cur = self.x_to_beat(response.rect, pos.x).max(0.0) - offset_beats;
                            let (snapped, _) = self.snap_beat(ui, response.rect, cur, app, None);
                            let start = app.audio_state.loop_start.load();
                            let end = snapped.max(start + min_len);
                            app.audio_state.loop_end.store(end);
                        }
                        _ => {}
                    }
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

                if ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
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
                let mut beat = self.x_to_beat(response.rect, pos.x);
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
            if let Some(track_id) = self
                .last_track_blocks
                .iter()
                .find(|(_, r)| r.contains(pos))
                .map(|(id, _)| *id)
            {
                if ui.input(|i| i.modifiers.ctrl) {
                    let is_midi = {
                        let state = app.state.lock().unwrap();
                        state
                            .tracks
                            .get(&track_id)
                            .map(|t| matches!(t.track_type, TrackType::Midi))
                            .unwrap_or(false)
                    };
                    if is_midi {
                        let mut beat = self.x_to_beat(response.rect, pos.x);
                        beat = if self.grid_snap > 0.0 {
                            (beat / self.grid_snap as f64).round() * self.grid_snap as f64
                        } else {
                            beat
                        };
                        let _ = app.command_tx.send(AudioCommand::CreateMidiClip {
                            track_id,
                            start_beat: beat,
                            length_beats: DEFAULT_MIDI_CLIP_LEN,
                        });
                    }
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
        const HEADER_H: f32 = 22.0;

        let visible_lanes: Vec<(usize, f32)> = track
            .automation_lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.visible)
            .map(|(i, l)| (i, l.height.max(24.0)))
            .collect();

        if visible_lanes.is_empty() {
            return;
        }

        let mut y = track_rect.top() + self.track_height;

        for (lane_idx, curve_h) in visible_lanes {
            let lane_h = HEADER_H + curve_h;

            let lane_rect = egui::Rect::from_min_size(
                egui::pos2(track_rect.left(), y),
                egui::vec2(track_rect.width(), lane_h),
            );

            let header_rect =
                egui::Rect::from_min_size(lane_rect.min, egui::vec2(lane_rect.width(), HEADER_H));

            let header_bg = egui::Color32::from_gray(30);
            ui.painter().rect_filled(header_rect, 0.0, header_bg);

            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(header_rect), |ui| {
                ui.set_clip_rect(header_rect);
                ui.horizontal(|ui| {
                    // Clear “this lane belongs to this track + param”
                    let param_label = match &track.automation_lanes[lane_idx].parameter {
                        AutomationTarget::TrackVolume => "Volume",
                        AutomationTarget::TrackPan => "Pan",
                        AutomationTarget::TrackSend(_) => "Send",
                        AutomationTarget::PluginParam { param_name, .. } => param_name.as_str(),
                    };

                    ui.label(
                        egui::RichText::new(format!("{} – {}", track.name, param_label))
                            .small()
                            .strong(),
                    );

                    ui.add_space(8.0);

                    // Mode combo (Read / Write / Touch / Latch / Off)
                    use crate::model::automation::AutomationMode;
                    let current_mode = {
                        let st = app.state.lock().unwrap();
                        st.tracks
                            .get(&track_id)
                            .and_then(|t| t.automation_lanes.get(lane_idx))
                            .map(|l| l.write_mode)
                            .unwrap_or(AutomationMode::Read)
                    };

                    let mode = current_mode;
                    // egui::ComboBox::from_id_salt(("auto_mode", track_id, lane_idx as u64))
                    //     .width(90.0)
                    //     .selected_text(match mode {
                    //         AutomationMode::Off => "Off",
                    //         AutomationMode::Read => "Read",
                    //         AutomationMode::Write => "Write",
                    //         AutomationMode::Touch => "Touch",
                    //         AutomationMode::Latch => "Latch",
                    //     })
                    //     .show_ui(ui, |ui| {
                    //         ui.selectable_value(&mut mode, AutomationMode::Read, "Read");
                    //         ui.selectable_value(&mut mode, AutomationMode::Write, "Write");
                    //         ui.selectable_value(&mut mode, AutomationMode::Touch, "Touch");
                    //         ui.selectable_value(&mut mode, AutomationMode::Latch, "Latch");
                    //         ui.selectable_value(&mut mode, AutomationMode::Off, "Off");
                    //     }); //TODO

                    if mode != current_mode {
                        let _ = app
                            .command_tx
                            .send(AudioCommand::SetAutomationMode(track_id, lane_idx, mode));
                    }

                    ui.add_space(4.0);

                    if ui
                        .button("Clear")
                        .on_hover_text("Clear all automation points in this lane")
                        .clicked()
                    {
                        let _ = app
                            .command_tx
                            .send(AudioCommand::ClearAutomationLane(track_id, lane_idx));
                    }
                });
            });

            let curve_rect = egui::Rect::from_min_max(
                egui::pos2(lane_rect.left(), lane_rect.top() + HEADER_H),
                lane_rect.max,
            );

            self.automation_hit_regions.push(lane_rect);

            while self.automation_widgets.len() <= lane_idx {
                self.automation_widgets.push(AutomationLaneWidget);
            }

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

            y += lane_h;
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
        if !self.show_clip_menu {
            return;
        }

        let ctx = ui.ctx();
        let mut close_menu = false;
        let mut popup_rect: Option<egui::Rect> = None;

        egui::Area::new(egui::Id::new("clip_context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(self.clip_menu_pos)
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    popup_rect = Some(ui.min_rect());
                    ui.set_min_width(180.0);

                    if ui.button("Cut").clicked() {
                        app.cut_selected();
                        close_menu = true;
                    }
                    if ui.button("Copy").clicked() {
                        app.copy_selected();
                        close_menu = true;
                    }
                    if ui.button("Paste").clicked() {
                        if let Some(primary_clip_id) = app.selected_clips.first().copied() {
                            if let Some(tid) = app
                                .state
                                .lock()
                                .unwrap()
                                .clips_by_id
                                .get(&primary_clip_id)
                                .map(|r| r.track_id)
                            {
                                app.selected_track = tid;
                            }
                        }
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

                    if let Some(primary_clip_id) = app.selected_clips.first().copied() {
                        let is_midi = app
                            .state
                            .lock()
                            .unwrap()
                            .clips_by_id
                            .get(&primary_clip_id)
                            .map_or(false, |r| r.is_midi);
                        if is_midi {
                            ui.separator();
                            if ui.button("Toggle Loop").clicked() {
                                let enabled = {
                                    let st = app.state.lock().unwrap();
                                    st.find_clip(primary_clip_id)
                                        .and_then(|(track, loc)| {
                                            if let crate::project::ClipLocation::Midi(idx) = loc {
                                                track.midi_clips.get(idx).map(|c| !c.loop_enabled)
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(true)
                                };
                                let _ = app.command_tx.send(AudioCommand::ToggleClipLoop {
                                    clip_id: primary_clip_id,
                                    enabled,
                                });
                                close_menu = true;
                            }

                            ui.separator();
                            ui.label("Quantize");

                            let (mut q_grid, mut q_strength, mut q_swing, mut q_enabled) = {
                                app.state
                                    .lock()
                                    .unwrap()
                                    .find_clip(primary_clip_id)
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
                            };

                            ui.horizontal(|ui| {
                                ui.label("Grid:");
                                const GRIDS: [(&str, f32); 6] = [
                                    ("1/1", 1.0),
                                    ("1/2", 0.5),
                                    ("1/4", 0.25),
                                    ("1/8", 0.125),
                                    ("1/16", 0.0625),
                                    ("1/32", 0.03125),
                                ];
                                for (label, g) in GRIDS {
                                    ui.selectable_value(&mut q_grid, g, label);
                                }
                            });
                            ui.add(egui::Slider::new(&mut q_strength, 0.0..=1.0).text("Strength"));
                            ui.add(egui::Slider::new(&mut q_swing, -0.5..=0.5).text("Swing"));
                            ui.checkbox(&mut q_enabled, "Enabled");

                            if ui.button("Apply Quantize").clicked() {
                                let _ = app.command_tx.send(AudioCommand::SetClipQuantize {
                                    clip_id: primary_clip_id,
                                    grid: q_grid,
                                    strength: q_strength,
                                    swing: q_swing,
                                    enabled: q_enabled,
                                });
                                close_menu = true;
                            }

                            ui.separator();
                            if ui.button("Duplicate (independent)").clicked() {
                                let _ = app.command_tx.send(AudioCommand::DuplicateMidiClip {
                                    clip_id: primary_clip_id,
                                });
                                close_menu = true;
                            }
                            if ui.button("Duplicate as Alias").clicked() {
                                let _ =
                                    app.command_tx.send(AudioCommand::DuplicateMidiClipAsAlias {
                                        clip_id: primary_clip_id,
                                    });
                                close_menu = true;
                            }

                            let is_alias = {
                                let st = app.state.lock().unwrap();
                                st.find_clip(primary_clip_id)
                                    .and_then(|(track, loc)| {
                                        if let crate::project::ClipLocation::Midi(idx) = loc {
                                            track.midi_clips.get(idx).and_then(|c| c.pattern_id)
                                        } else {
                                            None
                                        }
                                    })
                                    .is_some()
                            };
                            if ui
                                .add_enabled(is_alias, egui::Button::new("Make Unique"))
                                .clicked()
                            {
                                let _ = app.command_tx.send(AudioCommand::MakeClipUnique {
                                    clip_id: primary_clip_id,
                                });
                                close_menu = true;
                            }
                        }
                    }
                });
            });

        // close on any outside click
        let outside_clicked = ui.ctx().input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .map(|p| popup_rect.map(|r| !r.contains(p)).unwrap_or(true))
                    .unwrap_or(true)
        });

        if close_menu || outside_clicked {
            self.show_clip_menu = false;
        }
    }

    pub fn compute_project_end_beats(&self, app: &super::app::YadawApp) -> f64 {
        let state = app.state.lock().unwrap();
        state
            .tracks
            .values()
            .fold(DEFAULT_MIN_PROJECT_BEATS, |max_beat, t| {
                let audio_max = t
                    .audio_clips
                    .iter()
                    .fold(0.0, |m: f64, c| m.max(c.start_beat + c.length_beats));
                let midi_max = t
                    .midi_clips
                    .iter()
                    .fold(0.0, |m: f64, c| m.max(c.start_beat + c.length_beats));
                max_beat.max(audio_max).max(midi_max)
            })
    }

    fn x_to_beat(&self, rect: egui::Rect, x: f32) -> f64 {
        ((x - rect.left()) + self.scroll_x) as f64 / self.zoom_x as f64
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
            let new_x_at_anchor_beat = (anchor_beat as f32) * self.zoom_x;
            let desired_scroll_x = (new_x_at_anchor_beat - (anchor_x - rect.left())).max(0.0);
            self.scroll_x = desired_scroll_x;
        }
    }

    fn snap_beat(
        &self,
        ui: &egui::Ui,
        _rect: egui::Rect,
        beat: f64,
        app: &super::app::YadawApp,
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
        }

        // Clip edges (starts/ends)
        if self.snap_to_clips {
            let state = app.state.lock().unwrap();
            for &tid in &state.track_order {
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
        let mut best: Option<f64> = None;
        let mut best_d = thresh_beats;

        for &cand in &candidates {
            let d = (cand - beat).abs();
            if d < best_d {
                best = Some(cand);
                best_d = d;
            }
        }

        if let Some(best_cand) = best {
            (best_cand, Some(best_cand))
        } else {
            (beat, None)
        }
    }

    fn clip_rect_for_audio(&self, track_rect: egui::Rect, clip: &AudioClip) -> egui::Rect {
        let clip_x = self.beat_to_x(track_rect, clip.start_beat);
        let clip_w = clip.length_beats as f32 * self.zoom_x;
        egui::Rect::from_min_size(
            egui::pos2(clip_x, track_rect.top() + 20.0),
            egui::vec2(clip_w, self.track_height - 25.0),
        )
    }

    fn clip_rect_for_midi(&self, track_rect: egui::Rect, clip: &MidiClip) -> egui::Rect {
        let clip_x = self.beat_to_x(track_rect, clip.start_beat);
        let clip_w = clip.length_beats as f32 * self.zoom_x;
        egui::Rect::from_min_size(
            egui::pos2(clip_x, track_rect.top() + 5.0),
            egui::vec2(clip_w, self.track_height - 10.0),
        )
    }

    fn auto_pan_during_drag(&mut self, rect: egui::Rect, pointer_x: f32) {
        let margin = 48.0;
        let step_base = (self.zoom_x * 0.25).clamp(4.0, 40.0);

        if pointer_x > rect.right() - margin {
            let t = ((pointer_x - (rect.right() - margin)) / margin).clamp(0.0, 1.0);
            self.scroll_x += step_base * (0.25 + 0.75 * t);
        } else if pointer_x < rect.left() + margin {
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

                let tid = self.drag_target_track.or_else(|| {
                    self.last_track_blocks
                        .iter()
                        .find(|(_, r)| r.contains(pos))
                        .map(|(id, _)| *id)
                });
                let (target_clip_area, _target_track_id) = if let Some(tid) = tid {
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
                                ClipLocation::Midi(idx) => (
                                    track.midi_clips[idx].length_beats,
                                    true,
                                    track.midi_clips[idx].name.as_str(),
                                ),
                                ClipLocation::Audio(idx) => (
                                    track.audio_clips[idx].length_beats,
                                    false,
                                    track.audio_clips[idx].name.as_str(),
                                ),
                            };
                            let new_start = (*orig_start + delta).max(0.0);
                            let x = self.beat_to_x(target_clip_area, new_start);
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
        if app.selected_clips.is_empty() {
            return;
        }

        let mods = ui.input(|i| i.modifiers);
        let pressed = |k| ui.input(|i| i.key_pressed(k));

        let small_step = self.grid_snap.max(0.0001) as f64;
        let big_step = 1.0;
        let step = if mods.shift { big_step } else { small_step };

        let mut move_clips = |delta: f64| {
            if delta.abs() < f64::EPSILON {
                return;
            }
            // Keep it simple (no need of checks)
            app.push_undo();
            for &cid in &app.selected_clips {
                let st = app.state.lock().unwrap();
                if let Some((track, loc)) = st.find_clip(cid) {
                    let (is_midi, start) = match loc {
                        ClipLocation::Midi(i) => (true, track.midi_clips[i].start_beat),
                        ClipLocation::Audio(i) => (false, track.audio_clips[i].start_beat),
                    };
                    let new_start = (start + delta).max(0.0);
                    drop(st);
                    let cmd = if is_midi {
                        AudioCommand::MoveMidiClip {
                            clip_id: cid,
                            new_start,
                        }
                    } else {
                        AudioCommand::MoveAudioClip {
                            clip_id: cid,
                            new_start,
                        }
                    };
                    let _ = app.command_tx.send(cmd);
                }
            }
        };

        // Nudge left/right
        if pressed(egui::Key::ArrowLeft) {
            move_clips(-step);
        }
        if pressed(egui::Key::ArrowRight) {
            move_clips(step);
        }

        // Resize with Cmd/Ctrl (+Shift for left edge)
        if mods.command || mods.ctrl {
            if pressed(egui::Key::ArrowRight) || pressed(egui::Key::ArrowLeft) {
                let resize_delta = if pressed(egui::Key::ArrowRight) {
                    step
                } else {
                    -step
                };
                app.push_undo();
                for &cid in &app.selected_clips {
                    let st = app.state.lock().unwrap();
                    if let Some((track, loc)) = st.find_clip(cid) {
                        let (is_midi, start, len) = match loc {
                            ClipLocation::Midi(i) => (
                                true,
                                track.midi_clips[i].start_beat,
                                track.midi_clips[i].length_beats,
                            ),
                            ClipLocation::Audio(i) => (
                                false,
                                track.audio_clips[i].start_beat,
                                track.audio_clips[i].length_beats,
                            ),
                        };
                        drop(st);
                        let new_len = (len + resize_delta).max(self.grid_snap as f64);
                        let cmd = if is_midi {
                            AudioCommand::ResizeMidiClip {
                                clip_id: cid,
                                new_start: start,
                                new_length: new_len,
                            }
                        } else {
                            AudioCommand::ResizeAudioClip {
                                clip_id: cid,
                                new_start: start,
                                new_length: new_len,
                            }
                        };
                        let _ = app.command_tx.send(cmd);
                    }
                }
            }
        }

        // Alt+Arrows: slip MIDI content
        if mods.alt && (pressed(egui::Key::ArrowLeft) || pressed(egui::Key::ArrowRight)) {
            let slip_delta = if pressed(egui::Key::ArrowLeft) {
                -step
            } else {
                step
            };
            app.push_undo();
            for &cid in &app.selected_clips {
                let st = app.state.lock().unwrap();
                if let Some((track, loc)) = st.find_clip(cid) {
                    if let ClipLocation::Midi(i) = loc {
                        let offset = track.midi_clips[i].content_offset_beats;
                        drop(st);
                        let _ = app.command_tx.send(AudioCommand::SetClipContentOffset {
                            clip_id: cid,
                            new_offset: offset + slip_delta,
                        });
                    }
                }
            }
        }

        // Up/Down: move to prev/next compatible track

        if pressed(egui::Key::ArrowUp) || pressed(egui::Key::ArrowDown) {
            let dir = if pressed(egui::Key::ArrowUp) {
                -1isize
            } else {
                1isize
            };
            app.push_undo();

            let st = app.state.lock().unwrap();
            let order = &st.track_order;
            if let Some(&first_cid) = app.selected_clips.first() {
                if let Some(clip_ref) = st.clips_by_id.get(&first_cid) {
                    if let Some(cur_ix) = order.iter().position(|&tid| tid == clip_ref.track_id) {
                        // Find the next compatible track
                        let mut next_ix = (cur_ix as isize + dir) as usize;
                        while let Some(&target_tid) = order.get(next_ix) {
                            if let Some(target_track) = st.tracks.get(&target_tid) {
                                // Check if track types are compatible
                                if matches!(target_track.track_type, TrackType::Midi)
                                    == clip_ref.is_midi
                                {
                                    for &cid in &app.selected_clips {
                                        let st2 = app.state.lock().unwrap();
                                        // Use a match to safely get the start beat
                                        if let Some((track, loc)) = st2.find_clip(cid) {
                                            let start_beat = match loc {
                                                ClipLocation::Midi(i) => {
                                                    track.midi_clips[i].start_beat
                                                }
                                                ClipLocation::Audio(i) => {
                                                    track.audio_clips[i].start_beat
                                                }
                                            };
                                            drop(st2);

                                            let cmd = if clip_ref.is_midi {
                                                AudioCommand::MoveMidiClipToTrack {
                                                    clip_id: cid,
                                                    dest_track_id: target_tid,
                                                    new_start: start_beat,
                                                }
                                            } else {
                                                AudioCommand::MoveAudioClipToTrack {
                                                    clip_id: cid,
                                                    dest_track_id: target_tid,
                                                    new_start: start_beat,
                                                }
                                            };
                                            let _ = app.command_tx.send(cmd);
                                        }
                                    }
                                    return; // Moved clips, so exit
                                }
                            }
                            next_ix = (next_ix as isize + dir) as usize;
                        }
                    }
                }
            }
        }
    }

    fn draw_resize_previews(
        &self,
        ui: &mut egui::Ui,
        app: &mut super::app::YadawApp,
        rect: egui::Rect,
    ) {
        if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
            let interaction = self.timeline_interaction.as_ref();

            let (clip_id, new_start_beat, new_end_beat) = match interaction {
                Some(TimelineInteraction::ResizeClipLeft {
                    clip_id,
                    original_end_beat,
                }) => {
                    let candidate = self.x_to_beat(rect, pos.x).max(0.0);
                    let (snapped, _) = self.snap_beat(ui, rect, candidate, app, None);
                    let min_len = self.grid_snap.max(0.03125) as f64;
                    let new_start = snapped.min(*original_end_beat - min_len);
                    (*clip_id, new_start, *original_end_beat)
                }
                Some(TimelineInteraction::ResizeClipRight {
                    clip_id,
                    original_start_beat,
                }) => {
                    let candidate = self.x_to_beat(rect, pos.x).max(0.0);
                    let (snapped, _) = self.snap_beat(ui, rect, candidate, app, None);
                    let min_len = self.grid_snap.max(0.03125) as f64;
                    let new_end = snapped.max(*original_start_beat + min_len);
                    (*clip_id, *original_start_beat, new_end)
                }
                _ => return, // Not a resize interaction, so do nothing
            };

            let state = app.state.lock().unwrap();
            if let Some(clip_ref) = state.clips_by_id.get(&clip_id) {
                // Find the track's screen rectangle
                if let Some((_, track_block)) = self
                    .last_track_blocks
                    .iter()
                    .find(|(tid, _)| *tid == clip_ref.track_id)
                {
                    let painter = ui.painter();
                    let new_len_beats = new_end_beat - new_start_beat;

                    let x = self.beat_to_x(*track_block, new_start_beat);
                    let w = (new_len_beats as f32 * self.zoom_x).max(2.0);

                    let (y_offset, height_offset) = if clip_ref.is_midi {
                        (5.0, 10.0) // Matches draw_midi_clip
                    } else {
                        (20.0, 25.0) // Matches draw_audio_clip
                    };

                    let ghost_rect = egui::Rect::from_min_size(
                        egui::pos2(x, track_block.top() + y_offset),
                        egui::vec2(w, self.track_height - height_offset),
                    );

                    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 150, 255));

                    painter.rect_stroke(ghost_rect, 4.0, stroke, egui::StrokeKind::Outside);
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
