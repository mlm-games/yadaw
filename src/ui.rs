use crate::piano_roll::{PianoRoll, PianoRollAction};
use crate::plugin::PluginInfo;
use crate::state::{AppState, AudioCommand, Project, UIUpdate};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::sync::{Arc, Mutex};

pub struct YadawApp {
    state: Arc<Mutex<AppState>>,
    audio_tx: Sender<AudioCommand>,
    ui_rx: Receiver<UIUpdate>,
    available_plugins: Vec<PluginInfo>,
    show_plugin_browser: bool,
    selected_track_for_plugin: Option<usize>,
    piano_roll: PianoRoll,
    selected_track: usize,
    selected_pattern: usize,
    project_path: Option<String>,
    show_save_dialog: bool,
    show_load_dialog: bool,
    timeline_zoom: f32,
    show_message: Option<String>,
}

impl YadawApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        audio_tx: Sender<AudioCommand>,
        ui_rx: Receiver<UIUpdate>,
        available_plugins: Vec<PluginInfo>,
    ) -> Self {
        Self {
            state,
            audio_tx,
            ui_rx,
            available_plugins,
            show_plugin_browser: false,
            selected_track_for_plugin: None,
            piano_roll: PianoRoll::default(),
            selected_track: 1, // Start with MIDI track selected
            selected_pattern: 0,
            project_path: None,
            show_save_dialog: false,
            show_load_dialog: false,
            timeline_zoom: 100.0,
            show_message: None,
        }
    }
}

enum TimelineInteraction {
    DragClip {
        track_id: usize,
        clip_id: usize,
        delta: egui::Vec2,
    },
    ToggleClip {
        track_id: usize,
        clip_id: usize,
    },
}

impl eframe::App for YadawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process UI updates from audio thread
        while let Ok(update) = self.ui_rx.try_recv() {
            match update {
                UIUpdate::Position(pos) => {
                    let mut state = self.state.lock().unwrap();
                    state.current_position = pos;
                }
                UIUpdate::RecordingFinished(track_id, clip) => {
                    let mut state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get_mut(track_id) {
                        track.audio_clips.push(clip);
                    }
                }
                UIUpdate::RecordingLevel(level) => {
                    // TODO: Show recording level meter
                }
                _ => {}
            }
        }

        // Plugin browser window
        let mut show_browser = self.show_plugin_browser;
        egui::Window::new("Plugin Browser")
            .open(&mut show_browser)
            .show(ctx, |ui| {
                ui.heading("Available Plugins");

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for plugin in &self.available_plugins {
                        if ui.button(&plugin.name).clicked() {
                            if let Some(track_id) = self.selected_track_for_plugin {
                                let _ = self
                                    .audio_tx
                                    .send(AudioCommand::AddPlugin(track_id, plugin.uri.clone()));
                                self.show_plugin_browser = false;
                            }
                        }
                    }
                });
            });
        self.show_plugin_browser = show_browser;

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Project").clicked() {
                        // Reset to default state
                        let mut state = self.state.lock().unwrap();
                        *state = AppState::new();
                        self.project_path = None;
                        ui.close();
                    }

                    if ui.button("Open Project...").clicked() {
                        self.show_load_dialog = true;
                        ui.close();
                    }

                    if ui.button("Save Project").clicked() {
                        if let Some(path) = &self.project_path {
                            self.save_project(path);
                        } else {
                            self.show_save_dialog = true;
                        }
                        ui.close();
                    }

                    if ui.button("Save Project As...").clicked() {
                        self.show_save_dialog = true;
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        // File dialogs - Fixed borrow checker issues
        if self.show_save_dialog {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("YADAW Project", &["yadaw"])
                .add_filter("All Files", &["*"])
                .set_file_name("untitled.yadaw")
                .save_file()
            {
                self.save_project(path.to_str().unwrap());
                self.project_path = Some(path.to_str().unwrap().to_string());
            }
            self.show_save_dialog = false;
        }

        if self.show_load_dialog {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("YADAW Project", &["yadaw"])
                .add_filter("All Files", &["*"])
                .pick_file()
            {
                self.load_project(path.to_str().unwrap());
                self.project_path = Some(path.to_str().unwrap().to_string());
            }
            self.show_load_dialog = false;
        }

        if let Some(message) = &self.show_message.clone() {
            egui::Window::new("Message")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("OK").clicked() {
                        self.show_message = None;
                    }
                });
        }

        // Top panel - Transport controls
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut state = self.state.lock().unwrap();
                let is_playing = state.playing;

                if ui.button(if is_playing { "⏸" } else { "▶" }).clicked() {
                    if is_playing {
                        let _ = self.audio_tx.send(AudioCommand::Stop);
                    } else {
                        let _ = self.audio_tx.send(AudioCommand::Play);
                    }
                }

                if ui.button("⏹").clicked() {
                    let _ = self.audio_tx.send(AudioCommand::Stop);
                }

                if ui
                    .button(if state.recording {
                        "⏺ Recording"
                    } else {
                        "⏺"
                    })
                    .on_hover_text("Record")
                    .clicked()
                {
                    if state.recording {
                        let _ = self.audio_tx.send(AudioCommand::StopRecording);
                    } else {
                        // Find first armed track
                        let armed_track = state.tracks.iter().position(|t| t.armed && !t.is_midi);
                        if let Some(track_id) = armed_track {
                            let _ = self.audio_tx.send(AudioCommand::StartRecording(track_id));
                        } else {
                            // Show message that no track is armed
                            self.show_message =
                                Some("Please arm an audio track for recording".to_string());
                        }
                    }
                }

                ui.separator();

                // Time display
                let time_seconds = state.current_position / state.sample_rate as f64;
                let minutes = (time_seconds / 60.0) as i32;
                let seconds = time_seconds % 60.0;
                ui.label(format!("{:02}:{:05.2}", minutes, seconds));

                ui.separator();
                ui.label(format!("BPM: {:.1}", state.bpm));

                ui.separator();
                ui.label("Master Vol:");
                let mut master_vol = state.master_volume;
                if ui
                    .add(egui::Slider::new(&mut master_vol, 0.0..=1.0).show_value(false))
                    .changed()
                {
                    state.master_volume = master_vol;
                }
            });
        });

        // Left panel - Track controls
        egui::SidePanel::left("tracks")
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.heading("Tracks");

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut commands_to_send = Vec::new();

                    let mut add_track_clicked = false;

                    {
                        let mut state = self.state.lock().unwrap();
                        let num_tracks = state.tracks.len();

                        for i in 0..num_tracks {
                            let is_selected = i == self.selected_track;

                            ui.group(|ui| {
                                if is_selected {
                                    ui.visuals_mut().override_text_color =
                                        Some(egui::Color32::from_rgb(150, 200, 255));
                                }

                                let track = &mut state.tracks[i];

                                ui.horizontal(|ui| {
                                    ui.label(&track.name);

                                    if ui.button(&track.name).clicked() {
                                        self.selected_track = i;
                                    }

                                    ui.label(if track.is_midi { "🎹" } else { "🎵" });

                                    if ui
                                        .small_button(if track.muted { "🔇" } else { "🔊" })
                                        .clicked()
                                    {
                                        let muted = !track.muted;
                                        commands_to_send.push(AudioCommand::MuteTrack(i, muted));
                                    }

                                    if ui
                                        .small_button(if track.solo { "S" } else { "s" })
                                        .on_hover_text("Solo")
                                        .clicked()
                                    {
                                        track.solo = !track.solo;
                                    }

                                    if ui
                                        .small_button(if track.armed { "🔴" } else { "⭕" })
                                        .on_hover_text("Record Arm")
                                        .clicked()
                                    {
                                        track.armed = !track.armed;
                                    }
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Vol:");
                                    let mut volume = track.volume;
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut volume, 0.0..=1.0)
                                                .show_value(false),
                                        )
                                        .changed()
                                    {
                                        commands_to_send
                                            .push(AudioCommand::SetTrackVolume(i, volume));
                                    }
                                    ui.label(format!("{:.0}%", volume * 100.0));
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Pan:");
                                    let mut pan = track.pan;
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut pan, -1.0..=1.0)
                                                .show_value(false),
                                        )
                                        .changed()
                                    {
                                        commands_to_send.push(AudioCommand::SetTrackPan(i, pan));
                                    }
                                    let pan_text = if pan.abs() < 0.01 {
                                        "C".to_string()
                                    } else if pan < 0.0 {
                                        format!("L{:.0}", -pan * 100.0)
                                    } else {
                                        format!("R{:.0}", pan * 100.0)
                                    };
                                    ui.label(pan_text);
                                });

                                // Plugin chain
                                ui.separator();
                                ui.horizontal(|ui| {
                                    ui.label("Plugins:");
                                    if ui.small_button("+").clicked() {
                                        self.show_plugin_browser = true;
                                        self.selected_track_for_plugin = Some(i);
                                    }
                                });

                                let mut plugin_to_remove = None;
                                for (j, plugin) in track.plugin_chain.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(&plugin.name);
                                        if ui.small_button("×").clicked() {
                                            plugin_to_remove = Some(j);
                                        }
                                    });
                                }

                                if let Some(j) = plugin_to_remove {
                                    commands_to_send.push(AudioCommand::RemovePlugin(i, j));
                                }
                            });

                            ui.add_space(5.0);
                        }

                        if ui.button("➕ Add Track").clicked() {
                            add_track_clicked = true;
                        }

                        if add_track_clicked {
                            let new_track_id = state.tracks.len();

                            state.tracks.push(crate::state::Track {
                                id: new_track_id,
                                name: format!("Track {}", new_track_id + 1),
                                volume: 0.7,
                                pan: 0.0,
                                muted: false,
                                solo: false,
                                armed: false,
                                plugin_chain: vec![],
                                patterns: vec![],
                                is_midi: false,
                                audio_clips: vec![],
                            });
                        }
                    }

                    // Send commands after releasing the lock
                    for cmd in commands_to_send {
                        let _ = self.audio_tx.send(cmd);
                    }
                });
            });

        // Central panel - Timeline/Arrangement
        egui::CentralPanel::default().show(ctx, |ui| {
            // Check if selected track is MIDI
            let is_midi_track = {
                let state = self.state.lock().unwrap();
                state
                    .tracks
                    .get(self.selected_track)
                    .map(|t| t.is_midi)
                    .unwrap_or(false)
            };

            if is_midi_track {
                // Show piano roll for MIDI tracks
                ui.heading("Piano Roll");

                // Pattern selector
                ui.horizontal(|ui| {
                    ui.label("Pattern:");
                    let state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get(self.selected_track) {
                        for (i, pattern) in track.patterns.iter().enumerate() {
                            if ui
                                .selectable_label(i == self.selected_pattern, &pattern.name)
                                .clicked()
                            {
                                self.selected_pattern = i;
                            }
                        }
                    }
                });

                ui.separator();

                // Piano roll editor
                let mut pattern_actions = Vec::new();

                {
                    let state = self.state.lock().unwrap();
                    if state.playing {
                        let current_beat = state.position_to_beats(state.current_position);
                        let pattern_beat = current_beat
                            % state
                                .tracks
                                .get(self.selected_track)
                                .and_then(|t| t.patterns.first())
                                .map(|p| p.length)
                                .unwrap_or(4.0);
                        ctx.memory_mut(|mem| {
                            mem.data
                                .insert_temp(egui::Id::new("current_beat"), pattern_beat)
                        });
                    }
                }

                {
                    let mut state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get_mut(self.selected_track) {
                        if let Some(pattern) = track.patterns.get_mut(self.selected_pattern) {
                            pattern_actions = self.piano_roll.ui(ui, pattern);
                        }
                    }
                }

                // Process piano roll actions
                for action in pattern_actions {
                    match action {
                        PianoRollAction::AddNote(note) => {
                            let _ = self.audio_tx.send(AudioCommand::AddNote(
                                self.selected_track,
                                self.selected_pattern,
                                note,
                            ));
                        }
                        PianoRollAction::RemoveNote(index) => {
                            let _ = self.audio_tx.send(AudioCommand::RemoveNote(
                                self.selected_track,
                                self.selected_pattern,
                                index,
                            ));
                        }
                        PianoRollAction::UpdateNote(index, note) => {
                            let _ = self.audio_tx.send(AudioCommand::UpdateNote(
                                self.selected_track,
                                self.selected_pattern,
                                index,
                                note,
                            ));
                        }
                    }
                }
            } else {
                // Show timeline for audio tracks
                ui.heading("Timeline");

                let rect = ui.available_rect_before_wrap();
                let (is_playing, current_position, sample_rate, bpm) = {
                    let state = self.state.lock().unwrap();
                    (
                        state.playing,
                        state.current_position,
                        state.sample_rate,
                        state.bpm,
                    )
                };

                // Timeline controls
                ui.horizontal(|ui| {
                    ui.label("Zoom:");
                    if ui.button("-").clicked() {
                        self.timeline_zoom *= 0.8;
                    }
                    if ui.button("+").clicked() {
                        self.timeline_zoom *= 1.25;
                    }
                    ui.label(format!("{:.0} pixels/beat", self.timeline_zoom));
                });

                ui.separator();

                // Timeline scroll area
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let timeline_rect = ui.available_rect_before_wrap();
                        let track_height = 80.0;

                        // Allocate space for all tracks
                        let total_height =
                            self.state.lock().unwrap().tracks.len() as f32 * track_height;
                        ui.allocate_space(egui::vec2(timeline_rect.width(), total_height));

                        let painter = ui.painter();
                        let timeline_rect = ui.min_rect();

                        // Draw grid
                        self.draw_timeline_grid(&painter, timeline_rect, bpm);

                        // Draw tracks and clips
                        let mut clip_interactions = Vec::new();
                        {
                            let state = self.state.lock().unwrap();
                            for (track_idx, track) in state.tracks.iter().enumerate() {
                                let track_rect = egui::Rect::from_min_size(
                                    timeline_rect.min
                                        + egui::vec2(0.0, track_idx as f32 * track_height),
                                    egui::vec2(timeline_rect.width(), track_height),
                                );

                                // Track background
                                painter.rect_filled(
                                    track_rect,
                                    0.0,
                                    if track_idx % 2 == 0 {
                                        egui::Color32::from_gray(25)
                                    } else {
                                        egui::Color32::from_gray(30)
                                    },
                                );

                                // Track name
                                painter.text(
                                    track_rect.min + egui::vec2(5.0, 5.0),
                                    egui::Align2::LEFT_TOP,
                                    &track.name,
                                    egui::FontId::default(),
                                    egui::Color32::WHITE,
                                );

                                // Draw audio clips
                                for (clip_idx, clip) in track.audio_clips.iter().enumerate() {
                                    let clip_x = clip.start_beat as f32 * self.timeline_zoom;
                                    let clip_width = clip.length_beats as f32 * self.timeline_zoom;

                                    let clip_rect = egui::Rect::from_min_size(
                                        track_rect.min + egui::vec2(clip_x, 20.0),
                                        egui::vec2(clip_width, track_height - 25.0),
                                    );

                                    // Draw waveform
                                    crate::waveform::draw_waveform(
                                        &painter,
                                        clip_rect,
                                        clip,
                                        self.timeline_zoom,
                                        0.0,
                                    );

                                    // Check for interactions
                                    let response = ui.interact(
                                        clip_rect,
                                        ui.id().with((track_idx, clip_idx)),
                                        egui::Sense::click_and_drag(),
                                    );

                                    if response.dragged() {
                                        clip_interactions.push(TimelineInteraction::DragClip {
                                            track_id: track_idx,
                                            clip_id: clip_idx,
                                            delta: response.drag_delta(),
                                        });
                                    }

                                    if response.double_clicked() {
                                        clip_interactions.push(TimelineInteraction::ToggleClip {
                                            track_id: track_idx,
                                            clip_id: clip_idx,
                                        });
                                    }
                                }
                            }
                        }

                        // Process interactions
                        for interaction in clip_interactions {
                            match interaction {
                                TimelineInteraction::DragClip {
                                    track_id,
                                    clip_id,
                                    delta,
                                } => {
                                    let mut state = self.state.lock().unwrap();
                                    if let Some(track) = state.tracks.get_mut(track_id) {
                                        if let Some(clip) = track.audio_clips.get_mut(clip_id) {
                                            let beat_delta = delta.x / self.timeline_zoom;
                                            clip.start_beat =
                                                (clip.start_beat + beat_delta as f64).max(0.0);
                                        }
                                    }
                                }
                                TimelineInteraction::ToggleClip { track_id, clip_id } => {
                                    // TODO: Implement clip selection/editing
                                }
                            }
                        }

                        // Draw playhead
                        if is_playing {
                            let current_beat =
                                (current_position / sample_rate as f64) * (bpm / 60.0) as f64;
                            let playhead_x =
                                timeline_rect.left() + (current_beat as f32 * self.timeline_zoom);

                            painter.line_segment(
                                [
                                    egui::pos2(playhead_x, timeline_rect.top()),
                                    egui::pos2(playhead_x, timeline_rect.bottom()),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 100, 100)),
                            );
                        }
                    });
            }

            // Request repaint for smooth playback
            if self.state.lock().unwrap().playing {
                ctx.request_repaint();
            }
        });
    }
}
impl YadawApp {
    fn save_project(&self, path: &str) {
        let state = self.state.lock().unwrap();
        let project = Project::from(&*state);

        match std::fs::write(path, serde_json::to_string_pretty(&project).unwrap()) {
            Ok(_) => println!("Project saved to {}", path),
            Err(e) => eprintln!("Failed to save project: {}", e),
        }
    }

    fn load_project(&self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Project>(&content) {
                Ok(project) => {
                    let mut state = self.state.lock().unwrap();
                    state.load_project(project);
                    println!("Project loaded from {}", path);
                }
                Err(e) => eprintln!("Failed to parse project: {}", e),
            },
            Err(e) => eprintln!("Failed to read project file: {}", e),
        }
    }
}
impl YadawApp {
    fn draw_timeline_grid(&self, painter: &egui::Painter, rect: egui::Rect, bpm: f32) {
        // Vertical lines (beats)
        let beats_visible = (rect.width() / self.timeline_zoom) as i32 + 2;

        for beat in 0..beats_visible {
            let x = rect.left() + beat as f32 * self.timeline_zoom;

            if x >= rect.left() && x <= rect.right() {
                let color = if beat % 4 == 0 {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(40)
                };

                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, color),
                );

                // Beat numbers
                if beat % 4 == 0 {
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
    }
}
