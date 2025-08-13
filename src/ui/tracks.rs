use super::*;
use crate::audio_utils::{format_pan, linear_to_db};
use crate::level_meter::LevelMeter;
use crate::plugin::PluginParameterUpdate;
use crate::state::{AudioCommand, Track};
use crate::track_manager::{arm_track_exclusive, mute_track, solo_track, TrackType};

pub struct TracksPanel {
    track_meters: Vec<LevelMeter>,
    collapsed_groups: Vec<bool>,
    track_width: f32,
    show_mixer_strip: bool,
    show_automation_buttons: bool,
}

impl TracksPanel {
    pub fn new() -> Self {
        Self {
            track_meters: Vec::new(),
            collapsed_groups: Vec::new(),
            track_width: 300.0,
            show_mixer_strip: true,
            show_automation_buttons: true,
        }
    }

    pub fn update_levels(&mut self, levels: Vec<(f32, f32)>) {
        // Ensure we have enough meters
        while self.track_meters.len() < levels.len() {
            self.track_meters.push(LevelMeter::default());
        }

        // Update each meter
        for (i, (left, right)) in levels.iter().enumerate() {
            if let Some(meter) = self.track_meters.get_mut(i) {
                let samples = [left.max(*right)];
                meter.update(&samples, 1.0 / 60.0);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        // Panel header
        ui.horizontal(|ui| {
            ui.heading("Tracks");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("⚙", |ui| {
                    ui.label(format!("Track Options"));
                    ui.separator();

                    if ui.button("Rename...").clicked() {
                        // TODO..
                        ui.close();
                    }

                    if ui.button("Change Color...").clicked() {
                        // TODO
                        ui.close();
                    }

                    if ui.button("Freeze Track").clicked() {
                        // TODO
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Duplicate").clicked() {
                        app.duplicate_selected_track();
                        ui.close();
                    }

                    if ui.button("Delete").clicked() {
                        app.delete_selected_track();
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Track Options");

                ui.toggle_value(&mut self.show_mixer_strip, "🎚")
                    .on_hover_text("Show/Hide Mixer Strip");

                ui.toggle_value(&mut self.show_automation_buttons, "🎛")
                    .on_hover_text("Show/Hide Automation");
            });
        });

        ui.separator();

        // Track list
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_track_list(ui, app);
            });

        // Add track buttons at bottom
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("➕ Audio Track").clicked() {
                app.add_audio_track();
            }

            if ui.button("➕ MIDI Track").clicked() {
                app.add_midi_track();
            }

            if ui.button("➕ Bus").clicked() {
                app.add_bus_track();
            }
        });
    }

    fn draw_track_list(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let mut track_actions = Vec::new();
        let mut selected_track_changed = None;

        {
            let binding = app.state.clone();
            let mut state = binding.lock().unwrap();
            let num_tracks = state.tracks.len();

            for track_idx in 0..num_tracks {
                let is_selected = track_idx == app.selected_track;

                // Track frame
                let response = ui.group(|ui| {
                    self.draw_track_header(
                        ui,
                        &mut state.tracks[track_idx],
                        track_idx,
                        is_selected,
                    );

                    if self.show_mixer_strip {
                        self.draw_mixer_strip(
                            ui,
                            &mut state.tracks[track_idx],
                            track_idx,
                            &mut track_actions,
                        );
                    }

                    if self.show_automation_buttons {
                        self.draw_automation_controls(ui, &state.tracks[track_idx], track_idx, app);
                    }

                    self.draw_plugin_chain(ui, &mut state.tracks[track_idx], track_idx, app);
                });

                // Handle track selection
                if response.response.clicked() {
                    selected_track_changed = Some((track_idx, state.tracks[track_idx].is_midi));
                }

                // Context menu
                response.response.context_menu(|ui| {
                    if ui.button("Duplicate Track").clicked() {
                        track_actions.push(("duplicate", track_idx));
                        ui.close();
                    }

                    if ui.button("Delete Track").clicked() {
                        track_actions.push(("delete", track_idx));
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Add to Group...").clicked() {
                        track_actions.push(("group", track_idx));
                        ui.close();
                    }

                    if ui.button("Track Color...").clicked() {
                        track_actions.push(("color", track_idx));
                        ui.close();
                    }
                });
            }
        }

        if let Some((track_idx, is_midi)) = selected_track_changed {
            println!("Track {} clicked, is_midi: {}", track_idx, is_midi);
            app.selected_track = track_idx;
        }

        // Apply track actions
        for (action, track_idx) in track_actions {
            self.apply_track_action(app, action, track_idx);
        }
    }

    fn draw_track_header(&self, ui: &mut egui::Ui, track: &Track, idx: usize, is_selected: bool) {
        ui.horizontal(|ui| {
            // Track selection indicator
            if is_selected {
                ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "▶");
            } else {
                ui.label(" ");
            }

            // Track name (editable)
            ui.label(&track.name);

            // Track type icon
            ui.label(if track.is_midi { "🎹" } else { "🎵" });

            // Track number
            ui.weak(format!("#{}", idx + 1));
        });
    }

    fn draw_mixer_strip(
        &mut self,
        ui: &mut egui::Ui,
        track: &mut Track,
        idx: usize,
        actions: &mut Vec<(&str, usize)>,
    ) {
        ui.horizontal(|ui| {
            // Mute button
            if ui
                .selectable_label(track.muted, if track.muted { "M" } else { "m" })
                .on_hover_text("Mute")
                .clicked()
            {
                actions.push(("mute", idx));
            }

            // Solo button
            if ui
                .selectable_label(track.solo, if track.solo { "S" } else { "s" })
                .on_hover_text("Solo")
                .clicked()
            {
                actions.push(("solo", idx));
            }

            // Record arm button
            if ui
                .selectable_label(track.armed, if track.armed { "●" } else { "○" })
                .on_hover_text("Record Arm")
                .clicked()
            {
                actions.push(("arm", idx));
            }

            // Input monitoring
            if !track.is_midi {
                if ui
                    .selectable_label(false, "🎧")
                    .on_hover_text("Input Monitoring")
                    .clicked()
                {
                    actions.push(("monitor", idx));
                }
            }
        });

        // Volume fader
        ui.horizontal(|ui| {
            ui.label("Vol:");
            ui.add(
                egui::Slider::new(&mut track.volume, 0.0..=1.2)
                    .show_value(false)
                    .logarithmic(true),
            );
            ui.label(format!("{:.1}", linear_to_db(track.volume)));
        });

        // Pan knob
        ui.horizontal(|ui| {
            ui.label("Pan:");
            ui.add(egui::Slider::new(&mut track.pan, -1.0..=1.0).show_value(false));

            let pan_text = format_pan(track.pan);
            ui.label(pan_text);
        });

        // Level meter
        if idx < self.track_meters.len() {
            self.track_meters[idx].ui(ui, false);
        }
    }

    fn draw_automation_controls(
        &self,
        ui: &mut egui::Ui,
        track: &Track,
        idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        ui.horizontal(|ui| {
            ui.label("Automation:");

            ui.menu_button("➕", |ui| {
                if ui.button("Volume").clicked() {
                    app.add_automation_lane(idx, crate::state::AutomationTarget::TrackVolume);
                    ui.close();
                }

                if ui.button("Pan").clicked() {
                    app.add_automation_lane(idx, crate::state::AutomationTarget::TrackPan);
                    ui.close();
                }

                ui.separator();

                // Plugin parameters
                for (plugin_idx, plugin) in track.plugin_chain.iter().enumerate() {
                    ui.menu_button(&plugin.name, |ui| {
                        for (param_name, param) in &plugin.params {
                            if ui.button(&param.name).clicked() {
                                app.add_automation_lane(
                                    idx,
                                    crate::state::AutomationTarget::PluginParam {
                                        plugin_idx,
                                        param_name: param_name.clone(),
                                    },
                                );
                                ui.close();
                            }
                        }
                    });
                }
            });

            if !track.automation_lanes.is_empty() {
                ui.label(format!("({} lanes)", track.automation_lanes.len()));
            }
        });
    }

    fn draw_plugin_chain(
        &self,
        ui: &mut egui::Ui,
        track: &mut Track,
        idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Plugins:");
            if ui.button("➕").clicked() {
                app.show_plugin_browser_for_track(idx);
            }
        });

        // Plugin list
        let mut plugin_to_remove = None;

        for (plugin_idx, plugin) in track.plugin_chain.iter_mut().enumerate() {
            ui.collapsing(&plugin.name, |ui| {
                ui.horizontal(|ui| {
                    // Bypass toggle
                    if ui.checkbox(&mut plugin.bypass, "Bypass").changed() {
                        let _ = app.command_tx.send(AudioCommand::SetPluginBypass(
                            idx,
                            plugin_idx,
                            plugin.bypass,
                        ));
                    }

                    // Remove button
                    if ui.small_button("✕").clicked() {
                        plugin_to_remove = Some(plugin_idx);
                    }
                });

                // Plugin parameters
                for (param_name, param) in &mut plugin.params {
                    ui.horizontal(|ui| {
                        ui.label(&param.name);

                        let mut value = param.value;
                        if ui
                            .add(
                                egui::Slider::new(&mut value, param.min..=param.max)
                                    .show_value(true),
                            )
                            .changed()
                        {
                            let update = PluginParameterUpdate {
                                track_id: idx,
                                plugin_idx,
                                param_name: param_name.clone(),
                                value,
                            };

                            param.value = value;
                            let _ = app.command_tx.send(update.create_command());
                        }

                        // Reset button
                        if ui
                            .small_button("↺")
                            .on_hover_text("Reset to default")
                            .clicked()
                        {
                            param.value = param.default;
                            let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                                idx,
                                plugin_idx,
                                param_name.clone(),
                                param.default,
                            ));
                        }
                    });
                }
            });
        }

        if let Some(idx_to_remove) = plugin_to_remove {
            let _ = app
                .command_tx
                .send(AudioCommand::RemovePlugin(idx, idx_to_remove));
        }
    }

    fn apply_track_action(
        &mut self,
        app: &mut super::app::YadawApp,
        action: &str,
        track_idx: usize,
    ) {
        let mut state = app.state.lock().unwrap();

        match action {
            "mute" => mute_track(&mut state.tracks, track_idx, &app.command_tx),
            "solo" => solo_track(&mut state.tracks, track_idx, &app.command_tx),
            "arm" => arm_track_exclusive(&mut state.tracks, track_idx),
            "duplicate" => {
                if let Some(track) = state.tracks.get(track_idx) {
                    let new_track = track.clone();
                    state.tracks.insert(track_idx + 1, new_track);
                }
            }
            "delete" => {
                if state.tracks.len() > 1 {
                    state.tracks.remove(track_idx);
                    if app.selected_track >= state.tracks.len() {
                        app.selected_track = state.tracks.len() - 1;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Default for TracksPanel {
    fn default() -> Self {
        Self::new()
    }
}
