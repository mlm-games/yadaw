use crate::piano_roll::{PianoRoll, PianoRollAction};
use crate::plugin::PluginInfo;
use crate::state::{AppState, AudioCommand, UIUpdate};
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
        }
    }
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
                // Show timeline for audio tracks (existing code)
                let rect = ui.available_rect_before_wrap();
                let (is_playing, current_position, sample_rate) = {
                    let state = self.state.lock().unwrap();
                    (state.playing, state.current_position, state.sample_rate)
                };

                let painter = ui.painter();

                // Draw grid
                let grid_size = 50.0;
                let grid_color = egui::Color32::from_gray(40);

                for i in 0..((rect.width() / grid_size) as i32 + 1) {
                    let x = rect.left() + i as f32 * grid_size;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(1.0, grid_color),
                    );
                }

                for i in 0..((rect.height() / grid_size) as i32 + 1) {
                    let y = rect.top() + i as f32 * grid_size;
                    painter.line_segment(
                        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                        egui::Stroke::new(1.0, grid_color),
                    );
                }

                // Draw playhead
                if is_playing {
                    let pixels_per_second = 100.0;
                    let time_seconds = current_position / sample_rate as f64;
                    let playhead_x = rect.left() + (time_seconds * pixels_per_second) as f32;

                    painter.line_segment(
                        [
                            egui::pos2(playhead_x, rect.top()),
                            egui::pos2(playhead_x, rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 100, 100)),
                    );
                }

                ui.heading("Timeline");
                ui.label("Audio arrangement view - Coming soon!");
            }

            // Request repaint for smooth playback
            if self.state.lock().unwrap().playing {
                ctx.request_repaint();
            }
        });
    }
}
