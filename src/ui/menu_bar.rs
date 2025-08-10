use super::*;
use crate::state::AudioCommand;

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
            egui::menu::bar(ui, |ui| {
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
                ui.close_menu();
            }

            if ui.button("Open Project...").clicked() {
                app.dialogs.show_open_dialog();
                ui.close_menu();
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
                                ui.close_menu();
                            }
                        }
                    }

                    ui.separator();
                    if ui.button("Clear Recent").clicked() {
                        app.project_manager.clear_recent_projects();
                        ui.close_menu();
                    }
                }
            });

            ui.separator();

            if ui.button("Save").clicked() {
                app.save_project();
                ui.close_menu();
            }

            if ui.button("Save As...").clicked() {
                app.dialogs.show_save_dialog();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Import Audio...").clicked() {
                app.import_audio_dialog();
                ui.close_menu();
            }

            if ui.button("Export Audio...").clicked() {
                app.export_audio_dialog();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Project Settings...").clicked() {
                app.dialogs.show_project_settings();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Exit").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                    ui.close_menu();
                }
            });

            ui.add_enabled_ui(has_redo, |ui| {
                if ui.button("Redo").clicked() {
                    app.redo();
                    ui.close_menu();
                }
            });

            ui.separator();

            if ui.button("Cut").clicked() {
                app.cut_selected();
                ui.close_menu();
            }

            if ui.button("Copy").clicked() {
                app.copy_selected();
                ui.close_menu();
            }

            if ui.button("Paste").clicked() {
                app.paste_at_playhead();
                ui.close_menu();
            }

            if ui.button("Delete").clicked() {
                app.delete_selected();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Select All").clicked() {
                app.select_all();
                ui.close_menu();
            }

            if ui.button("Deselect All").clicked() {
                app.deselect_all();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Preferences...").clicked() {
                self.show_preferences = true;
                ui.close_menu();
            }
        });
    }

    fn view_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("View", |ui| {
            if ui.checkbox(&mut app.mixer_ui.visible, "Mixer").clicked() {
                ui.close_menu();
            }

            if ui
                .checkbox(&mut app.timeline_ui.show_automation, "Automation Lanes")
                .clicked()
            {
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Zoom In").clicked() {
                app.timeline_ui.zoom_x *= 1.25;
                ui.close_menu();
            }

            if ui.button("Zoom Out").clicked() {
                app.timeline_ui.zoom_x *= 0.8;
                ui.close_menu();
            }

            if ui.button("Zoom to Fit").clicked() {
                app.zoom_to_fit();
                ui.close_menu();
            }

            ui.separator();

            ui.menu_button("Theme", |ui| {
                if ui
                    .radio_value(&mut app.config.ui.theme, crate::config::Theme::Dark, "Dark")
                    .clicked()
                {
                    app.theme_manager.set_theme(super::theme::Theme::Dark);
                    ui.close_menu();
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
                    ui.close_menu();
                }

                ui.separator();

                for custom_theme in app.theme_manager.get_custom_themes() {
                    if ui.button(&custom_theme.name).clicked() {
                        app.theme_manager
                            .set_theme(super::theme::Theme::Custom(custom_theme.clone()));
                        ui.close_menu();
                    }
                }

                if ui.button("Edit Themes...").clicked() {
                    app.dialogs.show_theme_editor();
                    ui.close_menu();
                }
            });

            ui.separator();

            if ui
                .checkbox(&mut app.show_performance, "Performance Monitor")
                .clicked()
            {
                ui.close_menu();
            }
        });
    }

    fn track_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Track", |ui| {
            if ui.button("Add Audio Track").clicked() {
                app.add_audio_track();
                ui.close_menu();
            }

            if ui.button("Add MIDI Track").clicked() {
                app.add_midi_track();
                ui.close_menu();
            }

            if ui.button("Add Bus").clicked() {
                app.add_bus_track();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Duplicate Track").clicked() {
                app.duplicate_selected_track();
                ui.close_menu();
            }

            if ui.button("Delete Track").clicked() {
                app.delete_selected_track();
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Group Tracks...").clicked() {
                app.dialogs.show_track_grouping();
                ui.close_menu();
            }
        });
    }

    fn transport_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Transport", |ui| {
            if ui.button("Play/Stop").clicked() {
                app.transport_ui.toggle_playback(&app.command_tx);
                ui.close_menu();
            }

            if ui.button("Record").clicked() {
                // Toggle recording
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Go to Start").clicked() {
                app.transport_ui.transport.set_position(0.0);
                ui.close_menu();
            }

            if ui.button("Go to End").clicked() {
                // Go to end of project
                ui.close_menu();
            }

            ui.separator();

            if ui
                .checkbox(
                    &mut app.transport_ui.transport.metronome_enabled,
                    "Metronome",
                )
                .clicked()
            {
                ui.close_menu();
            }

            if ui.button("Tap Tempo").clicked() {
                app.tap_tempo();
                ui.close_menu();
            }
        });
    }

    fn tools_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Tools", |ui| {
            if ui.button("Plugin Manager...").clicked() {
                app.dialogs.show_plugin_manager();
                ui.close_menu();
            }

            if ui.button("Audio Setup...").clicked() {
                app.dialogs.show_audio_setup();
                ui.close_menu();
            }

            ui.separator();

            if app.is_selected_track_midi() {
                ui.menu_button("MIDI Tools", |ui| {
                    if ui.button("Quantize...").clicked() {
                        app.dialogs.show_quantize_dialog();
                        ui.close_menu();
                    }

                    if ui.button("Transpose...").clicked() {
                        app.dialogs.show_transpose_dialog();
                        ui.close_menu();
                    }

                    if ui.button("Humanize...").clicked() {
                        app.dialogs.show_humanize_dialog();
                        ui.close_menu();
                    }
                });
            }

            ui.menu_button("Audio Tools", |ui| {
                if ui.button("Normalize").clicked() {
                    app.normalize_selected();
                    ui.close_menu();
                }

                if ui.button("Reverse").clicked() {
                    app.reverse_selected();
                    ui.close_menu();
                }

                if ui.button("Time Stretch...").clicked() {
                    app.dialogs.show_time_stretch_dialog();
                    ui.close_menu();
                }
            });
        });
    }

    fn window_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Window", |ui| {
            if ui.button("Mixer").clicked() {
                app.mixer_ui.toggle_visibility();
                ui.close_menu();
            }

            if ui.button("Piano Roll").clicked() {
                // Switch to piano roll view
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Reset Layout").clicked() {
                app.reset_layout();
                ui.close_menu();
            }

            if ui.button("Save Layout...").clicked() {
                app.dialogs.show_save_layout_dialog();
                ui.close_menu();
            }

            if ui.button("Load Layout...").clicked() {
                app.dialogs.show_load_layout_dialog();
                ui.close_menu();
            }
        });
    }

    fn help_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.menu_button("Help", |ui| {
            if ui.button("User Manual").clicked() {
                // Open user manual
                ui.close_menu();
            }

            if ui.button("Keyboard Shortcuts").clicked() {
                self.show_keyboard_shortcuts = true;
                ui.close_menu();
            }

            ui.separator();

            if ui.button("About YADAW").clicked() {
                self.show_about = true;
                ui.close_menu();
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
                        ui.hyperlink("https://github.com/yourusername/yadaw");
                    });
                });
        }

        // Preferences dialog
        if self.show_preferences {
            egui::Window::new("Preferences")
                .open(&mut self.show_preferences)
                .resizable(true)
                .default_size(egui::vec2(600.0, 400.0))
                .show(ctx, |ui| {
                    self.draw_preferences(ui, app);
                });
        }

        // Keyboard shortcuts dialog
        if self.show_keyboard_shortcuts {
            egui::Window::new("Keyboard Shortcuts")
                .open(&mut self.show_keyboard_shortcuts)
                .resizable(true)
                .default_size(egui::vec2(400.0, 500.0))
                .show(ctx, |ui| {
                    self.draw_keyboard_shortcuts(ui);
                });
        }
    }

    fn draw_preferences(&self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.horizontal(|ui| {
            // Categories list
            ui.vertical(|ui| {
                ui.set_min_width(150.0);
                ui.selectable_label(true, "Audio");
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
                    ui.label(format!("{}", app.config.audio.buffer_size));
                });

                ui.horizontal(|ui| {
                    ui.label("Sample Rate:");
                    ui.label(format!("{} Hz", app.config.audio.sample_rate));
                });

                ui.separator();

                if ui.button("Apply").clicked() {
                    // Apply settings
                }
            });
        });
    }

    fn draw_keyboard_shortcuts(&self, ui: &mut egui::Ui) {
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
            ui.label("M - Toggle Mixer");
        });
    }
}
