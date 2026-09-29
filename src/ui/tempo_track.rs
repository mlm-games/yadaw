use eframe::egui;

use crate::model::tempo::{TempoCurve, TempoPoint, TempoRamp};
use crate::project::AppState;

const MIN_BPM: f64 = 20.0;
const MAX_BPM: f64 = 300.0;
const LANE_HEIGHT: f32 = 74.0;
const HANDLE: f32 = 13.0;
const MIN_GAP: f64 = 1.0e-6;
const DEFAULT_SPAN: f64 = 16.0;

#[derive(Debug, Clone, PartialEq)]
pub enum TempoAction {
    Set { points: Vec<TempoPoint>, undo: bool },
}

pub struct TempoTrackUI {
    selected: Option<usize>,
    open: bool,
    span: f64,
    undo_armed: bool,
}

impl Default for TempoTrackUI {
    fn default() -> Self {
        Self {
            selected: None,
            open: false,
            span: DEFAULT_SPAN,
            undo_armed: false,
        }
    }
}

impl TempoTrackUI {
    pub fn show(&mut self, ui: &mut egui::Ui, state: &AppState) -> Option<TempoAction> {
        let map = state.tempo_map.clone();
        let curve = TempoCurve::from_map(&map, f64::from(state.bpm));

        if !ui.input(|i| i.pointer.primary_down()) {
            self.span = (map.last().map_or(DEFAULT_SPAN, |p| p.beat) + 8.0).max(DEFAULT_SPAN);
            self.undo_armed = false;
        }

        let mut action = None;
        ui.vertical(|ui| {
            let mut cleared = false;
            ui.horizontal(|ui| {
                let text = if map.len() < 2 {
                    "Tempo: constant"
                } else {
                    "Tempo: automated"
                };
                if ui.selectable_label(self.open, text).clicked() {
                    self.open = !self.open;
                }
                ui.label(format!("{:.1} BPM", state.bpm));
                if !map.is_empty() && ui.button("Clear").clicked() {
                    self.selected = None;
                    cleared = true;
                }
            });
            if cleared {
                action = Some(self.commit(Vec::new(), false));
                return;
            }
            if !self.open {
                return;
            }
            if self.selected.is_some_and(|i| i >= map.len()) {
                self.selected = None;
            }

            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), LANE_HEIGHT),
                egui::Sense::click_and_drag(),
            );
            let mut next = map.clone();
            let dragging = self.paint(ui, rect, &map, &curve, &mut next, &response);
            if next != map {
                action = Some(self.commit(next, dragging));
            }
        });
        action
    }

    fn commit(&mut self, points: Vec<TempoPoint>, dragging: bool) -> TempoAction {
        let undo = !dragging || !self.undo_armed;
        if dragging {
            self.undo_armed = true;
        }
        TempoAction::Set { points, undo }
    }

    fn paint(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        map: &[TempoPoint],
        curve: &TempoCurve,
        next: &mut Vec<TempoPoint>,
        response: &egui::Response,
    ) -> bool {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(22));

        let span = self.span;
        let to_beat = |x: f32| -> f64 { ((x - rect.left()) / rect.width()) as f64 * span };
        let to_bpm = |y: f32| -> f64 {
            let t = ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0) as f64;
            MIN_BPM + t * (MAX_BPM - MIN_BPM)
        };
        let x_of = |beat: f64| rect.left() + (beat / span) as f32 * rect.width();
        let y_of = |bpm: f64| {
            let t = ((bpm - MIN_BPM) / (MAX_BPM - MIN_BPM)).clamp(0.0, 1.0) as f32;
            rect.bottom() - t * rect.height()
        };

        for bpm in [40.0, 80.0, 120.0, 160.0, 200.0, 240.0] {
            let y = y_of(bpm);
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(45)),
            );
            painter.text(
                egui::pos2(rect.left() + 2.0, y - 2.0),
                egui::Align2::LEFT_TOP,
                format!("{bpm:.0}"),
                egui::FontId::proportional(9.0),
                egui::Color32::from_gray(110),
            );
        }

        let mut line = Vec::with_capacity(257);
        for step in 0..=256 {
            let beat = span * step as f64 / 256.0;
            let bpm = curve.bpm_at(beat);
            if bpm.is_finite() {
                line.push(egui::pos2(x_of(beat), y_of(bpm)));
            }
        }
        if line.len() >= 2 {
            painter.add(egui::Shape::line(
                line,
                egui::Stroke::new(1.6, egui::Color32::from_rgb(120, 200, 255)),
            ));
        }

        let mut dragging = false;
        for (index, point) in map.iter().enumerate() {
            let pos = egui::pos2(x_of(point.beat), y_of(point.bpm));
            let id = egui::Id::new(("tempo_point", index));
            let handle = egui::Rect::from_center_size(pos, egui::vec2(HANDLE, HANDLE));
            let handle_response = ui.interact(handle, id, egui::Sense::click_and_drag());
            let picked = self.selected == Some(index);

            let color = if picked {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_rgb(120, 200, 255)
            };
            painter.circle_filled(pos, 5.0, color);
            painter.circle_stroke(pos, 5.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
            if point.ramp == TempoRamp::Hold {
                painter.rect_stroke(
                    egui::Rect::from_center_size(pos, egui::vec2(11.0, 11.0)),
                    0.0,
                    egui::Stroke::new(1.0, color),
                    egui::StrokeKind::Inside,
                );
            }

            if handle_response.clicked() {
                self.selected = Some(index);
            }
            if handle_response.secondary_clicked() && index > 0 {
                next.remove(index);
                self.selected = None;
                return dragging;
            }
            if handle_response.dragged()
                && let Some(pointer) = handle_response.interact_pointer_pos()
            {
                dragging = true;
                let beat = if index == 0 {
                    0.0
                } else {
                    let lo = next[index - 1].beat + MIN_GAP;
                    let hi = next
                        .get(index + 1)
                        .map_or(span, |p| (p.beat - MIN_GAP).max(lo));
                    to_beat(pointer.x).clamp(lo, hi)
                };
                next[index].beat = beat;
                next[index].bpm = to_bpm(pointer.y);
            }
        }

        if let Some(index) = self.selected
            && let Some(point) = map.get(index)
        {
            ui.horizontal(|ui| {
                ui.label(format!("Beat {:.2}  {:.1} BPM", point.beat, point.bpm));
                for (ramp, name) in [
                    (TempoRamp::Hold, "Hold"),
                    (TempoRamp::Beats, "Ramp"),
                    (TempoRamp::Seconds, "Ramp/sec"),
                ] {
                    if ui.selectable_label(point.ramp == ramp, name).clicked() {
                        next[index].ramp = ramp;
                    }
                }
                if ui.button("Delete").clicked() && index > 0 {
                    next.remove(index);
                    self.selected = None;
                }
            });
        }

        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && map.iter().all(|p| (x_of(p.beat) - pos.x).abs() > HANDLE)
        {
            let clicked = to_beat(pos.x);
            let mut lo = 0.0;
            let mut hi = span;
            for p in map {
                if p.beat <= clicked {
                    lo = p.beat + MIN_GAP;
                } else {
                    hi = hi.min(p.beat - MIN_GAP);
                }
            }
            if hi >= lo {
                let beat = clicked.clamp(lo, hi);
                let index = next
                    .iter()
                    .position(|p| p.beat > beat)
                    .unwrap_or(next.len());
                next.insert(index, TempoPoint::new(beat, to_bpm(pos.y)));
                self.selected = Some(index);
            }
        }

        dragging
    }
}
