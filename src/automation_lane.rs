use crate::state::{AutomationLane, AutomationTarget, OrderedFloat};
use eframe::egui;

pub struct AutomationLaneWidget {
    selected_points: Vec<f64>,
    dragging_point: Option<(f64, f32)>, // beat, original_value
}

impl Default for AutomationLaneWidget {
    fn default() -> Self {
        Self {
            selected_points: Vec::new(),
            dragging_point: None,
        }
    }
}

impl AutomationLaneWidget {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        lane: &mut AutomationLane,
        rect: egui::Rect,
        zoom_x: f32,
        scroll_x: f32,
    ) -> Vec<AutomationAction> {
        let mut actions = Vec::new();

        // Draw background
        ui.painter()
            .rect_filled(rect, 2.0, egui::Color32::from_gray(25));

        // Draw automation curve
        if lane.points.len() >= 2 {
            let points: Vec<egui::Pos2> = lane
                .points
                .iter()
                .map(|(beat, value)| {
                    let x = rect.left() + (beat.0 as f32 * zoom_x - scroll_x);
                    let y = rect.bottom() - (*value * rect.height());
                    egui::pos2(x, y)
                })
                .collect();

            // Draw lines between points
            for window in points.windows(2) {
                ui.painter().line_segment(
                    [window[0], window[1]],
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 150, 255)),
                );
            }
        }

        // Draw control points
        for (beat, value) in &lane.points {
            let x = rect.left() + (beat.0 as f32 * zoom_x - scroll_x);
            let y = rect.bottom() - (value * rect.height());
            let point_pos = egui::pos2(x, y);

            let point_rect = egui::Rect::from_center_size(point_pos, egui::vec2(8.0, 8.0));

            let is_selected = self.selected_points.contains(&beat.0);

            ui.painter().circle_filled(
                point_pos,
                4.0,
                if is_selected {
                    egui::Color32::from_rgb(255, 200, 100)
                } else {
                    egui::Color32::from_rgb(150, 180, 255)
                },
            );

            // Handle interaction
            let response = ui.interact(
                point_rect,
                ui.id().with(("automation_point", beat.0.to_bits())),
                egui::Sense::click_and_drag(),
            );

            if response.clicked() {
                if ui.input(|i| i.modifiers.ctrl) {
                    if is_selected {
                        self.selected_points.retain(|&b| b != beat.0);
                    } else {
                        self.selected_points.push(beat.0);
                    }
                } else {
                    self.selected_points.clear();
                    self.selected_points.push(beat.0);
                }
            }

            if response.drag_started() {
                self.dragging_point = Some((beat.0, *value));
            }

            if response.dragged() {
                if let Some((original_beat, _)) = self.dragging_point {
                    if let Some(pos) = response.hover_pos() {
                        let new_beat = ((pos.x - rect.left() + scroll_x) / zoom_x) as f64;
                        let new_value = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);

                        actions.push(AutomationAction::MovePoint {
                            old_beat: original_beat,
                            new_beat,
                            new_value,
                        });
                    }
                }
            }

            if response.drag_stopped() {
                self.dragging_point = None;
            }

            // Right-click to delete
            if response.secondary_clicked() {
                actions.push(AutomationAction::RemovePoint(beat.0));
            }
        }

        // Double-click to add point
        let area_response =
            ui.interact(rect, ui.id().with("automation_area"), egui::Sense::click());

        if area_response.double_clicked() {
            if let Some(pos) = area_response.interact_pointer_pos() {
                let beat = ((pos.x - rect.left() + scroll_x) / zoom_x) as f64;
                let value = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
                actions.push(AutomationAction::AddPoint { beat, value });
            }
        }

        actions
    }

    pub fn get_value_at_beat(lane: &AutomationLane, beat: f64) -> f32 {
        if lane.points.is_empty() {
            return 0.5; // Default middle value
        }

        let ordered_beat = OrderedFloat(beat);

        // Find surrounding points for interpolation
        let before = lane.points.range(..=ordered_beat).last();
        let after = lane.points.range(ordered_beat..).next();

        match (before, after) {
            (Some((b1, v1)), Some((b2, v2))) if b1.0 != b2.0 => {
                // Linear interpolation
                let t = (beat - b1.0) / (b2.0 - b1.0);
                v1 + (v2 - v1) * t as f32
            }
            (Some((_, v)), _) | (_, Some((_, v))) => *v,
            _ => 0.5,
        }
    }
}

#[derive(Debug)]
pub enum AutomationAction {
    AddPoint {
        beat: f64,
        value: f32,
    },
    RemovePoint(f64),
    MovePoint {
        old_beat: f64,
        new_beat: f64,
        new_value: f32,
    },
}
