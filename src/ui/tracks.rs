use std::collections::HashMap;

use egui::scroll_area::ScrollSource;

use super::*;
use crate::audio_utils::{format_pan, linear_to_db};
use crate::level_meter::LevelMeter;
use crate::messages::{AudioCommand, PluginParamInfo};
use crate::model::PluginDescriptor;
use crate::model::automation::AutomationTarget;
use crate::model::track::TrackType;
use crate::project::ArrangementRow;

use yadaw_plugin_api::{BackendKind, ParamKind};

pub struct TracksPanel {
    track_meters: HashMap<u64, LevelMeter>,
    group_meters: HashMap<u64, LevelMeter>,
    show_mixer_strip: bool,
    show_automation_buttons: bool,
    show_inputs: bool,
    cached_plugin_chains: HashMap<u64, (u64, Vec<PluginDescriptor>)>,

    dnd_dragging: Option<DndPayload>,
    dnd_row_rects: Vec<DndRow>,
    dnd_pointer_offset: egui::Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndPayload {
    Track(u64),
    Group(u64),
}

#[derive(Debug, Clone, Copy)]
pub enum DropTarget {

    InsertAt(usize),

    IntoGroup(u64),
}

struct DndRow {
    id: u64,
    rect: egui::Rect,
    zone: egui::Rect,
    is_group: bool,
}

/// Top 60% of a group header drops into it; below that means "insert here".
fn pointer_in_group_header(rect: &egui::Rect, pointer: egui::Pos2) -> bool {
    let zone = egui::Rect::from_min_max(
        rect.left_top(),
        egui::pos2(rect.right(), rect.top() + rect.height() * 0.6),
    );
    zone.contains(pointer)
}

fn track_order_index_for_insert(
    shown: &[u64],
    target_idx: usize,
    dragged: u64,
    track_order: &[u64],
) -> usize {
    let anchor = shown
        .iter()
        .enumerate()
        .skip(target_idx)
        .map(|(_, &id)| id)
        .find(|&id| id != dragged);

    match anchor {
        Some(id) => track_order
            .iter()
            .position(|&t| t == id)
            .unwrap_or(track_order.len()),
        None => track_order.len(),
    }
}

impl TracksPanel {
    pub fn new() -> Self {
        Self {
            track_meters: HashMap::new(),
            group_meters: HashMap::new(),
            show_mixer_strip: true,
            show_automation_buttons: true,
            show_inputs: true,
            cached_plugin_chains: HashMap::new(),

            dnd_dragging: None,
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

    pub fn update_group_levels(&mut self, levels: HashMap<u64, (f32, f32)>) {
        for (group_id, (left, right)) in levels {
            let meter = self.group_meters.entry(group_id).or_default();
            let samples = [left.max(right)];
            meter.update(&samples, 1.0 / 60.0);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Tracks");
            ui.toggle_value(&mut self.show_mixer_strip, "☰")
                .on_hover_text("Show/Hide Mixer Strip");
            ui.toggle_value(&mut self.show_automation_buttons, "~")
                .on_hover_text("Show/Hide Automation");
            ui.toggle_value(&mut self.show_inputs, "🔣")
                .on_hover_text("Show/Hide Input Options");
        });

        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_track_list(ui, app);
            });

        ui.separator();
        egui::ScrollArea::horizontal()
            .id_salt("tracks_add_row")
            .scroll_source(ScrollSource::ALL)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("+ Audio Track").clicked() {
                        app.add_audio_track();
                    }
                    if ui.button("+ MIDI Track").clicked() {
                        app.add_midi_track();
                    }
                    if ui.button("+ Bus").clicked() {
                        app.add_bus_track();
                    }
                });
            });
    }

    fn draw_track_list(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let mut track_actions = Vec::new();
        let mut automation_actions = Vec::new();
        self.dnd_row_rects.clear();

        // A collapsed group hides only its own subtree, never its siblings.
        let visible_rows: Vec<(ArrangementRow, usize)> = {
            let state = app.state.lock_sync();
            let mut out = Vec::new();
            let mut depth = 0usize;
            let mut hidden_by: Option<u64> = None;
            for row in state.arrangement_rows() {
                match row {
                    ArrangementRow::Group(gid) => {
                        if hidden_by.is_some_and(|h| h == gid) {
                            continue; // this group is itself inside a collapsed one
                        }
                        let nested = hidden_by.is_some();
                        if !nested {
                            hidden_by = None;
                            out.push((ArrangementRow::Group(gid), depth));
                        }
                        if state.groups.get(&gid).is_some_and(|g| g.collapsed) {
                            hidden_by = Some(gid);
                        }
                        depth += 1;
                    }
                    ArrangementRow::Track(tid) => {
                        if hidden_by.is_none() {
                            out.push((ArrangementRow::Track(tid), depth));
                        }
                    }
                }
            }
            out
        };

        for (row, depth) in visible_rows {
            let (id, is_group) = match row {
                ArrangementRow::Group(gid) => (gid, true),
                ArrangementRow::Track(tid) => (tid, false),
            };

            if is_group {
                let (resp, zone) = self.draw_group_header(ui, id, depth, app);
                if resp.clicked() {
                    app.select_group(id);
                }
                self.dnd_row_rects.push(DndRow {
                    id,
                    rect: resp.rect,
                    zone,
                    is_group: true,
                });
                if resp.drag_started() && self.dnd_dragging.is_none() {
                    self.begin_drag(DndPayload::Group(id), &resp);
                }
                continue;
            }

            let track_id = id;
            let is_selected = app.is_track_selected(track_id);

            let header_resp = ui
                .indent(("track_indent", track_id), |ui| {
                    let header_resp =
                        self.draw_track_header(ui, track_id, is_selected, depth, app, |action| {
                            track_actions.push((action, track_id))
                        });

                    if self.show_mixer_strip {
                        self.draw_mixer_strip(ui, track_id, app);
                    }

                    if self.show_automation_buttons
                        && let Some(action) = self.draw_automation_controls(ui, track_id, app)
                    {
                        automation_actions.push(action);
                    }

                    self.draw_plugin_chain(ui, track_id, app);

                    if self.show_inputs {
                        self.draw_io_section(ui, track_id, app);
                    }

                    header_resp
                })
                .inner;

            if header_resp.clicked() {
                app.click_select_track(track_id, ui.input(|i| i.modifiers));
            }

            self.dnd_row_rects.push(DndRow {
                id: track_id,
                rect: header_resp.rect,
                zone: header_resp.rect,
                is_group: false,
            });

            if header_resp.drag_started() && self.dnd_dragging.is_none() {
                self.begin_drag(DndPayload::Track(track_id), &header_resp);
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

    fn begin_drag(&mut self, payload: DndPayload, resp: &egui::Response) {
        self.dnd_dragging = Some(payload);
        self.dnd_pointer_offset = resp
            .interact_pointer_pos()
            .map(|p| p - resp.rect.left_top())
            .unwrap_or(egui::Vec2::ZERO);
    }

    fn draw_track_header<'a>(
        &self,
        ui: &mut egui::Ui,
        track_id: u64,
        is_selected: bool,
        depth: usize,
        app: &super::app::YadawApp,
        mut on_action: impl FnMut(&'a str),
    ) -> egui::Response {
        let (name, is_midi, is_frozen, track_color, group_options) = {
            let state = app.state.lock_sync();
            let track = state.tracks.get(&track_id);

            (
                track
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                track
                    .map(|t| matches!(t.track_type, TrackType::Midi))
                    .unwrap_or(false),
                track.map(|t| t.frozen).unwrap_or(false),
                track.and_then(|t| t.color),
                state
                    .ordered_group_ids()
                    .into_iter()
                    .map(|gid| {
                        let g = &state.groups[&gid];
                        (g.id, g.name.clone())
                    })
                    .collect::<Vec<_>>(),
            )
        };

        // Background tint from track color only (not group)
        let bg_tint =
            track_color.map(|(r, g, b)| egui::Color32::from_rgba_unmultiplied(r, g, b, 30));

        let inner = egui::Frame::group(ui.style())
            .fill(bg_tint.unwrap_or(egui::Color32::TRANSPARENT))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Color strip on left edge (track color only)
                    if let Some((r, g, b)) = track_color {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(4.0, 24.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 0.0, egui::Color32::from_rgb(r, g, b));
                    }

                    // Selected marker
                    if is_selected {
                        ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "⏵");
                    } else {
                        ui.label(" ");
                    }

                    // Tiny intensity viewer
                    if let Some(meter) = self.track_meters.get(&track_id) {
                        ui.scope(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                            ui.add(egui::Separator::default().spacing(4.0));
                            let (resp, painter) =
                                ui.allocate_painter(egui::vec2(60.0, 10.0), egui::Sense::hover());
                            let peak = meter.clone().data.peak_normalized();
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
                    ui.label(if is_midi { "🎹" } else { "♪" });
                    if depth > 0 {
                        ui.label(
                            egui::RichText::new(format!("·{} deep", depth))
                                .small()
                                .weak(),
                        );
                    }

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
                            if ui
                                .button(if is_frozen { "Unfreeze" } else { "Freeze" })
                                .clicked()
                            {
                                on_action("freeze_toggle");
                                ui.close();
                            }

                            ui.separator();

                            // Color picker submenu
                            ui.menu_button("Set Color", |ui| {
                                let current = track_color.unwrap_or((100, 150, 200));
                                if let Some((r, g, b)) = ColorPicker::palette_grid(ui, current) {
                                    let _ = app.command_tx.send(
                                        crate::messages::AudioCommand::SetTrackColor(
                                            track_id, r, g, b,
                                        ),
                                    );
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Clear Color").clicked() {
                                    // TODO: add ClearTrackColor command
                                    ui.close();
                                }
                            });

                            ui.separator();

                            if ui.button("Remove from Group").clicked() {
                                on_action("ungroup_track");
                                ui.close();
                            }
                            ui.menu_button("Add to Group", |ui| {
                                for (gid, gname) in &group_options {
                                    if ui.button(gname).clicked() {
                                        on_action("group_new_here");
                                        let _ = app.command_tx.send(AudioCommand::AddTrackToGroup(
                                            track_id, *gid,
                                        ));
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui.button("New Group from Selection").clicked() {
                                    on_action("group_new_here");
                                    ui.close();
                                }
                            });
                        });
                    });
                });
            });

        let rect = inner.response.rect;
        let id = ui.id().with(("track_header", track_id));

        let reserved_w = 48.0;
        let drag_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2((rect.right() - reserved_w).max(rect.left()), rect.bottom()),
        );

        let drag_resp = ui.interact(drag_rect, id, egui::Sense::click_and_drag());

        if is_selected {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                3.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 255)),
                egui::StrokeKind::Middle,
            );
        }

        drag_resp.union(inner.response)
    }

    fn draw_group_header(
        &mut self,
        ui: &mut egui::Ui,
        group_id: u64,
        depth: usize,
        app: &mut super::app::YadawApp,
    ) -> (egui::Response, egui::Rect) {
        let (name, color, collapsed, volume, muted, solo, member_count, child_count) = {
            let state = app.state.lock_sync();
            let g = state.groups.get(&group_id);
            (
                g.map(|g| g.name.clone()).unwrap_or_default(),
                g.map(|g| g.color).unwrap_or((120, 120, 120)),
                g.is_some_and(|g| g.collapsed),
                g.map(|g| g.volume).unwrap_or(1.0),
                g.is_some_and(|g| g.muted),
                g.is_some_and(|g| g.solo),
                state.get_group_subtree_tracks(group_id).len(),
                state.child_groups(group_id).len(),
            )
        };

        let fill = egui::Color32::from_rgba_unmultiplied(color.0, color.1, color.2, 38);
        let is_selected = app.selected_group == Some(group_id);

        let mut header_rect = egui::Rect::NOTHING;
        let frame = egui::Frame::group(ui.style())
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                header_rect = ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 12.0);

                    if ui
                        .small_button(if collapsed { "▶" } else { "▼" })
                        .on_hover_text(if collapsed { "Expand" } else { "Collapse" })
                        .clicked()
                    {
                        let _ = app.command_tx.send(AudioCommand::SetGroupCollapsed(
                            group_id,
                            !collapsed,
                        ));
                    }

                    let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 16.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::from_rgb(color.0, color.1, color.2));

                    if is_selected {
                        ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "⏵");
                    }

                    if ui
                        .selectable_label(is_selected, egui::RichText::new(&name).strong())
                        .clicked()
                    {
                        app.select_group(group_id);
                    }
                    ui.weak(format!(
                        "{member_count} track{}",
                        if member_count == 1 { "" } else { "s" }
                    ));
                    if child_count > 0 {
                        ui.weak(format!("· {child_count} subgroup(s)"));
                    }

                    if let Some(meter) = self.group_meters.get(&group_id) {
                        let (resp, painter) =
                            ui.allocate_painter(egui::vec2(60.0, 10.0), egui::Sense::hover());
                        let peak = meter.clone().data.peak_normalized();
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
                    }
                }).response.rect;

                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("⚙", |ui| {
                            if ui.button("Select All Tracks").clicked() {
                                let _ = app.command_tx.send(AudioCommand::UpdateTracks);
                                app.select_group_tracks(group_id);
                                ui.close();
                            }
                            if ui.button("Ungroup All Tracks").clicked() {
                                let tracks = {
                                    let st = app.state.lock_sync();
                                    st.get_group_subtree_tracks(group_id)
                                };
                                for tid in tracks {
                                    let _ = app
                                        .command_tx
                                        .send(AudioCommand::RemoveTrackFromGroup(tid));
                                }
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("New Subgroup…").clicked() {
                                app.create_group_named(
                                    "New Group".to_string(),
                                    Vec::new(),
                                    Some(group_id),
                                );
                                ui.close();
                            }
                            ui.separator();
                            ui.menu_button("Set Color", |ui| {
                                if let Some((r, g, b)) =
                                    super::color_picker::ColorPicker::palette_grid(ui, color)
                                {
                                    let _ = app
                                        .command_tx
                                        .send(AudioCommand::SetGroupColor(group_id, r, g, b));
                                    ui.close();
                                }
                            });
                            ui.separator();
                            if ui.button("Delete Group").clicked() {
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::RemoveGroup(group_id));
                                ui.close();
                            }
                        });

                        if ui
                            .selectable_label(solo, if solo { "S" } else { "s" })
                            .on_hover_text("Solo group")
                            .clicked()
                        {
                            let _ = app.command_tx.send(AudioCommand::SetGroupSolo(
                                group_id, !solo,
                            ));
                        }
                        if ui
                            .selectable_label(muted, if muted { "M" } else { "m" })
                            .on_hover_text("Mute group")
                            .clicked()
                        {
                            let _ = app
                                .command_tx
                                .send(AudioCommand::SetGroupMute(group_id, !muted));
                        }

                        let mut vol = volume;
                        let slider_w = (ui.available_width() - 44.0).max(40.0);
                        let changed = ui
                            .add_sized(
                                [slider_w, 16.0],
                                egui::Slider::new(&mut vol, 0.0..=1.2).show_value(false),
                            )
                            .changed();
                        if changed {
                            let _ = app
                                .command_tx
                                .send(AudioCommand::SetGroupVolume(group_id, vol));
                        }
                        ui.label(format!("{:.1}", linear_to_db(vol)));
                    });
                });
            });

        let rect = frame.response.rect;
        let id = ui.id().with(("group_header", group_id));
        let header_zone = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.right(), header_rect.bottom().max(rect.min.y)),
        );
        let drag_resp = ui.interact(header_zone, id, egui::Sense::click_and_drag());
        (drag_resp.union(frame.response), header_zone)
    }

    fn draw_mixer_strip(&mut self, ui: &mut egui::Ui, track_id: u64, app: &super::app::YadawApp) {
        let (mut volume, mut pan, muted, solo, armed, monitor_enabled, is_midi) = {
            let state = app.state.lock_sync();
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
                        matches!(t.track_type, TrackType::Midi),
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
                .selectable_label(armed, if armed { "■" } else { "○" })
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
            let state = app.state.lock_sync();
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
            let state = app.state.lock_sync();
            state
                .tracks
                .get(&track_id)
                .map(|t| t.plugin_chain.len())
                .unwrap_or(0)
        };

        // Only lock when we need to read plugin data
        for plugin_idx in 0..chain_len {
            let (plugin_id, plugin_name, plugin_uri, backend, bypass, has_editor, params) = {
                let state = app.state.lock_sync();
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
                    plugin.backend,
                    plugin.bypass,
                    plugin.has_editor,
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
                        if ui.small_button("⊗").clicked() {
                            plugin_to_remove = Some(plugin_id);
                        }
                        if plugin_idx > 0 && ui.small_button("⏶").clicked() {
                            move_action = Some((plugin_idx, plugin_idx - 1));
                        }
                        if plugin_idx < chain_len - 1 && ui.small_button("⏷").clicked() {
                            move_action = Some((plugin_idx, plugin_idx + 1));
                        }
                        #[cfg(not(target_os = "android"))]
                        if has_editor && ui.button("Open Editor").clicked() {
                            app.open_plugin_editor(track_id, plugin_id);
                        }
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.menu_button("Presets 📁", |ui| {
                            if ui.button("Save Snapshot").clicked() {
                                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                                let preset_name = format!("Snapshot_{}", ts);
                                let _ = app.command_tx.send(AudioCommand::SavePluginPreset(
                                    track_id,
                                    plugin_idx,
                                    preset_name,
                                ));
                                ui.close();
                            }

                            #[cfg(all(not(target_os = "android"), not(target_arch = "wasm32")))]
                            {
                                if ui.button("Open Presets Folder").clicked() {
                                    let uri_dir = crate::paths::presets_dir().join(
                                        plugin_uri
                                            .chars()
                                            .map(
                                                |c| if c.is_ascii_alphanumeric() { c } else { '_' },
                                            )
                                            .collect::<String>(),
                                    );

                                    if let Err(e) = std::fs::create_dir_all(&uri_dir) {
                                        app.dialogs.show_message(&format!(
                                            "Failed to create presets folder: {e}"
                                        ));
                                    } else if let Err(e) =
                                        crate::paths::open_path_in_file_manager(&uri_dir)
                                    {
                                        app.dialogs.show_message(&format!(
                                            "Failed to open presets folder: {e}"
                                        ));
                                    }

                                    ui.close();
                                }
                            }

                            let presets = crate::presets::list_presets_for(&plugin_uri);
                            if presets.is_empty() {
                                ui.label(egui::RichText::new("(no presets)").weak());
                            } else {
                                ui.separator();
                                for pname in presets {
                                    if ui.button(&pname).clicked() {
                                        let _ =
                                            app.command_tx.send(AudioCommand::LoadPluginPreset(
                                                track_id, plugin_idx, pname,
                                            ));
                                        ui.close();
                                    }
                                }
                            }
                        });
                    });

                    // Draw parameters based on backend
                    match backend {
                        BackendKind::Lv2 | BackendKind::Clap | BackendKind::Vst3 => self
                            .draw_plugin_params(ui, app, track_id, plugin_id, plugin_idx, &params),
                    }
                });
        }

        if let Some(id_to_remove) = plugin_to_remove {
            let _ = app
                .command_tx
                .send(AudioCommand::RemovePlugin(track_id, id_to_remove));
            self.cached_plugin_chains.remove(&track_id);

            app.invalidate_clap_params_for_track(track_id);
            let _ = app.command_tx.send(AudioCommand::RebuildAllRtChains);
        }

        if let Some((from, to)) = move_action {
            let _ = app
                .command_tx
                .send(AudioCommand::MovePlugin(track_id, from, to));
            self.cached_plugin_chains.remove(&track_id);

            app.invalidate_clap_params_for_track(track_id);
            let _ = app.command_tx.send(AudioCommand::RebuildAllRtChains);
        }
    }

    fn draw_plugin_params(
        &self,
        ui: &mut egui::Ui,
        app: &super::app::YadawApp,
        track_id: u64,
        plugin_id: u64,
        plugin_idx: usize,
        params: &HashMap<String, f32>,
    ) {
        if let Some(meta_list) = app.clap_param_meta.get(&(track_id, plugin_idx)) {
            let mut meta: Vec<PluginParamInfo> = meta_list
                .iter()
                .filter(|p| !p.is_hidden) // Skip hidden params
                .cloned()
                .collect();

            meta.sort_by(|a, b| {
                let ga = a.group.as_deref().unwrap_or("");
                let gb = b.group.as_deref().unwrap_or("");
                match ga.cmp(gb) {
                    std::cmp::Ordering::Equal => a.name.cmp(&b.name),
                    other => other,
                }
            });

            let draw_param = |ui: &mut egui::Ui,
                              pinfo: &PluginParamInfo,
                              params: &HashMap<String, f32>,
                              track_id: u64,
                              plugin_id: u64,
                              app: &super::app::YadawApp| {
                let mut v = params.get(&pinfo.name).copied().unwrap_or(pinfo.current);

                let is_readonly = pinfo.is_readonly;

                ui.horizontal(|ui| {
                    ui.label(&pinfo.name);

                    let changed = match pinfo.kind {
                        ParamKind::Bool => {
                            let mut bool_val = v > 0.5;
                            let resp = ui
                                .add_enabled(!is_readonly, egui::Checkbox::new(&mut bool_val, ""));
                            if resp.changed() {
                                v = if bool_val { 1.0 } else { 0.0 };
                            }
                            resp.changed()
                        }
                        ParamKind::Enum => {
                            if let Some(labels) = &pinfo.enum_labels {
                                let steps = labels.len() as i32 - 1;
                                let idx = ((v - pinfo.min).round() as i32).clamp(0, steps) as usize;

                                let current_label = labels
                                    .get(idx)
                                    .cloned()
                                    .unwrap_or_else(|| format!("{}", idx));

                                let mut new_idx = idx;
                                let mut changed = false; // Move outside

                                egui::ComboBox::from_id_salt((&pinfo.name, track_id, plugin_id))
                                    .selected_text(current_label)
                                    .width(160.0)
                                    .show_ui(ui, |ui| {
                                        egui::ScrollArea::vertical().max_height(300.0).show(
                                            ui,
                                            |ui| {
                                                ui.add_enabled_ui(!is_readonly, |ui| {
                                                    for (i, label) in labels.iter().enumerate() {
                                                        if ui
                                                            .selectable_value(
                                                                &mut new_idx,
                                                                i,
                                                                label,
                                                            )
                                                            .changed()
                                                        {
                                                            changed = true;
                                                        }
                                                    }
                                                });
                                            },
                                        );
                                    });

                                if changed {
                                    v = pinfo.min + new_idx as f32;
                                }
                                changed
                            } else {
                                // Fallback to Int slider if somehow enum_labels are missing
                                let mut int_val = v.round() as i32;
                                let min_i = pinfo.min.round() as i32;
                                let max_i = pinfo.max.round() as i32;
                                let resp = ui.add_enabled(
                                    !is_readonly,
                                    egui::Slider::new(&mut int_val, min_i..=max_i)
                                        .step_by(1.0)
                                        .show_value(true),
                                );
                                if resp.changed() {
                                    v = int_val as f32;
                                }
                                resp.changed()
                            }
                        }
                        ParamKind::Int => {
                            let mut int_val = v.round() as i32;
                            let min_i = pinfo.min.round() as i32;
                            let max_i = pinfo.max.round() as i32;
                            let resp = ui.add_enabled(
                                !is_readonly,
                                egui::Slider::new(&mut int_val, min_i..=max_i)
                                    .step_by(1.0)
                                    .show_value(true),
                            );
                            if resp.changed() {
                                v = int_val as f32;
                            }
                            resp.changed()
                        }
                        ParamKind::Float => {
                            let resp = ui.add_enabled(
                                !is_readonly,
                                egui::Slider::new(&mut v, pinfo.min..=pinfo.max).show_value(false), // We'll show formatted value
                            );

                            if let Some(ref unit) = pinfo.unit {
                                ui.label(format!("{:.2}{}", v, unit));
                            } else {
                                ui.label(format!("{:.2}", v));
                            }

                            resp.changed()
                        }
                    };

                    if changed && !is_readonly {
                        let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                            track_id,
                            plugin_id,
                            pinfo.name.clone(),
                            v,
                        ));

                        let mut state = app.state.lock_sync();
                        if let Some(track) = state.tracks.get_mut(&track_id) {
                            if let Some(plugin) =
                                track.plugin_chain.iter_mut().find(|p| p.id == plugin_id)
                            {
                                plugin.params.insert(pinfo.name.clone(), v);
                            }
                        }
                    }

                    // Reset button - only if not readonly
                    if !is_readonly {
                        let default_text = if let Some(ref unit) = pinfo.unit {
                            format!("{:.2}{}", pinfo.default, unit)
                        } else {
                            format!("{:.3}", pinfo.default)
                        };

                        if ui
                            .small_button("↺")
                            .on_hover_text(format!("Reset to {}", default_text))
                            .clicked()
                        {
                            let _ = app.command_tx.send(AudioCommand::SetPluginParam(
                                track_id,
                                plugin_id,
                                pinfo.name.clone(),
                                pinfo.default,
                            ));
                        }
                    }

                    if pinfo.is_automatable {
                        if ui
                            .weak("⚡")
                            .on_hover_text("Insert automation keyframe at current position")
                            .clicked()
                        {
                            let position = app.audio_state.get_position();
                            let current_beat = app.state.lock_sync().position_to_beats(position);
                            let target = AutomationTarget::PluginParam {
                                plugin_id,
                                param_name: pinfo.name.clone(),
                            };
                            let _ = app.command_tx.send(AudioCommand::AddAutomationPoint(
                                track_id,
                                target,
                                current_beat,
                                v,
                            ));
                        }
                    }
                });
            };

            // Walk meta grouped by `group`
            let mut i = 0;
            while i < meta.len() {
                let group = meta[i].group.clone();
                if let Some(ref grp) = group {
                    // contiguous run of same group
                    let start = i;
                    while i < meta.len() && meta[i].group.as_deref() == Some(grp.as_str()) {
                        i += 1;
                    }

                    egui::CollapsingHeader::new(grp)
                        .id_salt((grp.as_str(), track_id, plugin_id))
                        .show(ui, |ui| {
                            for pinfo in &meta[start..i] {
                                draw_param(ui, pinfo, params, track_id, plugin_id, app);
                            }
                        });
                } else {
                    // ungrouped, draw directly
                    let pinfo = &meta[i];
                    draw_param(ui, pinfo, params, track_id, plugin_id, app);
                    i += 1;
                }
            }
        } else {
            ui.label(egui::RichText::new("No parameter info available").weak());
        }
    }

    fn draw_io_section(&self, ui: &mut egui::Ui, track_id: u64, app: &mut super::app::YadawApp) {
        let track = {
            let state = app.state.lock_sync();
            state.tracks.get(&track_id).cloned()
        };

        if let Some(track) = track {
            if matches!(track.track_type, TrackType::Midi) {
                ui.horizontal(|ui| {
                    ui.label("MIDI In:");
                    let mut selected_port = track
                        .midi_input_port
                        .clone()
                        .unwrap_or_else(|| "None".to_string());

                    let changed = egui::ComboBox::from_id_salt(("midi_in", track_id))
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
                        })
                        .inner
                        .unwrap_or(false);

                    if changed {
                        let new_sel = if selected_port == "None" {
                            None
                        } else {
                            Some(selected_port)
                        };
                        let _ = app
                            .command_tx
                            .send(AudioCommand::SetTrackMidiInput(track_id, new_sel));
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label("Audio In:");
                    let mut in_sel = track
                        .input_device
                        .clone()
                        .unwrap_or_else(|| "Default".to_string());
                    let changed = egui::ComboBox::from_id_salt(("audio_in", track_id))
                        .selected_text(&in_sel)
                        .show_ui(ui, |ui| {
                            let mut ch = ui
                                .selectable_value(&mut in_sel, "Default".to_string(), "Default")
                                .changed();
                            ch |= ui
                                .selectable_value(&mut in_sel, "None".to_string(), "None")
                                .changed();
                            ch
                        })
                        .inner
                        .unwrap_or(false);
                    if changed {
                        let new_sel = if in_sel == "None" { None } else { Some(in_sel) };
                        let _ = app
                            .command_tx
                            .send(AudioCommand::SetTrackInput(track_id, new_sel));
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Audio Out:");
                    let mut out_sel = track
                        .output_device
                        .clone()
                        .unwrap_or_else(|| "Default".to_string());
                    let changed = egui::ComboBox::from_id_salt(("audio_out", track_id))
                        .selected_text(&out_sel)
                        .show_ui(ui, |ui| {
                            let mut ch = ui
                                .selectable_value(&mut out_sel, "Default".to_string(), "Default")
                                .changed();
                            ch |= ui
                                .selectable_value(&mut out_sel, "None".to_string(), "None")
                                .changed();
                            ch
                        })
                        .inner
                        .unwrap_or(false);
                    if changed {
                        let new_sel = if out_sel == "None" {
                            None
                        } else {
                            Some(out_sel)
                        };
                        let _ = app
                            .command_tx
                            .send(AudioCommand::SetTrackOutput(track_id, new_sel));
                    }
                });
            }
        }
    }

    fn handle_track_dnd(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        let Some(payload) = self.dnd_dragging else {
            return;
        };

        // Compute drop target index using pointer Y against row centers
        let pointer = match ui.ctx().input(|i| i.pointer.interact_pos()) {
            Some(p) => p,
            None => return,
        };

        let target = match self
            .dnd_row_rects
            .iter()
            .find(|r| r.is_group && pointer_in_group_header(&r.zone, pointer))
            .map(|r| r.id)
        {
            Some(gid) => DropTarget::IntoGroup(gid),
            None => DropTarget::InsertAt(self.insert_index_at(pointer)),
        };

        // Paint insertion line and ghost
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tracks_dnd_layer"));
        let painter = ui.ctx().layer_painter(layer);

        // draw insertion line spanning header width
        if let Some(first) = self.dnd_row_rects.first().map(|r| r.rect) {
            match target {
                DropTarget::IntoGroup(gid) => {
                    if let Some(row) = self.dnd_row_rects.iter().find(|r| r.id == gid) {
                        painter.rect_stroke(
                            row.rect,
                            3.0,
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                DropTarget::InsertAt(idx) => {
                    let y = if idx >= self.dnd_row_rects.len() {
                        self.dnd_row_rects
                            .last()
                            .map_or(first.bottom(), |r| r.rect.bottom())
                    } else {
                        self.dnd_row_rects[idx].rect.top()
                    };
                    painter.line_segment(
                        [egui::pos2(first.left(), y), egui::pos2(first.right(), y)],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
                    );
                }
            }
        }

        let drag_id = match payload {
            DndPayload::Track(id) | DndPayload::Group(id) => id,
        };
        if let Some(src_rect) = self.dnd_row_rects.iter().find(|r| r.id == drag_id).map(|r| r.rect)
        {
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

        let released = ui.ctx().input(|i| i.pointer.any_released());
        if released {
            match target {
                DropTarget::IntoGroup(gid) => {
                    if !matches!(payload, DndPayload::Group(g) if g == gid) {
                        let moving = self.moving_track_ids(app, payload);
                        if !moving.is_empty() {
                            app.push_undo();
                            for tid in moving {
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::AddTrackToGroup(tid, gid));
                            }
                        }
                    }
                }
                DropTarget::InsertAt(idx) => self.commit_reorder(app, payload, idx),
            }

            self.dnd_dragging = None;
            self.dnd_pointer_offset = egui::Vec2::ZERO;
        }
    }

    fn insert_index_at(&self, pointer: egui::Pos2) -> usize {
        for (i, row) in self.dnd_row_rects.iter().enumerate() {
            if pointer.y < row.rect.center().y {
                return i;
            }
        }
        self.dnd_row_rects.len()
    }

    fn moving_track_ids(&self, app: &super::app::YadawApp, payload: DndPayload) -> Vec<u64> {
        let st = app.state.lock_sync();
        match payload {
            DndPayload::Track(tid) => vec![tid],
            DndPayload::Group(gid) => st.get_group_subtree_tracks(gid),
        }
    }

    /// Group order goes through a command so undo and the audio see it.
    fn commit_reorder(
        &self,
        app: &mut super::app::YadawApp,
        payload: DndPayload,
        target_idx: usize,
    ) {
        match payload {
            DndPayload::Track(tid) => {
                let (from, to) = {
                    let st = app.state.lock_sync();
                    let Some(from) = st.track_order.iter().position(|&id| id == tid) else {
                        return;
                    };
                    // Resolve against the drawn order, then map to flat order.
                    let shown: Vec<u64> = st
                        .arrangement_rows()
                        .into_iter()
                        .filter_map(|r| match r {
                            ArrangementRow::Track(id) => Some(id),
                            ArrangementRow::Group(_) => None,
                        })
                        .collect();
                    let to = track_order_index_for_insert(&shown, target_idx, tid, &st.track_order);
                    (from, to)
                };
                if to == from || to >= app.state.lock_sync().track_order.len() {
                    return;
                }
                app.push_undo();
                let mut st = app.state.lock_sync();
                crate::track_manager::move_track(&mut st.track_order, from, to);
                drop(st);
                let _ = app.command_tx.send(AudioCommand::UpdateTracks);
            }
            DndPayload::Group(gid) => {
                let to = {
                    let st = app.state.lock_sync();
                    let Some(from) = st.group_order.iter().position(|&id| id == gid) else {
                        return;
                    };
                    // Just above the group under the pointer, in post-removal slots.
                    let anchor = self
                        .dnd_row_rects
                        .get(target_idx)
                        .and_then(|r| st.group_order.iter().position(|&g| g == r.id));
                    let to = match anchor {
                        Some(pos) if pos > from => pos - 1,
                        Some(pos) => pos,
                        None => st.group_order.len().saturating_sub(1),
                    };
                    if to == from {
                        return;
                    }
                    to
                };
                app.push_undo();
                let _ = app
                    .command_tx
                    .send(AudioCommand::MoveGroupInOrder(gid, to));
            }
        }
    }

    fn apply_track_action(&mut self, app: &mut super::app::YadawApp, action: &str, track_id: u64) {
        match action {
            "ungroup_track" => {
                app.push_undo();
                let _ = app
                    .command_tx
                    .send(AudioCommand::RemoveTrackFromGroup(track_id));
            }
            "group_new_here" => {
                app.create_group_from_selection();
            }
            "rename" => {
                let current_name = {
                    let state = app.state.lock_sync();
                    state
                        .tracks
                        .get(&track_id)
                        .map(|t| t.name.clone())
                        .unwrap_or_default()
                };
                app.dialogs.show_rename_track(track_id, current_name);
            }
            "duplicate" => {
                app.push_undo();
                let new_id_opt = {
                    let mut state = app.state.lock_sync();
                    if let Some(src) = state.tracks.get(&track_id).cloned() {
                        let mut resolved = src;
                        for mc in &mut resolved.midi_clips {
                            *mc = state.materialize_midi_clip(mc);
                        }
                        // Deep-copy plugins with fresh ids
                        let mut new_track = app.track_manager.duplicate_track(&resolved);
                        let new_id = state.fresh_id();
                        new_track.id = new_id;

                        let insert_pos = state
                            .track_order
                            .iter()
                            .position(|&id| id == track_id)
                            .map(|i| i + 1)
                            .unwrap_or(state.track_order.len());

                        state.track_order.insert(insert_pos, new_id);
                        state.tracks.insert(new_id, new_track);
                        state.ensure_ids(); // moves notes into fresh patterns + rebuilds clips_by_id
                        Some(new_id)
                    } else {
                        None
                    }
                };

                if let Some(new_id) = new_id_opt {
                    app.select_track(new_id);
                    let _ = app.command_tx.send(AudioCommand::UpdateTracks);
                    let _ = app.command_tx.send(AudioCommand::RebuildAllRtChains);
                }
            }
            "delete" => {
                let can_delete = {
                    let st = app.state.lock_sync();
                    st.track_order.len() > 1
                };
                if !can_delete {
                    app.dialogs.show_message("Cannot delete the last track");
                    return;
                }

                let new_selected = {
                    let mut st = app.state.lock_sync();
                    if let Some(pos) = st.track_order.iter().position(|&id| id == track_id) {
                        st.track_order.remove(pos);
                        st.tracks.remove(&track_id);
                        st.clips_by_id.retain(|_, r| r.track_id != track_id);

                        if pos > 0 {
                            st.track_order.get(pos - 1).copied()
                        } else {
                            st.track_order.first().copied()
                        }
                    } else {
                        None
                    }
                };

                if let Some(ns) = new_selected {
                    app.select_track(ns);
                }
                let _ = app.command_tx.send(AudioCommand::UpdateTracks);
            }
            "freeze_toggle" => {
                let is_frozen = {
                    let state = app.state.lock_sync();
                    state
                        .tracks
                        .get(&track_id)
                        .map(|t| t.frozen)
                        .unwrap_or(false)
                };
                let cmd = if is_frozen {
                    AudioCommand::UnfreezeTrack(track_id)
                } else {
                    AudioCommand::FreezeTrack(track_id)
                };
                let _ = app.command_tx.send(cmd);
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
