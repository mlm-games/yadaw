use eframe::egui;

use crate::model::automation::AutomationLane;

#[derive(Debug, Clone)]
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

#[derive(Default, Clone)]
pub struct AutomationLaneWidget;

impl AutomationLaneWidget {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        lane: &AutomationLane,
        lane_rect: egui::Rect,
        zoom_x: f32,
        scroll_x: f32,
    ) -> Vec<AutomationAction> {
        let mut actions = Vec::new();
        let painter = ui.painter_at(lane_rect);

        // Background
        let bg = egui::Color32::from_gray(22);
        painter.rect_filled(lane_rect, 0.0, bg);

        // Midline
        let mid_y = egui::lerp(lane_rect.bottom_up_range(), 0.5);
        painter.line_segment(
            [
                egui::pos2(lane_rect.left(), mid_y),
                egui::pos2(lane_rect.right(), mid_y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        );

        // Color for lane curve/points
        let lane_color = lane
            .color
            .map(|(r, g, b)| egui::Color32::from_rgb(r, g, b))
            .unwrap_or(egui::Color32::from_rgb(120, 170, 255));

        // Build screen positions for points (only those visible)
        let mut pts_screen: Vec<(usize, egui::Pos2)> = Vec::with_capacity(lane.points.len());
        for (i, p) in lane.points.iter().enumerate() {
            let x = lane_rect.left() + (p.beat as f32 * zoom_x - scroll_x);
            if x < lane_rect.left() - 8.0 || x > lane_rect.right() + 8.0 {
                continue;
            }
            let y = lane_rect.bottom() - (p.value.clamp(0.0, 1.0) * lane_rect.height());
            pts_screen.push((i, egui::pos2(x, y)));
        }

        // Curve (polyline)
        if pts_screen.len() >= 2 {
            painter.add(egui::Shape::line(
                pts_screen.iter().map(|(_, p)| *p).collect(),
                egui::Stroke::new(1.5, lane_color),
            ));
        }

        // Point handles
        let handle_r = 5.0;
        let id_base = ui.id().with(("auto_pts", lane as *const _ as usize));
        let mut hovered_any = false;

        for (i, pos) in pts_screen.iter().cloned() {
            let handle_rect = egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0));
            let id = id_base.with(i);
            let resp = ui.interact(handle_rect, id, egui::Sense::click_and_drag());

            hovered_any |= resp.hovered() || resp.dragged();

            // Draw
            let fill = if resp.hovered() || resp.dragged() {
                egui::Color32::from_rgb(
                    (lane_color.r() as f32 * 1.2).min(255.0) as u8,
                    (lane_color.g() as f32 * 1.2).min(255.0) as u8,
                    (lane_color.b() as f32 * 1.2).min(255.0) as u8,
                )
            } else {
                lane_color
            };
            painter.circle_filled(pos, handle_r, fill);
            painter.circle_stroke(pos, handle_r, egui::Stroke::new(1.0, egui::Color32::BLACK));

            // Drag to move
            if resp.dragged()
                && let Some(pointer) = resp.interact_pointer_pos()
            {
                let beat = ((pointer.x - lane_rect.left()) + scroll_x) / zoom_x;
                let value = ((lane_rect.bottom() - pointer.y) / lane_rect.height()).clamp(0.0, 1.0);

                let old_beat = lane.points[i].beat;
                actions.push(AutomationAction::MovePoint {
                    old_beat,
                    new_beat: beat as f64,
                    new_value: value,
                });
            }

            // Right-click to remove
            if resp.secondary_clicked() {
                actions.push(AutomationAction::RemovePoint(lane.points[i].beat));
            }
        }

        // Click empty space to add
        let lane_resp = ui.interact(
            lane_rect,
            ui.id().with(("auto_lane_bg", lane as *const _ as usize)),
            egui::Sense::click(),
        );
        if lane_resp.clicked()
            && !hovered_any
            && let Some(pos) = lane_resp.interact_pointer_pos()
        {
            let beat = ((pos.x - lane_rect.left()) + scroll_x) / zoom_x;
            let value = ((lane_rect.bottom() - pos.y) / lane_rect.height()).clamp(0.0, 1.0);
            actions.push(AutomationAction::AddPoint {
                beat: beat as f64,
                value,
            });
        }

        actions
    }
}
