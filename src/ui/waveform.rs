use crate::model::AudioClip;
use eframe::egui;

pub fn draw_waveform(
    painter: &egui::Painter,
    rect: egui::Rect,
    clip: &AudioClip,
    zoom_x: f32,
    scroll_x: f32,
) {
    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));

    let clip_px_total = (clip.length_beats as f32 * zoom_x).max(1.0);
    let start_px = scroll_x.clamp(0.0, clip_px_total);
    let start_sample = ((start_px / clip_px_total) * clip.samples.len() as f32) as usize;

    let samples_per_pixel =
        ((clip.samples.len().saturating_sub(start_sample)) as f32 / rect.width().max(1.0)).max(1.0);

    let mut points = Vec::new();
    let center_y = rect.center().y;
    let height = rect.height() * 0.8;

    for pixel_x in 0..rect.width() as i32 {
        let s0 = start_sample + (pixel_x as f32 * samples_per_pixel) as usize;
        let s1 = start_sample + (((pixel_x + 1) as f32) * samples_per_pixel) as usize;

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
            painter.line_segment(
                [*a, *b],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 200)),
            );
        }
    }

    painter.text(
        rect.left_top() + egui::vec2(4.0, 4.0),
        egui::Align2::LEFT_TOP,
        &clip.name,
        egui::FontId::default(),
        egui::Color32::WHITE,
    );
}
