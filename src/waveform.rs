use crate::state::AudioClip;
use eframe::egui;

pub fn draw_waveform(
    painter: &egui::Painter,
    rect: egui::Rect,
    clip: &AudioClip,
    zoom_x: f32,
    scroll_x: f32,
) {
    let samples_per_pixel =
        (clip.samples.len() as f32 / (clip.length_beats as f32 * zoom_x)).max(1.0);

    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));

    // Waveform
    let mut points = Vec::new();
    let center_y = rect.center().y;
    let height = rect.height() * 0.8;

    for pixel_x in 0..rect.width() as i32 {
        let sample_start = (pixel_x as f32 * samples_per_pixel) as usize;
        let sample_end = ((pixel_x + 1) as f32 * samples_per_pixel) as usize;

        if sample_start < clip.samples.len() {
            // Find min/max in this pixel's sample range
            let mut min_val = 0.0f32;
            let mut max_val = 0.0f32;

            for i in sample_start..sample_end.min(clip.samples.len()) {
                min_val = min_val.min(clip.samples[i]);
                max_val = max_val.max(clip.samples[i]);
            }

            let x = rect.left() + pixel_x as f32;
            let y_min = center_y - max_val * height * 0.5;
            let y_max = center_y - min_val * height * 0.5;

            points.push(egui::pos2(x, y_min));
            points.push(egui::pos2(x, y_max));
        }
    }

    // Draw the waveform
    for chunk in points.chunks(2) {
        if chunk.len() == 2 {
            painter.line_segment(
                [chunk[0], chunk[1]],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 200)),
            );
        }
    }

    // Draw clip name
    painter.text(
        rect.left_top() + egui::vec2(4.0, 4.0),
        egui::Align2::LEFT_TOP,
        &clip.name,
        egui::FontId::default(),
        egui::Color32::WHITE,
    );
}
