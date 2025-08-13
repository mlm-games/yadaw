use crate::audio_utils::linear_to_db;
use eframe::egui;

#[derive(Clone, Debug)]
/// Standard metering ranges and conversions
pub struct MeterScale {
    pub min_db: f32,
    pub max_db: f32,
    pub warning_db: f32,
    pub danger_db: f32,
}

impl Default for MeterScale {
    fn default() -> Self {
        Self {
            min_db: -60.0,
            max_db: 6.0,
            warning_db: -12.0,
            danger_db: -3.0,
        }
    }
}

impl MeterScale {
    /// Convert dB value to normalized position (0.0 to 1.0)
    #[inline]
    pub fn db_to_normalized(&self, db: f32) -> f32 {
        ((db - self.min_db) / (self.max_db - self.min_db)).clamp(0.0, 1.0)
    }

    /// Convert normalized position to dB value
    #[inline]
    pub fn normalized_to_db(&self, normalized: f32) -> f32 {
        self.min_db + normalized * (self.max_db - self.min_db)
    }

    /// Get color for a given dB level
    #[inline]
    pub fn level_color(&self, db: f32) -> egui::Color32 {
        if db > self.danger_db {
            egui::Color32::from_rgb(255, 0, 0)
        } else if db > self.warning_db {
            egui::Color32::from_rgb(255, 200, 0)
        } else {
            egui::Color32::from_rgb(0, 200, 0)
        }
    }
}

/// Common meter data that can be shared between different meter widgets
#[derive(Clone, Debug)]
pub struct MeterData {
    pub peak: f32,
    pub rms: f32,
    pub peak_hold: f32,
    pub peak_hold_time: f32,
    scale: MeterScale,
}

impl Default for MeterData {
    fn default() -> Self {
        Self {
            peak: 0.0,
            rms: 0.0,
            peak_hold: 0.0,
            peak_hold_time: 0.0,
            scale: MeterScale::default(),
        }
    }
}

impl MeterData {
    pub fn update(&mut self, samples: &[f32], dt: f32) {
        // Calculate peak
        self.peak = samples
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, |a, b| a.max(b));

        // Calculate RMS
        let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
        self.rms = (sum_squares / samples.len().max(1) as f32).sqrt();

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

    pub fn peak_db(&self) -> f32 {
        linear_to_db(self.peak)
    }

    pub fn rms_db(&self) -> f32 {
        linear_to_db(self.rms)
    }

    pub fn peak_hold_db(&self) -> f32 {
        linear_to_db(self.peak_hold)
    }

    pub fn peak_normalized(&self) -> f32 {
        self.scale.db_to_normalized(self.peak_db())
    }

    pub fn rms_normalized(&self) -> f32 {
        self.scale.db_to_normalized(self.rms_db())
    }

    pub fn peak_hold_normalized(&self) -> f32 {
        self.scale.db_to_normalized(self.peak_hold_db())
    }

    pub fn peak_color(&self) -> egui::Color32 {
        self.scale.level_color(self.peak_db())
    }
}

/// Draw a meter bar (can be used by both LevelMeter and VuMeter)
pub fn draw_meter_bar(painter: &egui::Painter, rect: egui::Rect, data: &MeterData, vertical: bool) {
    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));

    if vertical {
        // Vertical meter
        let peak_y = rect.bottom() - data.peak_normalized() * rect.height();
        let rms_y = rect.bottom() - data.rms_normalized() * rect.height();
        let peak_hold_y = rect.bottom() - data.peak_hold_normalized() * rect.height();

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
            data.peak_color(),
        );

        // Peak hold line
        if data.peak_hold_time > 0.0 {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), peak_hold_y),
                    egui::pos2(rect.right(), peak_hold_y),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 255, 0)),
            );
        }

        // 0 dB line
        let zero_db_y = rect.bottom() - data.scale.db_to_normalized(0.0) * rect.height();
        painter.line_segment(
            [
                egui::pos2(rect.left(), zero_db_y),
                egui::pos2(rect.right(), zero_db_y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 0, 0)),
        );
    } else {
        // Horizontal meter (similar logic)
        let peak_x = rect.left() + data.peak_normalized() * rect.width();
        let rms_x = rect.left() + data.rms_normalized() * rect.width();
        let peak_hold_x = rect.left() + data.peak_hold_normalized() * rect.width();

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
            data.peak_color(),
        );

        // Peak hold line
        if data.peak_hold_time > 0.0 {
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
