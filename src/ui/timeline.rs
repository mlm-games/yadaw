use std::sync::atomic::Ordering;

use super::*;
use crate::automation_lane::{AutomationAction, AutomationLaneWidget};
use crate::messages::AudioCommand;
use crate::model::{AudioClip, MidiClip, Track};

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
        if self.auto_scroll
            && app
                .audio_state
                .playing
                .load(Ordering::Relaxed)
        {
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
        let state = app.state.lock().unwrap();
        let num_tracks = state.tracks.len();
        let bpm = state.bpm;
        drop(state);

        // Calculate total size
        let total_width = ui.available_width().max(2000.0);
        let total_height = num_tracks as f32 * self.track_height;

        let (response, painter) = ui.allocate_painter(
            egui::vec2(total_width, total_height),
            egui::Sense::click_and_drag(),
        );

        let rect = response.rect;

        // Draw grid
        self.draw_grid(&painter, rect, bpm);

        // Draw loop region overlay
        self.draw_loop_region(&painter, rect, app);

        // Draw tracks
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

        // Draw playhead
        self.draw_playhead(&painter, rect, app);

        // Handle interactions
        self.handle_timeline_interaction(&response, ui, app);
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: egui::Rect, bpm: f32) {
        // Vertical lines (beats)
        let beats_visible = (rect.width() / self.zoom_x) as i32 + 2;
        let start_beat = (self.scroll_x / self.zoom_x) as i32;

        for beat in start_beat..(start_beat + beats_visible) {
            let x = rect.left() + (beat as f32 * self.zoom_x - self.scroll_x);

            if x >= rect.left() && x <= rect.right() {
                let is_bar = beat % 4 == 0;
                let color = if is_bar {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(40)
                };

                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(if is_bar { 1.5 } else { 1.0 }, color),
                );

                // Beat numbers
                if is_bar {
                    painter.text(
                        egui::pos2(x + 2.0, rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", beat / 4 + 1),
                        egui::FontId::default(),
                        egui::Color32::from_gray(100),
                    );
                }
            }
        }

        // Horizontal lines (track separators)
        let num_tracks = (rect.height() / self.track_height) as i32;
        for track in 0..=num_tracks {
            let y = rect.top() + track as f32 * self.track_height;
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

        self.handle_clip_interaction(response, track_idx, clip_idx, clip_rect, app);
    }

    fn draw_loop_region(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        app: &super::app::YadawApp,
    ) {
        let loop_enabled = app.audio_state.loop_enabled.load(Ordering::Relaxed);
        if !loop_enabled {
            return;
        }

        let loop_start = app.audio_state.loop_start.load();
        let loop_end = app.audio_state.loop_end.load();

        let start_x = rect.left() + (loop_start as f32 * self.zoom_x - self.scroll_x);
        let end_x = rect.left() + (loop_end as f32 * self.zoom_x - self.scroll_x);

        // Draw loop region overlay
        let loop_rect = egui::Rect::from_min_max(
            egui::pos2(start_x, rect.top()),
            egui::pos2(end_x, rect.bottom()),
        );

        painter.rect_filled(
            loop_rect,
            0.0,
            egui::Color32::from_rgba_premultiplied(100, 150, 255, 20),
        );

        // Draw loop markers
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
        for note in &clip.notes {
            let note_x = clip_rect.left()
                + (note.start as f32 * self.zoom_x / clip.length_beats as f32 * clip_width);
            let note_y = clip_rect.bottom() - ((note.pitch as f32 / 127.0) * clip_rect.height());
            let note_width = (note.duration as f32 * self.zoom_x / clip.length_beats as f32
                * clip_width)
                .max(2.0);

            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(note_x, note_y - 2.0),
                    egui::vec2(note_width, 2.0),
                ),
                0.0,
                egui::Color32::from_rgba_premultiplied(255, 255, 255, 100),
            );
        }

        // Draw clip name
        painter.text(
            clip_rect.min + egui::vec2(5.0, 5.0),
            egui::Align2::LEFT_TOP,
            &clip.name,
            egui::FontId::default(),
            egui::Color32::WHITE,
        );

        // Handle interaction
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

        // TODO: drag/resize similar to audio clips
    }

    fn handle_clip_interaction(
        &mut self,
        response: egui::Response,
        track_idx: usize,
        clip_idx: usize,
        clip_rect: egui::Rect,
        app: &mut super::app::YadawApp,
    ) {
        // Handle selection
        if response.clicked() {
            if !app.selected_clips.contains(&(track_idx, clip_idx)) {
                if !response.ctx.input(|i| i.modifiers.ctrl) {
                    app.selected_clips.clear();
                }
                app.selected_clips.push((track_idx, clip_idx));
            }
        }

        // Context menu
        if response.secondary_clicked() {
            self.show_clip_menu = true;
            self.clip_menu_pos = response.interact_pointer_pos().unwrap_or_default();
            if !app.selected_clips.contains(&(track_idx, clip_idx)) {
                app.selected_clips.clear();
                app.selected_clips.push((track_idx, clip_idx));
            }
        }

        // Drag/resize handling
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

        // Start drag/resize
        if response.drag_started() && self.timeline_interaction.is_none() {
            app.push_undo();

            // Determine interaction type
            // ... (similar to original implementation)
        }
    }

    fn draw_playhead(&self, painter: &egui::Painter, rect: egui::Rect, app: &super::app::YadawApp) {
        let position = app.audio_state.get_position();
        let sample_rate = app.audio_state.sample_rate.load();
        let bpm = app.audio_state.bpm.load();

        let current_beat = (position / sample_rate as f64) * (bpm as f64 / 60.0);
        let playhead_x = rect.left() + (current_beat as f32 * self.zoom_x - self.scroll_x);

        if playhead_x >= rect.left() && playhead_x <= rect.right() {
            painter.line_segment(
                [
                    egui::pos2(playhead_x, rect.top()),
                    egui::pos2(playhead_x, rect.bottom()),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 100, 100)),
            );
        }
    }

    fn handle_timeline_interaction(
        &mut self,
        response: &egui::Response,
        ui: &mut egui::Ui,
        app: &mut super::app::YadawApp,
    ) {
        // Handle empty space clicks for creating new clips
        if response.clicked() && self.timeline_interaction.is_none() {
            if let Some(pos) = response.interact_pointer_pos() {
                let grid_pos = pos - response.rect.min;
                let beat = (grid_pos.x + self.scroll_x) / self.zoom_x;
                let track_idx = (grid_pos.y / self.track_height) as usize;

                let state = app.state.lock().unwrap();
                if let Some(track) = state.tracks.get(track_idx) {
                    if track.is_midi && ui.input(|i| i.modifiers.ctrl) {
                        // Ctrl+click on MIDI track creates new MIDI clip
                        let snapped_beat =
                            ((beat / self.grid_snap).round() * self.grid_snap) as f64;
                        let _ = app.command_tx.send(AudioCommand::CreateMidiClip(
                            track_idx,
                            snapped_beat,
                            4.0, // Default 1 bar length
                        ));
                    }
                }
            }
        }

        // Handle drag operations
        if response.dragged() {
            if let Some(current_pos) = response.hover_pos() {
                match &mut self.timeline_interaction {
                    Some(TimelineInteraction::DragClip {
                        clips_and_starts,
                        start_drag_beat,
                    }) => {
                        let grid_pos = current_pos - response.rect.min;
                        let current_beat = ((grid_pos.x + self.scroll_x) / self.zoom_x) as f64;
                        let beat_delta = current_beat - *start_drag_beat;
                        let snapped_delta =
                            ((beat_delta / self.grid_snap as f64).round() * self.grid_snap as f64);

                        // Update clip positions
                        for (track_id, clip_id, original_start) in clips_and_starts {
                            let new_start = (*original_start + snapped_delta).max(0.0);

                            // Check if it's a MIDI or audio clip
                            let state = app.state.lock().unwrap();
                            if let Some(track) = state.tracks.get(*track_id) {
                                if track.is_midi {
                                    let _ = app.command_tx.send(AudioCommand::MoveMidiClip(
                                        *track_id, *clip_id, new_start,
                                    ));
                                } else {
                                    let _ = app.command_tx.send(AudioCommand::MoveAudioClip(
                                        *track_id, *clip_id, new_start,
                                    ));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // End interaction
        if response.drag_stopped() {
            self.timeline_interaction = None;
        }
    }

    fn draw_automation_lanes(
        &mut self,
        ui: &mut egui::Ui,
        track_rect: egui::Rect,
        track: &mut Track,
        track_idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        for (lane_idx, lane) in track.automation_lanes.iter_mut().enumerate() {
            if !lane.visible {
                continue;
            }

            let lane_rect = egui::Rect::from_min_size(
                track_rect.min
                    + egui::vec2(0.0, self.track_height - 30.0 * (lane_idx as f32 + 1.0)),
                egui::vec2(track_rect.width(), 30.0),
            );

            // Ensure we have enough widgets
            while self.automation_widgets.len() <= lane_idx {
                self.automation_widgets
                    .push(AutomationLaneWidget::default());
            }

            let actions = self.automation_widgets[lane_idx].ui(
                ui,
                lane,
                lane_rect,
                self.zoom_x,
                self.scroll_x,
            );

            // Process automation actions
            for action in actions {
                match action {
                    AutomationAction::AddPoint { beat, value } => {
                        app.push_undo();
                        let _ = app.command_tx.send(AudioCommand::AddAutomationPoint(
                            track_idx,
                            lane.parameter.clone(),
                            beat,
                            value,
                        ));
                    }
                    AutomationAction::RemovePoint(beat) => {
                        app.push_undo();
                        let _ = app.command_tx.send(AudioCommand::RemoveAutomationPoint(
                            track_idx, lane_idx, beat,
                        ));
                    }
                    AutomationAction::MovePoint {
                        old_beat,
                        new_beat,
                        new_value,
                    } => {
                        app.push_undo();
                        let _ = app.command_tx.send(AudioCommand::RemoveAutomationPoint(
                            track_idx, lane_idx, old_beat,
                        ));
                        let _ = app.command_tx.send(AudioCommand::AddAutomationPoint(
                            track_idx,
                            lane.parameter.clone(),
                            new_beat,
                            new_value,
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

        let current_beat = (position / sample_rate as f64) * (bpm as f64 / 60.0);
        let playhead_x = current_beat as f32 * self.zoom_x;

        // Auto-scroll to keep playhead visible
        let view_width = 800.0; // Approximate, should get from UI
        if playhead_x > self.scroll_x + view_width - 100.0 {
            self.scroll_x = playhead_x - view_width + 100.0;
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
