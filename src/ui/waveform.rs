use crate::model::AudioClip;
use eframe::egui;

/// Draws only the waveform lines.
pub fn draw_waveform(
    painter: &egui::Painter,
    rect: egui::Rect,
    clip: &AudioClip,
    color: egui::Color32,
) {
    let samples_per_pixel =
        (clip.samples.len() as f32 / rect.width().max(1.0)).max(1.0);

    let mut points = Vec::with_capacity(rect.width() as usize * 2);
    let center_y = rect.center().y;
    let height = rect.height() * 0.8;

    let stroke = egui::Stroke::new(1.0, color);

    for pixel_x in 0..rect.width() as i32 {
        let s0 = (pixel_x as f32 * samples_per_pixel) as usize;
        let s1 = (((pixel_x + 1) as f32) * samples_per_pixel) as usize;

        if s0 >= clip.samples.len() {
            break;
        }
        let end = s1.min(clip.samples.len());

        let mut min_val = 0.0f32;
        let mut max_val = 0.0f32;
        for i in s0..end {
            min_val = min_val.min(clip.samples[i]);
            max_val = max_val.max(clip.samples[i]);
        }

        let x = rect.left() + pixel_x as f32;
        let y_min = center_y - max_val * height * 0.5;
        let y_max = center_y - min_val * height * 0.5;

        points.push(egui::pos2(x, y_min));
        points.push(egui::pos2(x, y_max));
    }

    for chunk in points.chunks(2) {
        if let [a, b] = chunk {
            painter.line_segment([*a, *b], stroke);
        }
    }
}
