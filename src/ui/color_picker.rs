use crate::model::group::COLOR_PALETTE;
use egui::{Color32, Ui};

pub struct ColorPicker;

impl ColorPicker {
    /// Shows a palette grid, returns Some(color) if one was clicked
    pub fn palette_grid(ui: &mut Ui, current: (u8, u8, u8)) -> Option<(u8, u8, u8)> {
        let mut selected = None;

        ui.horizontal_wrapped(|ui| {
            for &(r, g, b) in COLOR_PALETTE {
                let color = Color32::from_rgb(r, g, b);
                let is_current = current == (r, g, b);

                let size = egui::vec2(24.0, 24.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, 4.0, color);
                    if is_current {
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(2.0, Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                }

                if response.clicked() {
                    selected = Some((r, g, b));
                }
            }
        });

        selected
    }

    /// Full color picker with palette + custom RGB
    pub fn show(ui: &mut Ui, color: &mut (u8, u8, u8)) -> bool {
        let mut changed = false;

        ui.vertical(|ui| {
            ui.label("Preset Colors:");
            if let Some(new_color) = Self::palette_grid(ui, *color) {
                *color = new_color;
                changed = true;
            }

            ui.separator();
            ui.label("Custom:");

            ui.horizontal(|ui| {
                let mut rgb = [color.0, color.1, color.2];
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    *color = (rgb[0], rgb[1], rgb[2]);
                    changed = true;
                }
            });
        });

        changed
    }
}
