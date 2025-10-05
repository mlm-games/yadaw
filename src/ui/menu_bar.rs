use std::sync::atomic::Ordering;

use super::*;
use crate::{constants::DEFAULT_MIN_PROJECT_BEATS, messages::AudioCommand};

pub struct MenuBar {
    show_about: bool,
    show_preferences: bool,
    show_keyboard_shortcuts: bool,
}

impl MenuBar {
    pub fn new() -> Self {
        Self {
            show_about: false,
            show_preferences: false,
            show_keyboard_shortcuts: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.file_menu(ui, app);
                self.edit_menu(ui, app);
                self.view_menu(ui, app);
                self.track_menu(ui, app);
                self.transport_menu(ui, app);
                self.tools_menu(ui, app);
                self.window_menu(ui, app);
                self.help_menu(ui, app);
            });
        });

        // Show dialogs
        self.show_dialogs(ctx, app);
    }

    fn file_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("File", |ui| {
            if ui.button("New Project").clicked() {
                app.new_project();
                ui.close();
            }

            if ui.button("Open Project...").clicked() {
                app.dialogs.show_open_dialog();
                ui.close();
            }

            // Recent projects submenu
            ui.menu_button("Open Recent", |ui| {
                let recent = app.project_manager.get_recent_projects().to_vec();
                if recent.is_empty() {
                    ui.label("No recent projects");
                } else {
                    for path in recent {
                        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                            if ui.button(name).clicked() {
                                app.load_project_from_path(&path);
                                ui.close();
                            }
                        }
                    }

                    ui.separator();
                    if ui.button("Clear Recent").clicked() {
                        app.project_manager.clear_recent_projects();
                        ui.close();
                    }
                }
            });

            ui.separator();

            if ui.button("Save").clicked() {
                app.save_project();
                ui.close();
            }

            if ui.button("Save As...").clicked() {
                app.dialogs.show_save_dialog();
                ui.close();
            }

            ui.separator();

            if ui.button("Import Audio...").clicked() {
                app.import_audio_dialog();
                ui.close();
            }

            if ui.button("Export Audio...").clicked() {
                app.export_audio_dialog();
                ui.close();
            }

            ui.separator();

            if ui.button("Project Settings...").clicked() {
                app.dialogs.show_project_settings();
                ui.close();
            }

            ui.separator();

            if ui.button("Exit").clicked() {
                // ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }

    fn edit_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Edit", |ui| {
            let has_undo = !app.undo_stack.is_empty();
            let has_redo = !app.redo_stack.is_empty();

            ui.add_enabled_ui(has_undo, |ui| {
                if ui.button("Undo").clicked() {
                    app.undo();
                    ui.close();
                }
            });

            ui.add_enabled_ui(has_redo, |ui| {
                if ui.button("Redo").clicked() {
                    app.redo();
                    ui.close();
                }
            });

            ui.separator();

            let notes_active =
                matches!(app.active_edit_target, super::app::ActiveEditTarget::Notes)
                    && app.is_selected_track_midi();

            // CUT
            if ui.button("Cut").clicked() {
                if notes_active {
                    let clipboard = app.piano_roll_view.cut_selected_notes(
                        &app.state,
                        app.selected_track,
                        &app.command_tx,
                    );
                    if let Some(notes) = clipboard {
                        app.note_clipboard = Some(notes);
                        app.push_undo();
                    }
                } else {
                    app.cut_selected();
                }
                ui.close();
            }

            // COPY
            if ui.button("Copy").clicked() {
                if notes_active {
                    let clipboard = app
                        .piano_roll_view
                        .copy_selected_notes(&app.state, app.selected_track);
                    if let Some(notes) = clipboard {
                        app.note_clipboard = Some(notes);
                    }
                } else {
                    app.copy_selected();
                }
                ui.close();
            }

            // PASTE
            if ui.button("Paste").clicked() {
                if notes_active {
                    if let Some(ref clipboard) = app.note_clipboard.clone() {
                        app.piano_roll_view.paste_notes(
                            &app.state,
                            app.selected_track,
                            &app.audio_state,
                            &app.command_tx,
                            clipboard,
                        );
                        app.push_undo();
                    }
                } else {
                    app.paste_at_playhead();
                }
                ui.close();
            }

            // DELETE
            if ui.button("Delete").clicked() {
                if notes_active {
                    if app.piano_roll_view.delete_selected_notes(
                        &app.state,
                        app.selected_track,
                        &app.command_tx,
                    ) {
                        app.push_undo();
                    }
                } else {
                    app.delete_selected();
                }
                ui.close();
            }

            ui.separator();

            if ui.button("Select All").clicked() {
                if notes_active {
                    app.piano_roll_view
                        .select_all_notes(&app.state, app.selected_track);
                } else {
                    app.select_all();
                }
                ui.close();
            }

            if ui.button("Deselect All").clicked() {
                if notes_active {
                    app.piano_roll_view.piano_roll.selected_note_ids.clear();
                    app.piano_roll_view.piano_roll.temp_selected_indices.clear();
                } else {
                    app.deselect_all();
                }
                ui.close();
            }

            ui.separator();

            if ui.button("Preferences...").clicked() {
                self.show_preferences = true;
                ui.close();
            }
        });
    }

    fn view_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("View", |ui| {
            if ui.checkbox(&mut app.mixer_ui.visible, "Mixer").clicked() {
                ui.close();
            }

            if ui
                .checkbox(&mut app.timeline_ui.show_automation, "Automation Lanes")
                .clicked()
            {
                ui.close();
            }

            ui.separator();

            if ui.button("Zoom In").clicked() {
                app.timeline_ui.zoom_x *= 1.25;
                ui.close();
            }

            if ui.button("Zoom Out").clicked() {
                app.timeline_ui.zoom_x *= 0.8;
                ui.close();
            }

            if ui.button("Zoom to Fit").clicked() {
                app.zoom_to_fit();
                ui.close();
            }

            ui.separator();

            ui.menu_button("Theme", |ui| {
                if ui
                    .radio_value(&mut app.config.ui.theme, crate::config::Theme::Dark, "Dark")
                    .clicked()
                {
                    app.theme_manager.set_theme(super::theme::Theme::Dark);
                    ui.close();
                }

                if ui
                    .radio_value(
                        &mut app.config.ui.theme,
                        crate::config::Theme::Light,
                        "Light",
                    )
                    .clicked()
                {
                    app.theme_manager.set_theme(super::theme::Theme::Light);
                    ui.close();
                }

                ui.separator();

                let binding = app.theme_manager.clone();
                let custom_themes = binding.get_custom_themes();

                for custom_theme in custom_themes {
                    if ui.button(&custom_theme.name).clicked() {
                        app.theme_manager
                            .set_theme(Theme::Custom(custom_theme.clone()));
                        ui.close();
                    }
                }

                if ui.button("Edit Themes...").clicked() {
                    app.dialogs.show_theme_editor();
                    ui.close();
                }
            });

            ui.separator();

            if ui
                .checkbox(&mut app.show_performance, "Performance Monitor")
                .clicked()
            {
                ui.close();
            }
        });
    }

    fn track_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Track", |ui| {
            if ui.button("Add Audio Track").clicked() {
                app.add_audio_track();
                ui.close();
            }

            if ui.button("Add MIDI Track").clicked() {
                app.add_midi_track();
                ui.close();
            }

            if ui.button("Add Bus").clicked() {
                app.add_bus_track();
                ui.close();
            }

            ui.separator();

            if ui.button("Duplicate Track").clicked() {
                app.duplicate_selected_track();
                ui.close();
            }

            if ui.button("Delete Track").clicked() {
                app.delete_selected_track();
                ui.close();
            }

            ui.separator();

            if ui.button("Group Tracks...").clicked() {
                app.dialogs.show_track_grouping();
                ui.close();
            }
        });
    }

    fn transport_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Transport", |ui| {
            if ui.button("Play/Stop").clicked() {
                app.transport_ui.toggle_playback(&app.command_tx);
                ui.close();
            }

            if ui.button("Record").clicked() {
                // Toggle recording
                ui.close();
            }

            ui.separator();

            if ui.button("Go to Start").clicked() {
                if let Some(transport) = &mut app.transport_ui.transport {
                    transport.set_position(0.0);
                }
                ui.close();
            }

            if ui.button("Go to End").clicked() {
                // Go to end of project
                ui.close();
            }

            ui.separator();

            let mut metronome_enabled = app
                .transport_ui
                .transport
                .as_ref()
                .map(|t| t.metronome_enabled)
                .unwrap_or(false);

            if ui.checkbox(&mut metronome_enabled, "Metronome").clicked() {
                if let Some(transport) = &mut app.transport_ui.transport {
                    transport.metronome_enabled = metronome_enabled;
                }
                ui.close();
            }

            if ui.button("Tap Tempo").clicked() {
                app.tap_tempo();
                ui.close();
            }

            ui.separator();

            let mut loop_enabled = app.audio_state.loop_enabled.load(Ordering::Relaxed);
            if ui.checkbox(&mut loop_enabled, "Loop Enabled").clicked() {
                app.audio_state
                    .loop_enabled
                    .store(loop_enabled, Ordering::Relaxed);
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetLoopEnabled(loop_enabled));

                if loop_enabled {
                    let (start, end) = {
                        // compute project end in beats
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
                        (0.0, max_beat)
                    };
                    let cur_s = app.audio_state.loop_start.load();
                    let cur_e = app.audio_state.loop_end.load();
                    if !(cur_e > cur_s) {
                        app.audio_state.loop_start.store(start);
                        app.audio_state.loop_end.store(end);
                        let _ = app.command_tx.send(AudioCommand::SetLoopRegion(start, end));
                    }
                }
                ui.close();
            }

            if ui.button("Set Loop to Selection").clicked() {
                app.set_loop_to_selection();
                ui.close();
            }

            if ui.button("Clear Loop").clicked() {
                app.audio_state.loop_enabled.store(false, Ordering::Relaxed);
                let _ = app.command_tx.send(AudioCommand::SetLoopEnabled(false));
                ui.close();
            }

            if ui.button("Go to End").clicked() {
                let end_beats = {
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
                };
                // convert beats->samples
                let sr = app.audio_state.sample_rate.load() as f64;
                let bpm = app.audio_state.bpm.load() as f64;
                if bpm > 0.0 && sr > 0.0 {
                    let samples = end_beats * (60.0 / bpm) * sr;
                    let _ = app.command_tx.send(AudioCommand::SetPosition(samples));
                }
                ui.close();
            }
        });
    }

    fn tools_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Tools", |ui| {
            if ui.button("Plugin Manager...").clicked() {
                app.dialogs.show_plugin_manager();
                ui.close();
            }

            if ui.button("Audio Setup...").clicked() {
                app.dialogs.show_audio_setup();
                ui.close();
            }

            ui.separator();

            if app.is_selected_track_midi() {
                ui.menu_button("MIDI Tools", |ui| {
                    if ui.button("Quantize...").clicked() {
                        app.dialogs.show_quantize_dialog();
                        ui.close();
                    }

                    if ui.button("Transpose...").clicked() {
                        app.dialogs.show_transpose_dialog();
                        ui.close();
                    }

                    if ui.button("Humanize...").clicked() {
                        app.dialogs.show_humanize_dialog();
                        ui.close();
                    }
                });
            }

            ui.menu_button("Audio Tools", |ui| {
                if ui.button("Normalize").clicked() {
                    app.normalize_selected();
                    ui.close();
                }

                if ui.button("Reverse").clicked() {
                    app.reverse_selected();
                    ui.close();
                }

                if ui.button("Time Stretch...").clicked() {
                    app.dialogs.show_time_stretch_dialog();
                    ui.close();
                }
            });
        });
    }

    fn window_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Window", |ui| {
            if ui.button("Mixer").clicked() {
                app.mixer_ui.toggle_visibility();
                ui.close();
            }

            if ui.button("Piano Roll").clicked() {
                app.switch_to_piano_roll();
                ui.close();
            }

            if ui.button("Timeline").clicked() {
                app.switch_to_timeline();
                ui.close();
            }

            ui.separator();

            if ui.button("Reset Layout").clicked() {
                app.reset_layout();
                ui.close();
            }

            if ui.button("Save Layout...").clicked() {
                app.dialogs.show_save_layout_dialog();
                ui.close();
            }

            if ui.button("Load Layout...").clicked() {
                app.dialogs.show_load_layout_dialog();
                ui.close();
            }
        });
    }

    fn help_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Help", |ui| {
            if ui.button("User Manual").clicked() {
                // Open user manual
                ui.close();
            }

            if ui.button("Keyboard Shortcuts").clicked() {
                self.show_keyboard_shortcuts = true;
                ui.close();
            }

            ui.separator();

            if ui.button("About YADAW").clicked() {
                self.show_about = true;
                ui.close();
            }
        });
    }

    fn show_dialogs(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        // About dialog
        if self.show_about {
            egui::Window::new("About YADAW")
                .open(&mut self.show_about)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("YADAW");
                        ui.label("Yet Another DAW");
                        ui.label("Version 0.1.0");
                        ui.separator();
                        ui.label("A practice DAW implementation in Rust");
                        ui.hyperlink("https://github.com/mlm-games/yadaw");
                    });
                });
        }

        if self.show_preferences {
            let mut show_preferences = true;
            let config = app.config.clone();

            egui::Window::new("Preferences")
                .open(&mut show_preferences)
                .resizable(true)
                .default_size(egui::vec2(600.0, 400.0))
                .show(ctx, |ui| {
                    draw_preferences_static(ui, &config);
                });
            self.show_preferences = show_preferences;
        }

        // Keyboard shortcuts dialog
        if self.show_keyboard_shortcuts {
            let mut show_keyboard_shortcuts = true;
            egui::Window::new("Keyboard Shortcuts")
                .open(&mut show_keyboard_shortcuts)
                .resizable(true)
                .default_size(egui::vec2(400.0, 500.0))
                .show(ctx, |ui| {
                    draw_keyboard_shortcuts_static(ui);
                });
            self.show_keyboard_shortcuts = show_keyboard_shortcuts;
        }
    }
}

fn draw_preferences_static(ui: &mut egui::Ui, config: &crate::config::Config) {
    ui.horizontal(|ui| {
        // Categories list
        ui.vertical(|ui| {
            ui.set_min_width(150.0);
            ui.selectable_label(true, "Audio"); // TODO for later
            ui.selectable_label(false, "MIDI");
            ui.selectable_label(false, "Appearance");
            ui.selectable_label(false, "Behavior");
            ui.selectable_label(false, "Plugins");
            ui.selectable_label(false, "File Paths");
        });

        ui.separator();

        // Settings panel
        ui.vertical(|ui| {
            ui.heading("Audio Settings");

            ui.horizontal(|ui| {
                ui.label("Buffer Size:");
                ui.label(format!("{}", config.audio.buffer_size));
            });

            ui.horizontal(|ui| {
                ui.label("Sample Rate:");
                ui.label(format!("{} Hz", config.audio.sample_rate));
            });

            ui.separator();

            if ui.button("Apply").clicked() {
                // Apply settings
            }
        });
    });
}

fn draw_keyboard_shortcuts_static(ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("Transport");
        ui.label("Space - Play/Stop");
        ui.label("R - Record");
        ui.label("Home - Go to Start");

        ui.separator();

        ui.heading("Editing");
        ui.label("Ctrl+Z - Undo");
        ui.label("Ctrl+Shift+Z - Redo");
        ui.label("Ctrl+X - Cut");
        ui.label("Ctrl+C - Copy");
        ui.label("Ctrl+V - Paste");
        ui.label("Delete - Delete Selected");

        ui.separator();

        ui.heading("File");
        ui.label("Ctrl+N - New Project");
        ui.label("Ctrl+O - Open Project");
        ui.label("Ctrl+S - Save Project");
        ui.label("Ctrl+Shift+S - Save As");

        ui.separator();

        ui.heading("View");
        ui.label("Ctrl++ - Zoom In");
        ui.label("Ctrl+- - Zoom Out");
        ui.label("Ctrl+M - Toggle Mixer");
    });
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}
