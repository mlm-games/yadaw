use super::*;
use crate::metering::{MeterData, draw_meter_bar};

// Custom reusable widgets

/// A knob control widget
pub struct Knob<'a> {
    value: &'a mut f32,
    min: f32,
    max: f32,
    label: Option<&'a str>,
}

impl<'a> Knob<'a> {
    pub fn new(value: &'a mut f32) -> Self {
        Self {
            value,
            min: 0.0,
            max: 1.0,
            label: None,
        }
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let size = egui::vec2(40.0, 40.0);
        let (response, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());

        let center = response.rect.center();
        let radius = response.rect.width() / 2.0 - 2.0;

        // Draw knob background
        painter.circle_filled(center, radius, egui::Color32::from_gray(40));
        painter.circle_stroke(
            center,
            radius,
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        );

        // Calculate angle from value
        let normalized = (*self.value - self.min) / (self.max - self.min);
        let angle = normalized * std::f32::consts::PI * 1.5 - std::f32::consts::PI * 0.75;

        // Draw indicator
        let indicator_pos =
            center + egui::vec2(angle.cos() * radius * 0.7, angle.sin() * radius * 0.7);

        painter.line_segment(
            [center, indicator_pos],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
        );

        // Handle interaction
        if response.dragged() {
            let delta = response.drag_delta();
            let sensitivity = 0.01;
            let change = -delta.y * sensitivity * (self.max - self.min);
            *self.value = (*self.value + change).clamp(self.min, self.max);
        }

        // Draw label if provided
        if let Some(label) = self.label {
            painter.text(
                center + egui::vec2(0.0, radius + 10.0),
                egui::Align2::CENTER_TOP,
                label,
                egui::FontId::default(),
                egui::Color32::from_gray(200),
            );
        }

        // Show value on hover
        if response.hovered() {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                format!("{:.1}", self.value),
                egui::FontId::default(),
                egui::Color32::WHITE,
            );
        }

        response
    }
}

/// A VU meter widget
pub struct VuMeter<'a> {
    data: &'a MeterData,
    vertical: bool,
    show_db: bool,
}

impl<'a> VuMeter<'a> {
    pub fn new(data: &'a MeterData) -> Self {
        Self {
            data,
            vertical: true,
            show_db: true,
        }
    }

    pub fn horizontal(mut self) -> Self {
        self.vertical = false;
        self
    }

    pub fn show_db(mut self, show: bool) -> Self {
        self.show_db = show;
        self
    }

    pub fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let size = if self.vertical {
            egui::vec2(20.0, 200.0)
        } else {
            egui::vec2(200.0, 20.0)
        };

        let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());

        // Use the common drawing function
        draw_meter_bar(&painter, response.rect, self.data, self.vertical);

        // Show dB value on hover
        if self.show_db && response.hovered() {
            painter.text(
                response.rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{:.1} dB", self.data.peak_db()),
                egui::FontId::default(),
                egui::Color32::WHITE,
            );
        }

        response
    }
}

/// A resizable panel splitter
pub struct PanelSplitter {
    split_ratio: f32,
    vertical: bool,
    min_size: f32,
}

impl PanelSplitter {
    pub fn new(split_ratio: f32, vertical: bool) -> Self {
        Self {
            split_ratio,
            vertical,
            min_size: 50.0,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        first_panel: impl FnOnce(&mut egui::Ui),
        second_panel: impl FnOnce(&mut egui::Ui),
    ) {
        let available_rect = ui.available_rect_before_wrap();

        if self.vertical {
            let split_x = available_rect.left() + available_rect.width() * self.split_ratio;

            // First panel
            let first_rect = egui::Rect::from_min_max(
                available_rect.min,
                egui::pos2(split_x - 2.0, available_rect.bottom()),
            );

            ui.allocate_ui_at_rect(first_rect, first_panel);

            // Splitter handle
            let splitter_rect = egui::Rect::from_min_size(
                egui::pos2(split_x - 2.0, available_rect.top()),
                egui::vec2(4.0, available_rect.height()),
            );

            let splitter_response = ui.interact(
                splitter_rect,
                ui.id().with("splitter"),
                egui::Sense::click_and_drag(),
            );

            if splitter_response.hovered() || splitter_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }

            if splitter_response.dragged() {
                let delta = splitter_response.drag_delta().x;
                let new_ratio = ((split_x + delta - available_rect.left())
                    / available_rect.width())
                .clamp(0.1, 0.9);
                self.split_ratio = new_ratio;
            }

            ui.painter().rect_filled(
                splitter_rect,
                0.0,
                if splitter_response.hovered() {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(40)
                },
            );

            // Second panel
            let second_rect = egui::Rect::from_min_max(
                egui::pos2(split_x + 2.0, available_rect.top()),
                available_rect.max,
            );

            ui.allocate_ui_at_rect(second_rect, second_panel);
        } else {
            // Horizontal split
            let split_y = available_rect.top() + available_rect.height() * self.split_ratio;

            // First panel
            let first_rect = egui::Rect::from_min_max(
                available_rect.min,
                egui::pos2(available_rect.right(), split_y - 2.0),
            );

            ui.allocate_ui_at_rect(first_rect, first_panel);

            // Splitter handle
            let splitter_rect = egui::Rect::from_min_size(
                egui::pos2(available_rect.left(), split_y - 2.0),
                egui::vec2(available_rect.width(), 4.0),
            );

            let splitter_response = ui.interact(
                splitter_rect,
                ui.id().with("h_splitter"),
                egui::Sense::click_and_drag(),
            );

            if splitter_response.hovered() || splitter_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }

            if splitter_response.dragged() {
                let delta = splitter_response.drag_delta().y;
                let new_ratio = ((split_y + delta - available_rect.top())
                    / available_rect.height())
                .clamp(0.1, 0.9);
                self.split_ratio = new_ratio;
            }

            ui.painter().rect_filled(
                splitter_rect,
                0.0,
                if splitter_response.hovered() {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(40)
                },
            );

            // Second panel
            let second_rect = egui::Rect::from_min_max(
                egui::pos2(available_rect.left(), split_y + 2.0),
                available_rect.max,
            );

            ui.allocate_ui_at_rect(second_rect, second_panel);
        }
    }
}
