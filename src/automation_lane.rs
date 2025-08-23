use crate::audio_state::AutomationTarget;
use egui;

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
        _ui: &mut egui::Ui,
        _lane: &mut crate::state::AutomationLane,
        _lane_rect: egui::Rect,
        _zoom_x: f32,
        _scroll_x: f32,
    ) -> Vec<AutomationAction> {
        Vec::new()
    }

    // Linear value evaluation for state lanes (used by audio engine)
    pub fn get_value_at_beat(lane: &crate::state::AutomationLane, beat: f64) -> f32 {
        if lane.points.is_empty() {
            return 0.0;
        }
        let mut prev = &lane.points[0];
        let mut next = &lane.points[lane.points.len() - 1];

        for p in &lane.points {
            if p.beat <= beat {
                prev = p;
            } else {
                next = p;
                break;
            }
        }

        if (next.beat - prev.beat).abs() < f64::EPSILON {
            return next.value;
        }

        let t = ((beat - prev.beat) / (next.beat - prev.beat)).clamp(0.0, 1.0);
        prev.value + (next.value - prev.value) * t as f32
    }
}
