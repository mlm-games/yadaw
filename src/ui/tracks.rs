use std::collections::HashMap;

use super::*;
use crate::audio_utils::{format_pan, linear_to_db};
use crate::level_meter::LevelMeter;
use crate::messages::AudioCommand;
use crate::model::PluginDescriptor;
use crate::model::automation::AutomationTarget;
use crate::plugin::get_control_port_info;
use crate::project::AppState;

pub struct TracksPanel {
    track_meters: HashMap<u64, LevelMeter>,
    show_mixer_strip: bool,
    show_automation_buttons: bool,
    show_inputs: bool,
    cached_plugin_chains: HashMap<u64, (u64, Vec<PluginDescriptor>)>,

    dnd_dragging_track: Option<u64>,
    dnd_dragging_from_idx: Option<usize>,
    dnd_drop_target_idx: Option<usize>,
    dnd_row_rects: Vec<(u64, egui::Rect, usize)>, // (track_id, header_rect, index)
    dnd_pointer_offset: egui::Vec2,
}

impl TracksPanel {
    pub fn new() -> Self {
        Self {
            track_meters: HashMap::new(),
            show_mixer_strip: true,
            show_automation_buttons: true,
            show_inputs: true,
            cached_plugin_chains: HashMap::new(),

            dnd_dragging_track: None,
            dnd_dragging_from_idx: None,
            dnd_drop_target_idx: None,
            dnd_row_rects: Vec::new(),
            dnd_pointer_offset: egui::Vec2::ZERO,
        }
    }

    pub fn update_levels(&mut self, levels: HashMap<u64, (f32, f32)>) {
        for (track_id, (left, right)) in levels {
            let meter = self.track_meters.entry(track_id).or_default();
            let samples = [left.max(right)];
            meter.update(&samples, 1.0 / 60.0);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.horizontal(|ui| {
            ui.heading("Tracks");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_mixer_strip, "🎚")
                    .on_hover_text("Show/Hide Mixer Strip");
                ui.toggle_value(&mut self.show_automation_buttons, "🎛")
                    .on_hover_text("Show/Hide Automation");
                ui.toggle_value(&mut self.show_inputs, "🔣")
                    .on_hover_text("Show/Hide Input Options");
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
        let mut automation_actions = Vec::new();

        // Get ordered track IDs and clone them to avoid holding the lock
        let track_ids = {
            let state = app.state.lock().unwrap();
            state.track_order.clone()
        };

        for &track_id in track_ids.iter() {
            let is_selected = track_id == app.selected_track;

            // Build the whole track UI inside a group and return the header response
            let header_resp = ui
                .group(|ui| {
                    let header_resp =
                        self.draw_track_header(ui, track_id, is_selected, app, |action| {
                            track_actions.push((action, track_id))
                        });

                    if self.show_mixer_strip {
                        self.draw_mixer_strip(ui, track_id, app);
                    }

                    if self.show_automation_buttons {
                        if let Some(action) = self.draw_automation_controls(ui, track_id, app) {
                            automation_actions.push(action);
                        }
                    }

                    self.draw_plugin_chain(ui, track_id, app);

                    if self.show_inputs {
                        self.draw_io_section(ui, track_id, app);
                    }

                    header_resp
                })
                .inner;

            // Select the track when the header is clicked
            if header_resp.clicked() {
                app.selected_track = track_id;
            }

            // record the header rect and logical index for DnD
            let idx_in_order = {
                let st = app.state.lock().unwrap();
                st.track_order
                    .iter()
                    .position(|&id| id == track_id)
                    .unwrap_or(0)
            };
            self.dnd_row_rects
                .push((track_id, header_resp.rect, idx_in_order));

            // start dragging from header
            if header_resp.drag_started() && self.dnd_dragging_track.is_none() {
                self.dnd_dragging_track = Some(track_id);
                self.dnd_dragging_from_idx = Some(idx_in_order);
                if let Some(pointer) = header_resp.interact_pointer_pos() {
                    self.dnd_pointer_offset = pointer - header_resp.rect.left_top();
                } else {
                    self.dnd_pointer_offset = egui::Vec2::ZERO;
                }
            }
        }

        self.handle_track_dnd(ui, app);

        // Apply actions after the main loop to avoid borrow issues
        for (action, track_id) in track_actions {
            self.apply_track_action(app, action, track_id);
        }
        for (track_id, target) in automation_actions {
            app.add_automation_lane_by_id(track_id, target);
        }
    }

    fn draw_track_header<'a>(
        &self,
        ui: &mut egui::Ui,
        track_id: u64,
        is_selected: bool,
        app: &super::app::YadawApp,
        mut on_action: impl FnMut(&'a str),
    ) -> egui::Response {
        // Query current name and type
        let (name, is_midi) = {
            let state = app.state.lock().unwrap();
            state
                .tracks
                .get(&track_id)
                .map(|t| (t.name.clone(), t.is_midi))
                .unwrap_or_else(|| ("Unknown".to_string(), false))
        };

        // Draw a framed header
        let inner = egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                // Selected marker
                if is_selected {
                    ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "▶");
                } else {
                    ui.label(" ");
                }

                // Tiny intensity viewer (uses your existing LevelMeter; non-vertical)
                if let Some(meter) = self.track_meters.get(&track_id) {
                    // render compactly
                    ui.scope(|ui| {
                        // Shrink spacing to keep header height reasonable
                        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                        ui.add(egui::Separator::default().spacing(4.0));
                        // Draw in a small reserved space
                        let (resp, painter) =
                            ui.allocate_painter(egui::vec2(60.0, 10.0), egui::Sense::hover());
                        // Re-use meter painter (horizontal)
                        // Note: LevelMeter::ui draws its own size; here we provide a minimal inline bar.
                        // If you prefer the existing visual, replace this block with: meter.ui(ui, false);
                        // Simple inline bar: draw a filled rect proportional to peak
                        let peak = meter.clone().data.peak_normalized(); // if data is private, fallback to a fixed small bar
                        let w = (resp.rect.width() * peak).clamp(0.0, resp.rect.width());
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                resp.rect.left_top(),
                                egui::vec2(w, resp.rect.height()),
                            ),
                            1.0,
                            egui::Color32::from_rgb(90, 180, 90),
                        );
                        painter.rect_stroke(
                            resp.rect,
                            1.0,
                            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                            egui::StrokeKind::Middle,
                        );
                    });
                }

                ui.label(name);
                ui.label(if is_midi { "🎹" } else { "🎵" });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⚙", |ui| {
                        ui.label("Track Options");
                        ui.separator();
                        if ui.button("Rename…").clicked() {
                            on_action("rename");
                            ui.close();
                        }
                        if ui.button("Duplicate").clicked() {
                            on_action("duplicate");
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            on_action("delete");
                            ui.close();
                        }
                    });
                });
            });
        });

        let rect = inner.response.rect;
        let id = ui.id().with(("track_header", track_id));

        // Reserve a right edge area so it stays clickable
        let reserved_w = 48.0;
        let drag_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2((rect.right() - reserved_w).max(rect.left()), rect.bottom()),
        );

        // Drag only on the left area
        let drag_resp = ui.interact(drag_rect, id, egui::Sense::click_and_drag());

        // highlight around header
        if is_selected {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                3.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 255)),
                egui::StrokeKind::Middle,
            );
        }

        // combined response
        drag_resp.union(inner.response)
    }

    fn draw_mixer_strip(&mut self, ui: &mut egui::Ui, track_id: u64, app: &super::app::YadawApp) {
        // Read the track's current state
        let (mut volume, mut pan, muted, solo, armed, monitor_enabled, is_midi) = {
            let state = app.state.lock().unwrap();
            state
                .tracks
                .get(&track_id)
                .map(|t| {
                    (
                        t.volume,
                        t.pan,
                        t.muted,
                        t.solo,
                        t.armed,
                        t.monitor_enabled,
                        t.is_midi,
                    )
                })
                .unwrap_or((0.7, 0.0, false, false, false, false, false))
        };

        ui.horizontal(|ui| {
            if ui
                .selectable_label(muted, if muted { "M" } else { "m" })
                .on_hover_text("Mute")
                .clicked()
            {
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetTrackMute(track_id, !muted));
            }
            if ui
                .selectable_label(solo, if solo { "S" } else { "s" })
                .on_hover_text("Solo")
                .clicked()
            {
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetTrackSolo(track_id, !solo));
            }
            if ui
                .selectable_label(armed, if armed { "●" } else { "○" })
                .on_hover_text("Record Arm")
                .clicked()
            {
                let _ = app
                    .command_tx
                    .send(AudioCommand::ArmForRecording(track_id, !armed));
            }
            if !is_midi
                && ui
                    .selectable_label(monitor_enabled, "🎧")
                    .on_hover_text("Input Monitoring")
                    .clicked()
            {
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetTrackMonitor(track_id, !monitor_enabled));
            }
        });

        ui.horizontal(|ui| {
            ui.label("Vol:");
            if ui
                .add(
                    egui::Slider::new(&mut volume, 0.0..=1.2)
                        .show_value(false)
                        .logarithmic(true),
                )
                .changed()
            {
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetTrackVolume(track_id, volume));
            }
            ui.label(format!("{:.1}", linear_to_db(volume)));
        });

        ui.horizontal(|ui| {
            ui.label("Pan:");
            if ui
                .add(egui::Slider::new(&mut pan, -1.0..=1.0).show_value(false))
                .changed()
            {
                let _ = app
                    .command_tx
                    .send(AudioCommand::SetTrackPan(track_id, pan));
            }
            ui.label(format_pan(pan));
        });
    }

    fn draw_automation_controls(
        &self,
        ui: &mut egui::Ui,
        track_id: u64,
        app: &super::app::YadawApp,
    ) -> Option<(u64, AutomationTarget)> {
        let mut action = None;
        let (plugin_chain, num_lanes) = {
            let state = app.state.lock().unwrap();
            state
                .tracks
                .get(&track_id)
                .map(|t| (t.plugin_chain.clone(), t.automation_lanes.len()))
                .unwrap_or_default()
        };

        ui.horizontal(|ui| {
            ui.label("Automation:");
            ui.menu_button("+", |ui| {
                if ui.button("Volume").clicked() {
                    action = Some((track_id, AutomationTarget::TrackVolume));
                    ui.close();
                }
                if ui.button("Pan").clicked() {
                    action = Some((track_id, AutomationTarget::TrackPan));
                    ui.close();
                }
                ui.separator();
                for plugin in &plugin_chain {
                    let plugin_id = plugin.id;
                    let param_names: Vec<_> = plugin.params.keys().cloned().collect();
                    ui.menu_button(&plugin.name, |ui| {
                        for param_name in param_names {
                            if ui.button(&param_name).clicked() {
                                action = Some((
                                    track_id,
                                    AutomationTarget::PluginParam {
                                        plugin_id,
                                        param_name,
                                    },
                                ));
                                ui.close();
                            }
                        }
                    });
                }
            });
            if num_lanes > 0 {
                ui.label(format!("({} lanes)", num_lanes));
            }
        });
        action
    }

    fn draw_plugin_chain(
        &mut self,
        ui: &mut egui::Ui,
        track_id: u64,
        app: &mut super::app::YadawApp,
    ) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Plugins:");
            if ui.button("+").clicked() {
                app.show_plugin_browser_for_track(track_id);
            }
        });

        let mut plugin_to_remove: Option<u64> = None;
        let mut move_action: Option<(usize, usize)> = None;

        // Get cached chain to avoid cloning every frame
        let chain_len = {
            let state = app.state.lock().unwrap();
            state
                .tracks
                .get(&track_id)
                .map(|t| t.plugin_chain.len())
                .unwrap_or(0)
        };

        // Only lock when we need to read plugin data
        for plugin_idx in 0..chain_len {
            let (plugin_id, plugin_name, plugin_uri, bypass, params) = {
                let state = app.state.lock().unwrap();
                let track = match state.tracks.get(&track_id) {
                    Some(t) => t,
                    None => continue,
                };

                let plugin = match track.plugin_chain.get(plugin_idx) {
                    Some(p) => p,
                    None => continue,
                };

                (
                    plugin.id,
                    plugin.name.clone(),
                    plugin.uri.clone(),
                    plugin.bypass,
                    plugin.params.clone(),
                )
            };

            let mut bypass_local = bypass;

            egui::CollapsingHeader::new(&plugin_name)
                .id_salt(("plugin", track_id, plugin_id))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut bypass_local, "Bypass").changed() {
                            let _ = app.command_tx.send(AudioCommand::SetPluginBypass(
                                track_id,
                                plugin_id,
                                bypass_local,
                            ));
                        }
                        if ui.small_button("✕ (Remove)").clicked() {
                            plugin_to_remove = Some(plugin_id);
                        }
                        if plugin_idx > 0 && ui.small_button("▲ (Up)").clicked() {
                            move_action = Some((plugin_idx, plugin_idx - 1));
                        }
                        if plugin_idx < chain_len - 1 && ui.small_button("▼ (Down)").clicked() {
                            move_action = Some((plugin_idx, plugin_idx + 1));
                        }
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.menu_button("Presets ▾", |ui| {
                            // Save snapshot (timestamped name)
                            if ui.button("Save Snapshot").clicked() {
                                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                                let preset_name = format!("Snapshot_{}", ts);
                                let _ = app.command_tx.send(
                                    crate::messages::AudioCommand::SavePluginPreset(
                                        track_id,
                                        plugin_idx,
                                        preset_name,
                                    ),
                                );
                                ui.close();
                            }

                            // List existing presets for this plugin URI
                            let presets = crate::presets::list_presets_for(&plugin_uri);
                            if presets.is_empty() {
                                ui.label(egui::RichText::new("(no presets)").weak());
                            } else {
                                ui.separator();
                                for pname in presets {
                                    if ui.button(&pname).clicked() {
                                        let _ = app.command_tx.send(
                                            crate::messages::AudioCommand::LoadPluginPreset(
                                                track_id, plugin_idx, pname,
                                            ),
                                        );
                                        ui.close();
                                    }
                                }
                            }
                        });
                    });

                    // Draw parameters
                    for (pname, &pval) in &params {
                        let mut v = pval;
                        ui.horizontal(|ui| {
                            ui.label(pname);

                            let meta = get_control_port_info(&plugin_uri, pname);
                            let (min_v, max_v, default_v) = meta
                                .as_ref()
                                .map(|m| (m.min, m.max, m.default))
                                .unwrap_or((0.0, 1.0, 0.0));

                            if ui
                                .add(egui::Slider::new(&mut v, min_v..=max_v).show_value(true))
                                .changed()
                            {
                                let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                                    track_id,
                                    plugin_id,
                                    pname.clone(),
                                    v,
                                ));
                            }
                            if ui
                                .small_button("↺")
                                .on_hover_text(format!("Reset to default ({:.3})", default_v))
                                .clicked()
                            {
                                let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                                    track_id,
                                    plugin_id,
                                    pname.clone(),
                                    default_v,
                                ));
                            }
                        });
                    }
                });
        }

        // Apply actions after iteration
        if let Some(id_to_remove) = plugin_to_remove {
            let _ = app
                .command_tx
                .send(AudioCommand::RemovePlugin(track_id, id_to_remove));
            // Invalidate cache
            self.cached_plugin_chains.remove(&track_id);
        }

        if let Some((from, to)) = move_action {
            let _ = app
                .command_tx
                .send(AudioCommand::MovePlugin(track_id, from, to));
            self.cached_plugin_chains.remove(&track_id);
        }
    }

    fn draw_io_section(&self, ui: &mut egui::Ui, track_id: u64, app: &mut super::app::YadawApp) {
        let track = {
            let state = app.state.lock().unwrap();
            state.tracks.get(&track_id).cloned()
        };

        if let Some(track) = track {
            if track.is_midi {
                ui.horizontal(|ui| {
                    ui.label("MIDI In:");

                    let mut selected_port = track
                        .midi_input_port
                        .clone()
                        .unwrap_or_else(|| "None".to_string());

                    let response = egui::ComboBox::from_id_salt(("midi_in", track_id))
                        .selected_text(&selected_port)
                        .show_ui(ui, |ui| {
                            let mut changed = ui
                                .selectable_value(&mut selected_port, "None".to_string(), "None")
                                .changed();
                            for port_name in &app.available_midi_ports {
                                changed |= ui
                                    .selectable_value(
                                        &mut selected_port,
                                        port_name.clone(),
                                        port_name,
                                    )
                                    .changed();
                            }
                            changed
                        });

                    if response.inner.unwrap_or(false) {
                        // Check if the value changed
                        let new_selection = if selected_port == "None" {
                            None
                        } else {
                            Some(selected_port)
                        };

                        let _ = app
                            .command_tx
                            .send(AudioCommand::SetTrackMidiInput(track_id, new_selection));
                    }
                });
            } else {
                // Placeholder for audio input selection
                ui.horizontal(|ui| {
                    ui.label("Audio In:");
                    ui.label("Default Input");
                });
            }
        }
    }

    fn handle_track_dnd(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let Some(drag_id) = self.dnd_dragging_track else {
            return;
        };

        // Compute drop target index using pointer Y against row centers
        let pointer = match ui.ctx().input(|i| i.pointer.interact_pos()) {
            Some(p) => p,
            None => return,
        };

        // Sort rows by index to get visual order
        let mut rows = self.dnd_row_rects.clone();
        rows.sort_by_key(|(_, _, idx)| *idx);

        let mut target_idx = rows.len();
        for (i, (_tid, rect, _idx)) in rows.iter().enumerate() {
            let center_y = rect.center().y;
            if pointer.y < center_y {
                target_idx = i;
                break;
            }
        }
        self.dnd_drop_target_idx = Some(target_idx);

        // Paint insertion line and ghost
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tracks_dnd_layer"));
        let painter = ui.ctx().layer_painter(layer);

        // draw insertion line spanning header width
        if let Some((_tid, any_rect, _)) = rows.first() {
            let x0 = any_rect.left();
            let x1 = any_rect.right();
            let y = if target_idx == rows.len() {
                // after last -> line below last row
                rows.last().unwrap().1.bottom()
            } else {
                rows[target_idx].1.top()
            };
            painter.line_segment(
                [egui::pos2(x0, y), egui::pos2(x1, y)],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
            );
        }

        // ghost: show semi-transparent preview following pointer, using dragged row size
        if let Some((_, src_rect, _)) = rows.iter().find(|(tid, _, _)| *tid == drag_id) {
            let pos = pointer - self.dnd_pointer_offset;
            let ghost_rect = egui::Rect::from_min_size(pos, src_rect.size());
            painter.rect_filled(
                ghost_rect,
                4.0,
                egui::Color32::from_rgba_unmultiplied(90, 140, 255, 60),
            );
            painter.rect_stroke(
                ghost_rect,
                4.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
                egui::StrokeKind::Inside,
            );
        }

        // Drop
        let released = ui.ctx().input(|i| i.pointer.any_released());
        if released {
            let from = self.dnd_dragging_from_idx.unwrap_or(0);
            let to = self.dnd_drop_target_idx.unwrap_or(from);

            // Resolve track ids by order
            let (ids, len) = {
                let st = app.state.lock().unwrap();
                (st.track_order.clone(), st.track_order.len())
            };

            if len >= 2 && from < len {
                let mut new_to = to.min(len);
                // When dragging downward, removing first shifts indices
                if new_to > from {
                    new_to = new_to.saturating_sub(1);
                }
                if new_to != from && new_to < len {
                    // Apply reorder in AppState
                    use crate::track_manager::move_track;
                    {
                        let mut st = app.state.lock().unwrap();
                        move_track(&mut st.track_order, from, new_to);
                    }
                }

                app.selected_track = drag_id;
                let _ = app
                    .command_tx
                    .send(crate::messages::AudioCommand::UpdateTracks);
            }

            // Clear DnD state
            self.dnd_dragging_track = None;
            self.dnd_dragging_from_idx = None;
            self.dnd_drop_target_idx = None;
            self.dnd_pointer_offset = egui::Vec2::ZERO;
        }
    }

    fn apply_track_action(&mut self, app: &mut super::app::YadawApp, action: &str, track_id: u64) {
        match action {
            "rename" => {
                let current_name = {
                    let state = app.state.lock().unwrap();
                    state
                        .tracks
                        .get(&track_id)
                        .map(|t| t.name.clone())
                        .unwrap_or_default()
                };
                app.dialogs.show_rename_track(track_id, current_name);
            }
            "duplicate" => app.duplicate_selected_track(),
            "delete" => app.delete_selected_track(),
            _ => {}
        }
    }
}

impl Default for TracksPanel {
    fn default() -> Self {
        Self::new()
    }
}
