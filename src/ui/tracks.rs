use super::*;
use crate::audio_utils::{format_pan, linear_to_db};
use crate::level_meter::LevelMeter;
use crate::messages::AudioCommand;
use crate::model::automation::AutomationTarget;
use crate::model::track::Track;
use crate::plugin::get_control_port_info;
use crate::track_manager::{arm_track_exclusive, mute_track, solo_track};

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
        while self.track_meters.len() < levels.len() {
            self.track_meters.push(LevelMeter::default());
        }
        for (i, (left, right)) in levels.iter().enumerate() {
            if let Some(meter) = self.track_meters.get_mut(i) {
                let samples = [left.max(*right)];
                meter.update(&samples, 1.0 / 60.0);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.horizontal(|ui| {
            ui.heading("Tracks");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("⚙", |ui| {
                    ui.label("Track Options");
                    ui.separator();
                    if ui.button("Rename...").clicked() {
                        ui.close();
                    }
                    if ui.button("Change Color...").clicked() {
                        ui.close();
                    }
                    if ui.button("Freeze Track").clicked() {
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
                });
                ui.toggle_value(&mut self.show_mixer_strip, "🎚")
                    .on_hover_text("Show/Hide Mixer Strip");
                ui.toggle_value(&mut self.show_automation_buttons, "🎛")
                    .on_hover_text("Show/Hide Automation");
            });
        });

        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_track_list(ui, app);
            });

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
        let mut automation_actions = Vec::new(); // ← NEW: collect automation actions

        // Sanitize groups
        {
            let len = app.state.lock().unwrap().tracks.len();
            app.track_manager.sanitize(len);
        }

        let groups = app.track_manager.get_groups().to_vec();
        while self.collapsed_groups.len() < groups.len() {
            self.collapsed_groups.push(false);
        }

        // Build grouped set
        use std::collections::HashSet;
        let mut grouped: HashSet<usize> = HashSet::new();
        for g in &groups {
            for &i in &g.track_ids {
                grouped.insert(i);
            }
        }

        // Draw groups
        for (gidx, g) in groups.iter().enumerate() {
            let mut collapsed = *self.collapsed_groups.get(gidx).unwrap_or(&false);

            self.draw_group_block(
                ui,
                app,
                &g.name,
                &g.track_ids,
                &mut collapsed,
                &mut track_actions,
                &mut selected_track_changed,
                &mut automation_actions,
            );

            if self.collapsed_groups.len() <= gidx {
                self.collapsed_groups.resize(gidx + 1, false);
            }
            self.collapsed_groups[gidx] = collapsed;
            ui.add_space(6.0);
        }

        // Ungrouped tracks
        let command_tx = app.command_tx.clone();

        {
            let binding = app.state.clone();
            let mut state = binding.lock().unwrap();
            let num_tracks = state.tracks.len();

            for track_idx in 0..num_tracks {
                if grouped.contains(&track_idx) {
                    continue;
                }
                let is_selected = track_idx == app.selected_track;

                let row = ui.group(|ui| {
                    let header_resp = self.draw_track_header(
                        ui,
                        &state.tracks[track_idx],
                        track_idx,
                        is_selected,
                        &mut track_actions,
                    );
                    if header_resp.clicked() {
                        selected_track_changed = Some((track_idx, state.tracks[track_idx].is_midi));
                    }

                    if self.show_mixer_strip {
                        self.draw_mixer_strip(
                            ui,
                            &mut state.tracks[track_idx],
                            track_idx,
                            &mut track_actions,
                            &command_tx,
                        );
                    }

                    if self.show_automation_buttons
                        && let Some(action) =
                            self.draw_automation_controls(ui, &state.tracks[track_idx], track_idx)
                        {
                            automation_actions.push(action); // ← Collect for later
                        }

                    self.draw_plugin_chain(ui, &mut state.tracks[track_idx], track_idx, app);
                });

                row.response.context_menu(|ui| {
                    if ui.button("Duplicate Selected Track").clicked() {
                        track_actions.push(("duplicate", track_idx));
                        ui.close();
                    }
                    if ui.button("Delete Selected Track").clicked() {
                        track_actions.push(("delete", track_idx));
                        ui.close();
                    }
                });
            }
        }

        // Apply automation actions
        for (idx, target) in automation_actions {
            app.add_automation_lane(idx, target);
        }

        // Handle selection change
        if let Some((track_idx, is_midi)) = selected_track_changed {
            app.selected_track = track_idx;
        }

        // Apply track actions
        for (action, track_idx) in track_actions {
            self.apply_track_action(app, action, track_idx);
        }
    }

    fn draw_track_header(
        &self,
        ui: &mut egui::Ui,
        track: &Track,
        idx: usize,
        is_selected: bool,
        actions: &mut Vec<(&str, usize)>,
    ) -> egui::Response {
        const HEADER_H: f32 = 24.0;

        let desired = egui::vec2(ui.available_width(), HEADER_H);
        let (rect, bg_resp) = ui.allocate_exact_size(desired, egui::Sense::click());

        if is_selected {
            let visuals = ui.visuals();
            ui.painter()
                .rect_filled(rect, 0.0, visuals.selection.bg_fill);
        }

        ui.allocate_ui_at_rect(rect, |ui| {
            ui.horizontal(|ui| {
                if is_selected {
                    ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "▶");
                } else {
                    ui.label(" ");
                }
                ui.label(&track.name);
                ui.label(if track.is_midi { "🎹" } else { "🎵" });
                ui.weak(format!("#{}", idx + 1));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⚙", |ui| {
                        ui.label("Track Options");
                        ui.separator();
                        if ui.button("Rename…").clicked() {
                            actions.push(("rename", idx));
                            ui.close();
                        }
                        if ui.button("Change Color…").clicked() {
                            actions.push(("color", idx));
                            ui.close();
                        }
                        if ui.button("Freeze Track").clicked() {
                            actions.push(("freeze", idx));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Duplicate").clicked() {
                            actions.push(("duplicate", idx));
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            actions.push(("delete", idx));
                            ui.close();
                        }
                    });
                });
            });
        });

        bg_resp
    }

    fn draw_mixer_strip(
        &mut self,
        ui: &mut egui::Ui,
        track: &mut Track,
        idx: usize,
        actions: &mut Vec<(&str, usize)>,
        command_tx: &Sender<AudioCommand>,
    ) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(track.muted, if track.muted { "M" } else { "m" })
                .on_hover_text("Mute")
                .clicked()
            {
                actions.push(("mute", idx));
            }
            if ui
                .selectable_label(track.solo, if track.solo { "S" } else { "s" })
                .on_hover_text("Solo")
                .clicked()
            {
                actions.push(("solo", idx));
            }
            if ui
                .selectable_label(track.armed, if track.armed { "●" } else { "○" })
                .on_hover_text("Record Arm")
                .clicked()
            {
                actions.push(("arm", idx));
            }
            if !track.is_midi
                && ui
                    .selectable_label(track.monitor_enabled, "🎧")
                    .on_hover_text("Input Monitoring")
                    .clicked()
                {
                    // Toggle and notify audio thread
                    track.monitor_enabled = !track.monitor_enabled;
                    let _ =
                        command_tx.send(AudioCommand::SetTrackMonitor(idx, track.monitor_enabled));
                }
        });

        ui.horizontal(|ui| {
            ui.label("Vol:");
            ui.add(
                egui::Slider::new(&mut track.volume, 0.0..=1.2)
                    .show_value(false)
                    .logarithmic(true),
            );
            ui.label(format!("{:.1}", linear_to_db(track.volume)));
        });

        ui.horizontal(|ui| {
            ui.label("Pan:");
            ui.add(egui::Slider::new(&mut track.pan, -1.0..=1.0).show_value(false));
            ui.label(format_pan(track.pan));
        });

        if idx < self.track_meters.len() {
            self.track_meters[idx].ui(ui, false);
        }
    }

    fn draw_automation_controls(
        &self,
        ui: &mut egui::Ui,
        track: &Track,
        idx: usize,
    ) -> Option<(usize, AutomationTarget)> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label("Automation:");
            ui.menu_button("➕", |ui| {
                if ui.button("Volume").clicked() {
                    action = Some((idx, AutomationTarget::TrackVolume));
                    ui.close();
                }
                if ui.button("Pan").clicked() {
                    action = Some((idx, AutomationTarget::TrackPan));
                    ui.close();
                }
                ui.separator();
                for (plugin_idx, plugin) in track.plugin_chain.iter().enumerate() {
                    ui.menu_button(&plugin.name, |ui| {
                        for param_name in plugin.params.keys() {
                            if ui.button(param_name).clicked() {
                                action = Some((
                                    idx,
                                    AutomationTarget::PluginParam {
                                        plugin_idx,
                                        param_name: param_name.clone(),
                                    },
                                ));
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
        action
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

        let mut plugin_to_remove = None;

        for (plugin_idx, plugin) in track.plugin_chain.iter_mut().enumerate() {
            ui.collapsing(&plugin.name, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut plugin.bypass, "Bypass").changed() {
                        let _ = app.command_tx.send(AudioCommand::SetPluginBypass(
                            idx,
                            plugin_idx,
                            plugin.bypass,
                        ));
                    }
                    if ui.small_button("✕").clicked() {
                        plugin_to_remove = Some(plugin_idx);
                    }
                });

                for (pname, val) in plugin.params.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.label(pname);

                        let meta = get_control_port_info(&plugin.uri, pname);
                        let (min_v, max_v, default_v) = match meta.as_ref() {
                            Some(m) => (m.min, m.max, m.default),
                            None => (0.0, 1.0, 0.0),
                        };

                        let mut v = *val;
                        if ui
                            .add(egui::Slider::new(&mut v, min_v..=max_v).show_value(true))
                            .changed()
                        {
                            *val = v;
                            let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                                idx,
                                plugin_idx,
                                pname.clone(),
                                v,
                            ));
                        }

                        if ui
                            .small_button("↺")
                            .on_hover_text(format!("Reset to default ({:.3})", default_v))
                            .clicked()
                        {
                            *val = default_v;
                            let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                                idx,
                                plugin_idx,
                                pname.clone(),
                                default_v,
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
            "rename" => {
                let current = state
                    .tracks
                    .get(track_idx)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                drop(state);
                app.dialogs.show_rename_track(track_idx, current);
            }
            "mute" => {
                mute_track(&mut state.tracks, track_idx, &app.command_tx);
            }
            "solo" => {
                solo_track(&mut state.tracks, track_idx, &app.command_tx);
            }
            "arm" => {
                arm_track_exclusive(&mut state.tracks, track_idx);
                drop(state);
                let _ = app.command_tx.send(AudioCommand::UpdateTracks);
            }
            "duplicate" => {
                if let Some(track) = state.tracks.get(track_idx).cloned() {
                    let dup = app.track_manager.duplicate_track(&track);
                    state.tracks.insert(track_idx + 1, dup);
                    state.ensure_ids();
                }
                drop(state);
                let _ = app.command_tx.send(AudioCommand::UpdateTracks);
            }
            "delete" => {
                if state.tracks.len() > 1 {
                    state.tracks.remove(track_idx);
                    if app.selected_track >= state.tracks.len() {
                        app.selected_track = state.tracks.len().saturating_sub(1);
                    }
                }
                drop(state);
                let _ = app.command_tx.send(AudioCommand::UpdateTracks);
            }
            _ => {}
        }
    }

    fn draw_group_block(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut super::app::YadawApp,
        name: &str,
        track_ids: &[usize],
        collapsed: &mut bool,
        track_actions: &mut Vec<(&str, usize)>,
        selected_track_changed: &mut Option<(usize, bool)>,
        automation_actions: &mut Vec<(usize, AutomationTarget)>, // ← ADD THIS
    ) {
        // Header
        let hdr = ui.collapsing(name, |_ui| {});
        if hdr.header_response.clicked() {
            *collapsed = !*collapsed;
        }
        // Optional header context menu
        hdr.header_response.context_menu(|ui| {
            if ui
                .button(if *collapsed { "Expand" } else { "Collapse" })
                .clicked()
            {
                *collapsed = !*collapsed;
                ui.close();
            }
        });

        if *collapsed {
            return;
        }

        // Extract command_tx before locking
        let command_tx = app.command_tx.clone();

        let binding = app.state.clone();
        let mut state = binding.lock().unwrap();

        for &track_idx in track_ids {
            if track_idx >= state.tracks.len() {
                continue;
            }
            let is_selected = track_idx == app.selected_track;

            let row = ui.group(|ui| {
                let header_resp = self.draw_track_header(
                    ui,
                    &state.tracks[track_idx],
                    track_idx,
                    is_selected,
                    track_actions,
                );
                if header_resp.clicked() {
                    *selected_track_changed = Some((track_idx, state.tracks[track_idx].is_midi));
                }

                if self.show_mixer_strip {
                    self.draw_mixer_strip(
                        ui,
                        &mut state.tracks[track_idx],
                        track_idx,
                        track_actions,
                        &command_tx,
                    );
                }

                if self.show_automation_buttons
                    && let Some(action) =
                        self.draw_automation_controls(ui, &state.tracks[track_idx], track_idx)
                    {
                        automation_actions.push(action);
                    }

                self.draw_plugin_chain(ui, &mut state.tracks[track_idx], track_idx, app);
            });

            row.response.context_menu(|ui| {
                if ui.button("Duplicate Track").clicked() {
                    track_actions.push(("duplicate", track_idx));
                    ui.close();
                }
                if ui.button("Delete Track").clicked() {
                    track_actions.push(("delete", track_idx));
                    ui.close();
                }
            });
        }
    }
}

impl Default for TracksPanel {
    fn default() -> Self {
        Self::new()
    }
}
