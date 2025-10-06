use eframe::egui;

use crate::metering::{MeterData, draw_meter_bar};

#[derive(Clone, Debug, Default)]
pub struct LevelMeter {
    data: MeterData,
}

impl LevelMeter {
    pub fn update(&mut self, samples: &[f32], dt: f32) {
        self.data.update(samples, dt);
    }

    pub fn ui(&self, ui: &mut egui::Ui, vertical: bool) {
        let size = if vertical {
            egui::vec2(20.0, 200.0)
        } else {
            egui::vec2(200.0, 20.0)
        };

        let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
        let rect = response.rect;

        // Use the common drawing function
        draw_meter_bar(&painter, rect, &self.data, vertical);
    }
}
