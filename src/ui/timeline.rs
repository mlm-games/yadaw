use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::*;
use crate::constants::{DEFAULT_MIDI_CLIP_LEN, DEFAULT_MIN_PROJECT_BEATS};
use crate::messages::AudioCommand;
use crate::model::{AudioClip, MidiClip, Track};
use crate::ui::automation_lane::{AutomationAction, AutomationLaneWidget};
use smallvec::SmallVec;

pub struct TimelineView {
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub grid_snap: f32,
    pub show_automation: bool,
    pub auto_scroll: bool,

    // Interaction state
    timeline_interaction: Option<TimelineInteraction>,
    automation_widgets: Vec<AutomationLaneWidget>,

    // Clip selection
    show_clip_menu: bool,
    clip_menu_pos: egui::Pos2,

    // Appearance
    track_height: f32,
    min_track_height: f32,
    max_track_height: f32,

    last_view_width: f32,
    pending_clip_undo: bool,
}

#[derive(Clone)]
enum TimelineInteraction {
    DragClip {
        clips_and_starts: Vec<(usize, usize, f64)>,
        start_drag_beat: f64,
    },
    ResizeClipLeft {
        track_id: usize,
        clip_id: usize,
        original_end_beat: f64,
    },
    ResizeClipRight {
        track_id: usize,
        clip_id: usize,
        original_start_beat: f64,
    },
    SelectionBox {
        start_pos: egui::Pos2,
        current_pos: egui::Pos2,
    },
    LoopDragStart {
        offset_beats: f64,
    }, // dragging loop start edge
    LoopDragEnd {
        offset_beats: f64,
    }, // dragging loop end edge
    LoopCreate {
        anchor_beat: f64,
    }, // click-drag to create/replace loop
    SlipContent {
        track_id: usize,
        clip_id: usize,
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
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.heading("Timeline");

        // Timeline toolbar
        self.draw_toolbar(ui, app);

        ui.separator();

        // Main timeline area
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_timeline(ui, app);
            });

        // Handle context menus
        self.draw_context_menus(ui, app);

        // Auto-scroll if playing
        if self.auto_scroll && app.audio_state.playing.load(Ordering::Relaxed) {
            self.update_auto_scroll(app);
        }
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, app: &super::app::YadawApp) {
        ui.horizontal(|ui| {
            // Zoom controls
            ui.label("Zoom:");
            if ui.button("−").clicked() {
                self.zoom_x = (self.zoom_x * 0.8).max(10.0);
            }
            if ui.button("╋").clicked() {
                self.zoom_x = (self.zoom_x * 1.25).min(500.0);
            }
            ui.label(format!("{:.0}px/beat", self.zoom_x));

            ui.separator();

            // Track height
            ui.label("Track Height:");
            if ui
                .add(
                    egui::Slider::new(
                        &mut self.track_height,
                        self.min_track_height..=self.max_track_height,
                    )
                    .show_value(false),
                )
                .changed()
            {
                // Track height changed
            }

            ui.separator();

            // Grid snap
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

            // View options
            ui.checkbox(&mut self.show_automation, "Show Automation");
            ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
        });
    }

    fn draw_timeline(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        // Determine total beats to show (content + margin)
        let project_end_beats = self
            .compute_project_end_beats(app)
            .max(DEFAULT_MIN_PROJECT_BEATS);
        let margin_beats = 8.0;
        let content_beats = project_end_beats + margin_beats;

        // Size to content or viewport, whichever is larger
        let view_w = ui.available_width();
        self.last_view_width = view_w;
        let total_width = view_w.max(content_beats as f32 * self.zoom_x);
        let num_tracks = app.state.lock().unwrap().tracks.len();
        let total_height = (num_tracks as f32 * self.track_height).max(1.0);

        let (response, painter) = ui.allocate_painter(
            egui::vec2(total_width, total_height),
            egui::Sense::click_and_drag(),
        );

        // Mark clips as active target when the timeline is hot
        if response.hovered() || response.is_pointer_button_down_on() {
            app.active_edit_target = super::app::ActiveEditTarget::Clips;
        }

        let rect = response.rect;

        // Grid and loop region
        self.draw_grid(&painter, rect, app.state.lock().unwrap().bpm);

        // Tracks
        {
            let binding = app.state.clone();
            let mut state = binding.lock().unwrap();
            for track_idx in 0..num_tracks {
                let track_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(0.0, track_idx as f32 * self.track_height),
                    egui::vec2(rect.width(), self.track_height),
                );
                self.draw_track(
                    &painter,
                    ui,
                    track_rect,
                    &mut state.tracks[track_idx],
                    track_idx,
                    app,
                );
            }
        }

        self.draw_loop_region(&painter, rect, app);

        // Playhead overlay
        {
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
        }

        if self.pending_clip_undo {
            app.push_undo();
            self.pending_clip_undo = false;
        }

        // Interactions
        self.handle_timeline_interaction(&response, ui, app);
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: egui::Rect, bpm: f32) {
        let ruler_h = 18.0;

        // Ruler background
        painter.rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + ruler_h)),
            0.0,
            egui::Color32::from_gray(22),
        );

        // Vertical lines (beats)
        let beats_visible = (rect.width() / self.zoom_x) as i32 + 2;
        let start_beat = (self.scroll_x / self.zoom_x) as i32;

        for beat in start_beat..(start_beat + beats_visible) {
            let x = rect.left() + (beat as f32 * self.zoom_x - self.scroll_x);
            if x < rect.left() || x > rect.right() {
                continue;
            }
            let is_bar = beat % 4 == 0;
            let color = if is_bar {
                egui::Color32::from_gray(70)
            } else {
                egui::Color32::from_gray(45)
            };
            // Tick on ruler
            painter.line_segment(
                [
                    egui::pos2(x, rect.top()),
                    egui::pos2(x, rect.top() + ruler_h),
                ],
                egui::Stroke::new(if is_bar { 1.5 } else { 1.0 }, color),
            );

            // Beat numbers on ruler for bars
            if is_bar {
                painter.text(
                    egui::pos2(x + 3.0, rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}", beat / 4 + 1),
                    egui::FontId::default(),
                    egui::Color32::from_gray(160),
                );
            }

            // Full-height grid lines in tracks area (below ruler)
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + ruler_h),
                    egui::pos2(x, rect.bottom()),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(40)),
            );
        }

        // Horizontal lines (track separators)
        let num_tracks = ((rect.height() - ruler_h) / self.track_height) as i32;
        for track in 0..=num_tracks {
            let y = rect.top() + ruler_h + track as f32 * self.track_height;
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(40)),
            );
        }
    }

    fn draw_track(
        &mut self,
        painter: &egui::Painter,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        track: &mut Track,
        track_idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        // Track background
        let bg_color = if track_idx % 2 == 0 {
            egui::Color32::from_gray(25)
        } else {
            egui::Color32::from_gray(30)
        };
        painter.rect_filled(rect, 0.0, bg_color);

        // Track name
        painter.text(
            rect.min + egui::vec2(5.0, 5.0),
            egui::Align2::LEFT_TOP,
            &track.name,
            egui::FontId::default(),
            egui::Color32::WHITE,
        );

        // Draw clips based on track type
        if track.is_midi {
            for (clip_idx, clip) in track.midi_clips.iter().enumerate() {
                self.draw_midi_clip(painter, ui, rect, clip, track_idx, clip_idx, app);
            }
        } else {
            for (clip_idx, clip) in track.audio_clips.iter().enumerate() {
                self.draw_audio_clip(painter, ui, rect, clip, track_idx, clip_idx, app);
            }
        }

        // Draw automation lanes if visible
        if self.show_automation {
            self.draw_automation_lanes(ui, rect, track, track_idx, app);
        }
    }

    fn draw_audio_clip(
        &mut self,
        painter: &egui::Painter,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        clip: &AudioClip,
        track_idx: usize,
        clip_idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_width = clip.length_beats as f32 * self.zoom_x;

        let clip_rect = egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 20.0),
            egui::vec2(clip_width, self.track_height - 25.0),
        );

        // Only draw if visible
        if clip_rect.right() < track_rect.left() || clip_rect.left() > track_rect.right() {
            return;
        }

        // Draw waveform
        crate::waveform::draw_waveform(painter, clip_rect, clip, self.zoom_x, self.scroll_x);

        // Selection highlight
        if app.selected_clips.contains(&(track_idx, clip_idx)) {
            painter.rect_stroke(
                clip_rect,
                2.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }

        // Interaction
        let response = ui.interact(
            clip_rect,
            ui.id().with((track_idx, clip_idx)),
            egui::Sense::click_and_drag(),
        );

        self.handle_clip_interaction(response, track_idx, ui, clip_idx, clip_rect, app);
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

        // Semi-transparent overlay below the ruler
        let overlay = egui::Rect::from_min_max(
            egui::pos2(start_x, rect.top() + 18.0),
            egui::pos2(end_x, rect.bottom()),
        );
        painter.rect_filled(
            overlay,
            0.0,
            egui::Color32::from_rgba_premultiplied(100, 150, 255, 28),
        );

        // Loop markers (full height)
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
        track_idx: usize,
        clip_idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        let clip_x = clip.start_beat as f32 * self.zoom_x - self.scroll_x;
        let clip_width = clip.length_beats as f32 * self.zoom_x;

        let clip_rect = egui::Rect::from_min_size(
            track_rect.min + egui::vec2(clip_x, 5.0),
            egui::vec2(clip_width, self.track_height - 10.0),
        );

        // Only draw if visible
        if clip_rect.right() < track_rect.left() || clip_rect.left() > track_rect.right() {
            return;
        }

        // Draw clip background
        let color = if let Some((r, g, b)) = clip.color {
            egui::Color32::from_rgb(r, g, b)
        } else {
            egui::Color32::from_rgb(100, 150, 200)
        };
        painter.rect_filled(clip_rect, 4.0, color);

        // Draw MIDI notes preview
        let content_len = clip.content_len_beats.max(0.000001);
        let inst_len = clip.length_beats.max(0.0);
        let clip_left = clip_rect.left();

        // Visible window in beats relative to clip start
        let vis_start_rel: f64 =
            ((track_rect.left() - clip_left) as f64 + self.scroll_x as f64) / self.zoom_x as f64;
        let vis_end_rel: f64 =
            ((track_rect.right() - clip_left) as f64 + self.scroll_x as f64) / self.zoom_x as f64;

        let content_len = clip.content_len_beats.max(0.000001);

        // Compute repeat range that intersects the visible window
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

        let color = if let Some((r, g, b)) = clip.color {
            egui::Color32::from_rgb(r, g, b)
        } else {
            egui::Color32::from_rgb(100, 150, 200)
        };
        painter.rect_filled(clip_rect, 4.0, color);

        // Optional: draw content segment dividers when looping
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
                let s_loc = (note.start + offset as f64).rem_euclid(content_len);
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

        // Clip name
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
            ui.id().with(("midi_clip", track_idx, clip_idx)),
            egui::Sense::click_and_drag(),
        );

        if response.double_clicked() {
            // Open in piano roll
            app.selected_track = track_idx;
            app.open_midi_clip_in_piano_roll(clip_idx);
        }

        // MIDI clips' selection/drag/resize/delete
        self.handle_clip_interaction(response, track_idx, ui, clip_idx, clip_rect, app);
    }

    fn handle_clip_interaction(
        &mut self,
        response: egui::Response,
        track_idx: usize,
        ui: &mut egui::Ui,
        clip_idx: usize,
        clip_rect: egui::Rect,
        app: &mut super::app::YadawApp,
    ) {
        // Select on click
        if response.clicked() {
            if !app.selected_clips.contains(&(track_idx, clip_idx)) {
                if !response.ctx.input(|i| i.modifiers.ctrl) {
                    app.selected_clips.clear();
                }
                app.selected_clips.push((track_idx, clip_idx));
            }
        }

        // Context menu on right-click
        if response.secondary_clicked() {
            self.show_clip_menu = true;
            self.clip_menu_pos = response.interact_pointer_pos().unwrap_or_default();
            if !app.selected_clips.contains(&(track_idx, clip_idx)) {
                app.selected_clips.clear();
                app.selected_clips.push((track_idx, clip_idx));
            }
        }

        // Edge hover feedback
        let edge_threshold = 5.0;
        let hover_left = response
            .hover_pos()
            .map_or(false, |p| (p.x - clip_rect.left()).abs() < edge_threshold);
        let hover_right = response
            .hover_pos()
            .map_or(false, |p| (clip_rect.right() - p.x).abs() < edge_threshold);

        if hover_left || hover_right {
            response
                .ctx
                .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        // Helper to convert X -> beat
        let beat_at = |x: f32, rect_left: f32, scroll_x: f32, zoom_x: f32| -> f64 {
            let rel = (x - rect_left) + scroll_x;
            (rel / zoom_x) as f64
        };

        // Begin drag/resize
        if response.drag_started() && self.timeline_interaction.is_none() {
            self.pending_clip_undo = true;

            // Access clip timing
            let state = app.state.lock().unwrap();
            let clip_is_midi = state
                .tracks
                .get(track_idx)
                .map(|t| t.is_midi)
                .unwrap_or(false);
            let (clip_start, clip_len) = if let Some(track) = state.tracks.get(track_idx) {
                if clip_is_midi {
                    if let Some(c) = track.midi_clips.get(clip_idx) {
                        (c.start_beat, c.length_beats)
                    } else {
                        (0.0, 0.0)
                    }
                } else {
                    if let Some(c) = track.audio_clips.get(clip_idx) {
                        (c.start_beat, c.length_beats)
                    } else {
                        (0.0, 0.0)
                    }
                }
            } else {
                (0.0, 0.0)
            };
            drop(state);

            // Ruler-independent beat under pointer
            let start_beat_under_mouse = response
                .interact_pointer_pos()
                .map(|pos| beat_at(pos.x, response.rect.left(), self.scroll_x, self.zoom_x))
                .unwrap_or(clip_start);

            let alt = ui.input(|i| i.modifiers.alt);
            if alt && !hover_left && !hover_right {
                // Slip content (Alt+drag)
                // Read current offset/content_len once
                let (start_offset, content_len) = {
                    let state = app.state.lock().unwrap();
                    if let Some(t) = state.tracks.get(track_idx) {
                        if let Some(c) = t.midi_clips.get(clip_idx) {
                            (c.content_offset_beats, c.content_len_beats.max(0.000001))
                        } else {
                            (0.0, 1.0)
                        }
                    } else {
                        (0.0, 1.0)
                    }
                };
                let start_mouse_beat = response
                    .interact_pointer_pos()
                    .map(|pos| {
                        let rel = (pos.x - response.rect.left()) + self.scroll_x;
                        (rel / self.zoom_x) as f64
                    })
                    .unwrap_or(0.0);
                self.timeline_interaction = Some(TimelineInteraction::SlipContent {
                    track_id: track_idx,
                    clip_id: clip_idx,
                    start_offset: start_offset.rem_euclid(content_len),
                    start_mouse_beat,
                });
                return;
            }

            if hover_left {
                // Start resizing left edge
                self.timeline_interaction = Some(TimelineInteraction::ResizeClipLeft {
                    track_id: track_idx,
                    clip_id: clip_idx,
                    original_end_beat: clip_start + clip_len,
                });
                return;
            } else if hover_right {
                // Start resizing right edge
                self.timeline_interaction = Some(TimelineInteraction::ResizeClipRight {
                    track_id: track_idx,
                    clip_id: clip_idx,
                    original_start_beat: clip_start,
                });
                return;
            } else {
                // Drag whole clip (or multi-selection)
                // Build list of (track_id, clip_id, original_start)
                let mut clips_and_starts = Vec::new();
                let state = app.state.lock().unwrap();

                // Use selection if the current clip is selected, else just this one
                let selected = if app.selected_clips.contains(&(track_idx, clip_idx)) {
                    app.selected_clips.clone()
                } else {
                    vec![(track_idx, clip_idx)]
                };

                for (t_id, c_id) in selected {
                    if let Some(t) = state.tracks.get(t_id) {
                        if t.is_midi {
                            if let Some(c) = t.midi_clips.get(c_id) {
                                clips_and_starts.push((t_id, c_id, c.start_beat));
                            }
                        } else {
                            if let Some(c) = t.audio_clips.get(c_id) {
                                clips_and_starts.push((t_id, c_id, c.start_beat));
                            }
                        }
                    }
                }
                drop(state);

                self.timeline_interaction = Some(TimelineInteraction::DragClip {
                    clips_and_starts,
                    start_drag_beat: start_beat_under_mouse,
                });
            }
        }

        // End interaction if the user ends drag over this clip
        if response.drag_stopped() {
            self.timeline_interaction = None;
        }
    }

    fn draw_playhead(&self, painter: &egui::Painter, rect: egui::Rect, app: &super::app::YadawApp) {
        let position = app.audio_state.get_position();
        let sample_rate = app.audio_state.sample_rate.load();
        let bpm = app.audio_state.bpm.load();
        if sample_rate <= 0.0 || bpm <= 0.0 {
            return;
        }

        let current_beat = (position / sample_rate as f64) * (bpm as f64 / 60.0);
        let x = rect.left() + (current_beat as f32 * self.zoom_x - self.scroll_x);

        if x >= rect.left() && x <= rect.right() {
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 80, 80)),
            );
        }
    }

    fn compute_project_end_beats(&self, app: &super::app::YadawApp) -> f64 {
        let state = app.state.lock().unwrap();
        let mut max_beat: f64 = DEFAULT_MIN_PROJECT_BEATS;
        for t in &state.tracks {
            for c in &t.audio_clips {
                max_beat = max_beat.max(c.start_beat + c.length_beats);
            }
            for c in &t.midi_clips {
                max_beat = max_beat.max(c.start_beat + c.length_beats);
            }
        }
        max_beat
    }

    fn beats_to_samples(&self, beats: f64, app: &super::app::YadawApp) -> f64 {
        let sr = app.audio_state.sample_rate.load() as f64;
        let bpm = app.audio_state.bpm.load() as f64;
        if bpm <= 0.0 || sr <= 0.0 {
            return 0.0;
        }
        beats * (60.0 / bpm) * sr
    }

    fn handle_timeline_interaction(
        &mut self,
        response: &egui::Response,
        ui: &mut egui::Ui,
        app: &mut super::app::YadawApp,
    ) {
        let rect = response.rect;
        let ruler_h = 18.0;

        // Helpers
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
        let min_len = (self.grid_snap.max(0.03125)) as f64; // at least 1/32 note

        // Start drag: handle ruler (loop) interactions first
        if response.drag_started() && self.timeline_interaction.is_none() {
            if let Some(pos) = response.interact_pointer_pos() {
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
                    // Create/replace loop by dragging on ruler
                    self.timeline_interaction = Some(TimelineInteraction::LoopCreate {
                        anchor_beat: target,
                    });
                    return;
                }
            }
        }

        // During drag
        if response.dragged() {
            if let Some(pos) = response.hover_pos() {
                match &mut self.timeline_interaction {
                    // Loop start edge drag
                    Some(TimelineInteraction::LoopDragStart { offset_beats }) => {
                        let mut new_start = (beat_at(pos.x) - *offset_beats).max(0.0);
                        new_start = snap(new_start, self.grid_snap);
                        let end = app.audio_state.loop_end.load();
                        if end > new_start {
                            let _ = app
                                .command_tx
                                .send(AudioCommand::SetLoopRegion(new_start, end));
                            let _ = app.command_tx.send(AudioCommand::SetLoopEnabled(true));
                        }
                    }
                    // Loop end edge drag
                    Some(TimelineInteraction::LoopDragEnd { offset_beats }) => {
                        let mut new_end = (beat_at(pos.x) - *offset_beats).max(0.0);
                        new_end = snap(new_end, self.grid_snap);
                        let start = app.audio_state.loop_start.load();
                        if new_end > start {
                            let _ = app
                                .command_tx
                                .send(AudioCommand::SetLoopRegion(start, new_end));
                            let _ = app.command_tx.send(AudioCommand::SetLoopEnabled(true));
                        }
                    }
                    // Loop create drag
                    Some(TimelineInteraction::LoopCreate { anchor_beat }) => {
                        let mut current = beat_at(pos.x).max(0.0);
                        current = snap(current, self.grid_snap);
                        let (s, e) = if current >= *anchor_beat {
                            (*anchor_beat, current)
                        } else {
                            (current, *anchor_beat)
                        };
                        if e > s {
                            let _ = app.command_tx.send(AudioCommand::SetLoopRegion(s, e));
                            let _ = app.command_tx.send(AudioCommand::SetLoopEnabled(true));
                        }
                    }
                    // Clip drag move (multi-clip aware)
                    Some(TimelineInteraction::DragClip {
                        clips_and_starts,
                        start_drag_beat,
                    }) => {
                        let mut current = beat_at(pos.x);
                        // delta relative to where the drag started in timeline beats
                        let mut delta = current - *start_drag_beat;
                        delta = snap(delta, self.grid_snap);

                        for (t_id, c_id, original_start) in clips_and_starts.iter().copied() {
                            let new_start = (original_start + delta).max(0.0);
                            // Check clip type
                            let state = app.state.lock().unwrap();
                            if let Some(t) = state.tracks.get(t_id) {
                                if t.is_midi {
                                    let _ = app
                                        .command_tx
                                        .send(AudioCommand::MoveMidiClip(t_id, c_id, new_start));
                                } else {
                                    let _ = app
                                        .command_tx
                                        .send(AudioCommand::MoveAudioClip(t_id, c_id, new_start));
                                }
                            }
                        }
                    }
                    // Resize left edge (change start & length)
                    Some(TimelineInteraction::ResizeClipLeft {
                        track_id,
                        clip_id,
                        original_end_beat,
                    }) => {
                        let mut drag_at = snap(beat_at(pos.x).max(0.0), self.grid_snap);

                        // Clamp to end - min_len
                        let new_start = drag_at.min(*original_end_beat - min_len);
                        let new_len = (*original_end_beat - new_start).max(min_len);

                        let state = app.state.lock().unwrap();
                        let is_midi = state
                            .tracks
                            .get(*track_id)
                            .map(|t| t.is_midi)
                            .unwrap_or(false);
                        drop(state);

                        if is_midi {
                            let _ = app.command_tx.send(AudioCommand::ResizeMidiClip(
                                *track_id, *clip_id, new_start, new_len,
                            ));
                        } else {
                            let _ = app.command_tx.send(AudioCommand::ResizeAudioClip(
                                *track_id, *clip_id, new_start, new_len,
                            ));
                        }
                    }
                    // Resize right edge (change length only)
                    Some(TimelineInteraction::ResizeClipRight {
                        track_id,
                        clip_id,
                        original_start_beat,
                    }) => {
                        let mut drag_at = snap(beat_at(pos.x).max(0.0), self.grid_snap);

                        // Enforce minimum length
                        let new_end = drag_at.max(*original_start_beat + min_len);
                        let new_len = (new_end - *original_start_beat).max(min_len);

                        let state = app.state.lock().unwrap();
                        let is_midi = state
                            .tracks
                            .get(*track_id)
                            .map(|t| t.is_midi)
                            .unwrap_or(false);
                        drop(state);

                        if is_midi {
                            let _ = app.command_tx.send(AudioCommand::ResizeMidiClip(
                                *track_id,
                                *clip_id,
                                *original_start_beat,
                                new_len,
                            ));
                        } else {
                            let _ = app.command_tx.send(AudioCommand::ResizeAudioClip(
                                *track_id,
                                *clip_id,
                                *original_start_beat,
                                new_len,
                            ));
                        }
                    }
                    Some(TimelineInteraction::SlipContent {
                        track_id,
                        clip_id,
                        start_offset,
                        start_mouse_beat,
                    }) => {
                        if let Some(pos) = response.hover_pos() {
                            let cur =
                                ((pos.x - response.rect.left()) + self.scroll_x) / self.zoom_x;
                            let delta = (cur as f64) - *start_mouse_beat;
                            // Throttle sends like you do elsewhere (30 ms)
                            let mem_root = egui::Id::new(("slip", *track_id, *clip_id));
                            let due = {
                                let last =
                                    ui.ctx().memory(|m| m.data.get_temp::<Instant>(mem_root));
                                last.map_or(true, |t| {
                                    Instant::now().duration_since(t) >= Duration::from_millis(30)
                                })
                            };
                            if due {
                                // Get content_len once to wrap server-side too, but send raw desired
                                let new_off = *start_offset + delta;
                                let _ = app.command_tx.send(
                                    crate::messages::AudioCommand::SetClipContentOffset(
                                        *track_id, *clip_id, new_off,
                                    ),
                                );
                                ui.ctx()
                                    .memory_mut(|m| m.data.insert_temp(mem_root, Instant::now()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // End interactions
        if response.drag_stopped() {
            self.timeline_interaction = None;
        }

        // Click on ruler to set playhead (without loop changes)
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let on_ruler = pos.y >= rect.top() && pos.y <= rect.top() + ruler_h;
                if on_ruler {
                    let mut beat = beat_at(pos.x).max(0.0);
                    beat = snap(beat, self.grid_snap);
                    // Convert beats -> samples and set position
                    let sr = app.audio_state.sample_rate.load() as f64;
                    let bpm = app.audio_state.bpm.load() as f64;
                    if bpm > 0.0 && sr > 0.0 {
                        let samples = beat * (60.0 / bpm) * sr;
                        let _ = app.command_tx.send(AudioCommand::SetPosition(samples));
                    }
                    return;
                }
            }
        }

        // Existing “click on empty to create MIDI clip with Ctrl” behavior
        if response.clicked() && self.timeline_interaction.is_none() {
            if let Some(pos) = response.interact_pointer_pos() {
                let grid_pos = pos - response.rect.min;
                let mut beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                beat = snap(beat as f64, self.grid_snap) as f32;
                let track_idx = (grid_pos.y / self.track_height) as usize;

                let state = app.state.lock().unwrap();
                if let Some(track) = state.tracks.get(track_idx) {
                    if track.is_midi && ui.input(|i| i.modifiers.ctrl) {
                        let _ = app.command_tx.send(AudioCommand::CreateMidiClip(
                            track_idx,
                            beat as f64,
                            DEFAULT_MIDI_CLIP_LEN,
                        ));
                    }
                }
            }
        }
    }

    fn draw_automation_lanes(
        &mut self,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        track: &mut crate::model::track::Track,
        track_idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        // Compute total height of visible lanes
        let mut visible_lanes: Vec<(usize, f32)> = track
            .automation_lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.visible)
            .map(|(i, l)| (i, l.height.max(20.0)))
            .collect();

        if visible_lanes.is_empty() {
            return;
        }

        // Start stacking from the bottom of the track
        let mut y = track_rect.bottom();
        for (lane_idx, h) in visible_lanes.iter().cloned() {
            y -= h;
            let lane_rect = egui::Rect::from_min_size(
                egui::pos2(track_rect.left(), y),
                egui::vec2(track_rect.width(), h),
            );

            // Ensure widget exists
            while self.automation_widgets.len() <= lane_idx {
                self.automation_widgets
                    .push(AutomationLaneWidget::default());
            }

            // Draw label strip on the left (80 px)
            let label_w = 80.0_f32.min(track_rect.width() * 0.25);
            let label_rect = egui::Rect::from_min_size(lane_rect.min, egui::vec2(label_w, h));
            let lane_name = match &track.automation_lanes[lane_idx].parameter {
                crate::model::automation::AutomationTarget::TrackVolume => "Volume",
                crate::model::automation::AutomationTarget::TrackPan => "Pan",
                crate::model::automation::AutomationTarget::TrackSend(_) => "Send",
                crate::model::automation::AutomationTarget::PluginParam { param_name, .. } => {
                    param_name.as_str()
                }
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

            // Widget draws and returns actions
            let actions = self.automation_widgets[lane_idx].ui(
                ui,
                &mut track.automation_lanes[lane_idx],
                curve_rect,
                self.zoom_x,
                self.scroll_x,
            );

            // Dispatch actions
            for action in actions {
                match action {
                    AutomationAction::AddPoint { beat, value } => {
                        let target = track.automation_lanes[lane_idx].parameter.clone();
                        let _ =
                            app.command_tx
                                .send(crate::messages::AudioCommand::AddAutomationPoint(
                                    track_idx, target, beat, value,
                                ));
                    }
                    AutomationAction::RemovePoint(beat) => {
                        let _ = app.command_tx.send(
                            crate::messages::AudioCommand::RemoveAutomationPoint(
                                track_idx, lane_idx, beat,
                            ),
                        );
                    }
                    AutomationAction::MovePoint {
                        old_beat,
                        new_beat,
                        new_value,
                    } => {
                        let _ = app.command_tx.send(
                            crate::messages::AudioCommand::RemoveAutomationPoint(
                                track_idx, lane_idx, old_beat,
                            ),
                        );
                        let target = track.automation_lanes[lane_idx].parameter.clone();
                        let _ =
                            app.command_tx
                                .send(crate::messages::AudioCommand::AddAutomationPoint(
                                    track_idx, target, new_beat, new_value,
                                ));
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

        // Playhead within the right 20% of the view
        let view_w = self.last_view_width.max(200.0);
        let right_margin = view_w * 0.2;
        if playhead_x > self.scroll_x + view_w - right_margin {
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

                        if ui.button("Toggle Loop").clicked() {
                            if let Some((t, c)) = app.selected_clips.first().copied() {
                                let state = app.state.lock().unwrap();
                                let enabled = state
                                    .tracks
                                    .get(t)
                                    .and_then(|tr| tr.midi_clips.get(c))
                                    .map(|cl| !cl.loop_enabled)
                                    .unwrap_or(true);
                                drop(state);
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::ToggleClipLoop(t, c, enabled));
                            }
                            close_menu = true;
                        }

                        ui.separator();

                        // Find the primary selected clip (first entry)
                        let primary = app.selected_clips.first().copied();

                        // Peek alias state once (no long locks)
                        let (is_alias, track_id, clip_id) = if let Some((t, c)) = primary {
                            let st = app.state.lock().unwrap();
                            let is_alias = st
                                .tracks
                                .get(t)
                                .and_then(|tr| tr.midi_clips.get(c))
                                .and_then(|cl| cl.pattern_id)
                                .is_some();
                            (is_alias, Some(t), Some(c))
                        } else {
                            (false, None, None)
                        };

                        // Duplicate (independent)
                        if ui.button("Duplicate (independent)").clicked() {
                            if let (Some(t), Some(c)) = (track_id, clip_id) {
                                let _ = app
                                    .command_tx
                                    .send(crate::messages::AudioCommand::DuplicateMidiClip(t, c));
                            }
                            close_menu = true;
                        }

                        // Duplicate as Alias (creates alias id if needed, duplicates as alias)
                        if ui.button("Duplicate as Alias").clicked() {
                            if let (Some(t), Some(c)) = (track_id, clip_id) {
                                let _ = app.command_tx.send(
                                    crate::messages::AudioCommand::DuplicateMidiClipAsAlias(t, c),
                                );
                            }
                            close_menu = true;
                        }

                        // Make Unique (only enabled if this clip is currently an alias)
                        let mut make_unique_btn = egui::Button::new("Make Unique");
                        if ui.add_enabled(is_alias, make_unique_btn).clicked() {
                            if let (Some(t), Some(c)) = (track_id, clip_id) {
                                let _ = app
                                    .command_tx
                                    .send(crate::messages::AudioCommand::MakeClipUnique(t, c));
                            }
                            close_menu = true;
                        }

                        ui.separator();
                        ui.label("Quantize");
                        static GRIDS: [(&str, f32); 6] = [
                            ("1/1", 1.0),
                            ("1/2", 0.5),
                            ("1/4", 0.25),
                            ("1/8", 0.125),
                            ("1/16", 0.0625),
                            ("1/32", 0.03125),
                        ];
                        let (mut grid, mut strength, mut swing, mut enabled) = {
                            let st = app.state.lock().unwrap();
                            if let Some((t, c)) = app.selected_clips.first().copied() {
                                if let Some(clip) =
                                    st.tracks.get(t).and_then(|tr| tr.midi_clips.get(c))
                                {
                                    (
                                        clip.quantize_grid,
                                        clip.quantize_strength,
                                        clip.swing,
                                        clip.quantize_enabled,
                                    )
                                } else {
                                    (0.25, 1.0, 0.0, false)
                                }
                            } else {
                                (0.25, 1.0, 0.0, false)
                            }
                        };
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
                            if let Some((t, c)) = app.selected_clips.first().copied() {
                                let _ = app.command_tx.send(AudioCommand::SetClipQuantize(
                                    t, c, grid, strength, swing, enabled,
                                ));
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
}

impl Default for TimelineView {
    fn default() -> Self {
        Self::new()
    }
}
