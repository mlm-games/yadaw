// src/level_meter.rs

use eframe::egui;

pub struct LevelMeter {
    peak: f32,
    rms: f32,
    peak_hold: f32,
    peak_hold_time: f32,
}

impl Default for LevelMeter {
    fn default() -> Self {
        Self {
            peak: 0.0,
            rms: 0.0,
            peak_hold: 0.0,
            peak_hold_time: 0.0,
        }
    }
}

impl LevelMeter {
    pub fn update(&mut self, samples: &[f32], dt: f32) {
        // Calculate peak
        self.peak = samples
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, |a, b| a.max(b));

        // Calculate RMS
        let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
        self.rms = (sum_squares / samples.len() as f32).sqrt();

        // Update peak hold
        if self.peak > self.peak_hold {
            self.peak_hold = self.peak;
            self.peak_hold_time = 2.0; // Hold for 2 seconds
        } else {
            self.peak_hold_time -= dt;
            if self.peak_hold_time <= 0.0 {
                self.peak_hold = self.peak;
            }
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui, vertical: bool) {
        let size = if vertical {
            egui::vec2(20.0, 200.0)
        } else {
            egui::vec2(200.0, 20.0)
        };

        let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
        let rect = response.rect;

        // Background
        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));

        // Convert levels to dB
        let peak_db = 20.0 * self.peak.max(0.0001).log10();
        let rms_db = 20.0 * self.rms.max(0.0001).log10();
        let peak_hold_db = 20.0 * self.peak_hold.max(0.0001).log10();

        // Map dB to position (0 dB = max, -60 dB = min)
        let db_to_pos = |db: f32| {
            let normalized = (db + 60.0) / 60.0;
            normalized.clamp(0.0, 1.0)
        };

        if vertical {
            // Vertical meter
            let peak_y = rect.bottom() - db_to_pos(peak_db) * rect.height();
            let rms_y = rect.bottom() - db_to_pos(rms_db) * rect.height();
            let peak_hold_y = rect.bottom() - db_to_pos(peak_hold_db) * rect.height();

            // RMS bar
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rms_y),
                    egui::pos2(rect.right(), rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgb(0, 100, 0),
            );

            // Peak bar
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), peak_y),
                    egui::pos2(rect.right(), rms_y),
                ),
                0.0,
                egui::Color32::from_rgb(0, 200, 0),
            );

            // Peak hold line
            if self.peak_hold_time > 0.0 {
                painter.line_segment(
                    [
                        egui::pos2(rect.left(), peak_hold_y),
                        egui::pos2(rect.right(), peak_hold_y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 255, 0)),
                );
            }

            // 0 dB line
            let zero_db_y = rect.bottom() - db_to_pos(0.0) * rect.height();
            painter.line_segment(
                [
                    egui::pos2(rect.left(), zero_db_y),
                    egui::pos2(rect.right(), zero_db_y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 0, 0)),
            );
        } else {
            // Horizontal meter (similar logic but horizontal)
            let peak_x = rect.left() + db_to_pos(peak_db) * rect.width();
            let rms_x = rect.left() + db_to_pos(rms_db) * rect.width();
            let peak_hold_x = rect.left() + db_to_pos(peak_hold_db) * rect.width();

            // RMS bar
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rms_x, rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgb(0, 100, 0),
            );

            // Peak bar
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rms_x, rect.top()),
                    egui::pos2(peak_x, rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgb(0, 200, 0),
            );

            // Peak hold line
            if self.peak_hold_time > 0.0 {
                painter.line_segment(
                    [
                        egui::pos2(peak_hold_x, rect.top()),
                        egui::pos2(peak_hold_x, rect.bottom()),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 255, 0)),
                );
            }
        }
    }
}
