use egui::{Color32, Pos2, Rect, Stroke, Ui};

/// EQ band parameters
#[derive(Clone, Debug)]
pub struct EqBand {
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    pub band_type: BandType,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BandType {
    LowShelf,
    HighShelf,
    Peak,
    LowPass,
    HighPass,
}

impl Default for EqBand {
    fn default() -> Self {
        Self {
            freq: 1000.0,
            gain: 0.0,
            q: 1.0,
            band_type: BandType::Peak,
            enabled: true,
        }
    }
}

/// EQ Visualizer that reads plugin params and draws curve
pub struct EqVisualizer {
    bands: Vec<EqBand>,
    selected_band: Option<usize>,
    drag_mode: DragMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DragMode {
    None,
    FreqGain, // Left mouse: drag freq (X) and gain (Y)
    Q,        // Right mouse or Ctrl+drag: adjust Q
}

impl EqVisualizer {
    pub fn new() -> Self {
        Self {
            bands: Vec::new(),
            selected_band: None,
            drag_mode: DragMode::None,
        }
    }

    /// Create with default 8-band parametric EQ
    pub fn new_parametric(num_bands: usize) -> Self {
        let bands = Self::create_default_bands(num_bands);
        Self {
            bands,
            selected_band: None,
            drag_mode: DragMode::None,
        }
    }

    fn create_default_bands(num_bands: usize) -> Vec<EqBand> {
        let frequencies = [80.0, 200.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 12000.0];

        (0..num_bands)
            .map(|i| {
                let freq = if i < frequencies.len() {
                    frequencies[i]
                } else {
                    1000.0 * (i as f32 + 1.0)
                };

                let band_type = if i == 0 {
                    BandType::LowShelf
                } else if i == num_bands - 1 {
                    BandType::HighShelf
                } else {
                    BandType::Peak
                };

                EqBand {
                    freq,
                    gain: 0.0,
                    q: 1.0,
                    band_type,
                    enabled: true,
                }
            })
            .collect()
    }

    /// Update bands from plugin parameters
    /// Expects params with patterns like: "Band 1 Freq", "Band 1 Gain", etc.
    pub fn update_from_params(&mut self, params: &[(String, f32)]) {
        // Use BTreeMap to maintain sorted order by band number
        let mut band_map: std::collections::BTreeMap<usize, EqBand> =
            std::collections::BTreeMap::new();

        for (name, value) in params {
            let lower = name.to_lowercase();

            // Try multiple extraction patterns
            let band_num = Self::extract_band_number(&lower);
            let Some(idx) = band_num else { continue };

            let band = band_map.entry(idx).or_insert_with(EqBand::default);

            if lower.contains("freq") || lower.contains("frequency") || lower.contains("hz") {
                band.freq = value.clamp(20.0, 20000.0);
            } else if lower.contains("gain") || lower.contains("level") || lower.contains("db") {
                band.gain = value.clamp(-24.0, 24.0);
            } else if lower.contains("q") || lower.contains("bandwidth") || lower.contains("bw") {
                band.q = value.clamp(0.1, 10.0);
            } else if lower.contains("type") || lower.contains("mode") || lower.contains("shape") {
                band.band_type = Self::parse_band_type(*value as i32);
            } else if lower.contains("enable") || lower.contains("on") || lower.contains("active") {
                band.enabled = *value > 0.5;
            }
        }

        if !band_map.is_empty() {
            self.bands = band_map.into_values().collect();
        }
    }

    fn extract_band_number(name: &str) -> Option<usize> {
        // Pattern 1: "band 1", "band_1", "band-1"
        if let Some(rest) = name
            .strip_prefix("band")
            .map(|s| s.trim_start_matches(|c: char| c == '_' || c == '-' || c == ' '))
        {
            if let Some(num) = rest.split(|c: char| !c.is_ascii_digit()).next() {
                if let Ok(n) = num.parse::<usize>() {
                    return Some(n);
                }
            }
        }

        // Pattern 2: "eq1", "eq 1"
        if let Some(rest) = name.strip_prefix("eq").map(|s| s.trim()) {
            if let Some(num) = rest.split(|c: char| !c.is_ascii_digit()).next() {
                if let Ok(n) = num.parse::<usize>() {
                    return Some(n);
                }
            }
        }

        // Pattern 3: "1 freq", "1_gain" - number at start
        if let Some(num_str) = name.split(|c: char| !c.is_ascii_digit()).next() {
            if !num_str.is_empty() {
                if let Ok(n) = num_str.parse::<usize>() {
                    return Some(n);
                }
            }
        }

        // Pattern 4: "freq1", "gain_2" - number at end
        let digits: String = name
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if !digits.is_empty() {
            return digits.parse().ok();
        }

        None
    }

    fn parse_band_type(value: i32) -> BandType {
        match value {
            0 => BandType::Peak,
            1 => BandType::LowShelf,
            2 => BandType::HighShelf,
            3 => BandType::LowPass,
            4 => BandType::HighPass,
            _ => BandType::Peak,
        }
    }

    /// Manually set bands
    pub fn set_bands(&mut self, bands: Vec<EqBand>) {
        self.bands = bands;
    }

    /// Get current bands (for saving state)
    pub fn bands(&self) -> &[EqBand] {
        &self.bands
    }

    /// Get mutable bands
    pub fn bands_mut(&mut self) -> &mut Vec<EqBand> {
        &mut self.bands
    }

    /// Add a new band
    pub fn add_band(&mut self, freq: f32) -> usize {
        let band = EqBand {
            freq: freq.clamp(20.0, 20000.0),
            gain: 0.0,
            q: 1.0,
            band_type: BandType::Peak,
            enabled: true,
        };
        self.bands.push(band);
        self.bands
            .sort_by(|a, b| a.freq.partial_cmp(&b.freq).unwrap());
        self.bands
            .iter()
            .position(|b| (b.freq - freq).abs() < 0.01)
            .unwrap_or(self.bands.len() - 1)
    }

    /// Remove a band
    pub fn remove_band(&mut self, index: usize) {
        if index < self.bands.len() {
            self.bands.remove(index);
        }
    }

    /// Draw the EQ curve
    pub fn show(&mut self, ui: &mut Ui, size: egui::Vec2) -> Option<EqInteraction> {
        let (response, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
        let rect = response.rect;

        // Background
        painter.rect_filled(rect, 4.0, Color32::from_rgb(25, 25, 30));

        // Grid
        self.draw_grid(&painter, rect);

        // Frequency response curve
        self.draw_curve(&painter, rect);

        // Band handles
        let interaction = self.draw_handles(&painter, rect, &response);

        // Double-click to add band
        if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let freq = x_to_freq(pos.x, rect);
                let idx = self.add_band(freq);
                return Some(EqInteraction::BandAdded {
                    band_index: idx,
                    freq,
                });
            }
        }

        // Border
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, Color32::from_gray(60)),
            egui::StrokeKind::Inside,
        );

        interaction
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: Rect) {
        let grid_color = Color32::from_gray(45);
        let text_color = Color32::from_gray(120);

        // Frequency lines (logarithmic)
        let freqs = [
            20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0,
        ];
        for freq in freqs {
            let x = freq_to_x(freq, rect);
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, grid_color),
            );

            let label = if freq >= 1000.0 {
                format!("{}k", freq as i32 / 1000)
            } else {
                format!("{}", freq as i32)
            };
            painter.text(
                Pos2::new(x, rect.bottom() - 12.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(9.0),
                text_color,
            );
        }

        // Gain lines
        let gains = [-18.0, -12.0, -6.0, 0.0, 6.0, 12.0, 18.0];
        for gain in gains {
            let y = gain_to_y(gain, rect);
            let stroke = if gain == 0.0 {
                Stroke::new(1.5, Color32::from_gray(60))
            } else {
                Stroke::new(1.0, grid_color)
            };
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                stroke,
            );

            painter.text(
                Pos2::new(rect.left() + 16.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{:+.0}", gain),
                egui::FontId::proportional(9.0),
                text_color,
            );
        }
    }

    fn draw_curve(&self, painter: &egui::Painter, rect: Rect) {
        let zero_y = gain_to_y(0.0, rect);

        if self.bands.is_empty() {
            painter.line_segment(
                [
                    Pos2::new(rect.left(), zero_y),
                    Pos2::new(rect.right(), zero_y),
                ],
                Stroke::new(2.0, Color32::from_rgb(100, 180, 255)),
            );
            return;
        }

        // Sample frequency response
        let num_points = (rect.width() as usize).max(100);
        let mut points = Vec::with_capacity(num_points);

        for i in 0..num_points {
            let t = i as f32 / (num_points - 1) as f32;
            let x = rect.left() + t * rect.width();
            let freq = x_to_freq(x, rect);
            let gain_db = self.calculate_response(freq);
            let y = gain_to_y(gain_db, rect).clamp(rect.top(), rect.bottom());
            points.push(Pos2::new(x, y));
        }

        // Draw filled area
        let mut fill_points = points.clone();
        fill_points.push(Pos2::new(rect.right(), zero_y));
        fill_points.push(Pos2::new(rect.left(), zero_y));

        painter.add(egui::Shape::convex_polygon(
            fill_points,
            Color32::from_rgba_unmultiplied(100, 180, 255, 30),
            Stroke::NONE,
        ));

        // Draw curve line
        for window in points.windows(2) {
            painter.line_segment(
                [window[0], window[1]],
                Stroke::new(2.0, Color32::from_rgb(100, 180, 255)),
            );
        }

        // Draw individual band curves (dimmed)
        for (i, band) in self.bands.iter().enumerate() {
            if !band.enabled {
                continue;
            }

            let color = Self::band_color(i);
            let mut band_points = Vec::with_capacity(num_points);

            for j in 0..num_points {
                let t = j as f32 / (num_points - 1) as f32;
                let x = rect.left() + t * rect.width();
                let freq = x_to_freq(x, rect);
                let gain_db = self.calculate_band_response(band, freq);
                let y = gain_to_y(gain_db, rect).clamp(rect.top(), rect.bottom());
                band_points.push(Pos2::new(x, y));
            }

            for window in band_points.windows(2) {
                painter.line_segment(
                    [window[0], window[1]],
                    Stroke::new(1.0, color.gamma_multiply(0.4)),
                );
            }
        }
    }

    fn draw_handles(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        response: &egui::Response,
    ) -> Option<EqInteraction> {
        let mut interaction = None;
        let pointer_pos = response.interact_pointer_pos();
        let drag_started = response.drag_started();
        let dragged = response.dragged();
        let drag_stopped = response.drag_stopped();
        let modifiers = response.ctx.input(|i| i.modifiers);

        for (i, band) in self.bands.iter_mut().enumerate() {
            if !band.enabled {
                continue;
            }

            let x = freq_to_x(band.freq, rect);
            let y = gain_to_y(band.gain, rect);
            let center = Pos2::new(x, y);
            let radius = 10.0;

            let color = Self::band_color(i);
            let is_selected = self.selected_band == Some(i);

            // Q indicator ring
            let q_radius = radius + 4.0 + (1.0 / band.q) * 8.0;
            painter.circle_stroke(
                center,
                q_radius,
                Stroke::new(1.5, color.gamma_multiply(0.3)),
            );

            // Handle circle
            painter.circle_filled(
                center,
                radius,
                if is_selected {
                    color
                } else {
                    color.gamma_multiply(0.7)
                },
            );
            painter.circle_stroke(center, radius, Stroke::new(2.0, Color32::WHITE));

            // Band type indicator
            let type_char = match band.band_type {
                BandType::LowShelf => "L",
                BandType::HighShelf => "H",
                BandType::Peak => "●",
                BandType::LowPass => "⊥",
                BandType::HighPass => "⊤",
            };
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                type_char,
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );

            // Check interaction
            if let Some(pointer) = pointer_pos {
                let dist = (pointer - center).length();

                if drag_started && dist < radius * 1.5 {
                    self.selected_band = Some(i);
                    self.drag_mode = if modifiers.ctrl || modifiers.alt {
                        DragMode::Q
                    } else {
                        DragMode::FreqGain
                    };
                }

                if self.selected_band == Some(i) && dragged {
                    match self.drag_mode {
                        DragMode::FreqGain => {
                            let new_freq = x_to_freq(pointer.x, rect).clamp(20.0, 20000.0);
                            let new_gain = y_to_gain(pointer.y, rect).clamp(-24.0, 24.0);
                            band.freq = new_freq;
                            band.gain = new_gain;
                            interaction = Some(EqInteraction::BandChanged {
                                band_index: i,
                                freq: new_freq,
                                gain: new_gain,
                                q: band.q,
                            });
                        }
                        DragMode::Q => {
                            // Vertical drag changes Q
                            let delta_y = response.drag_delta().y;
                            let new_q = (band.q + delta_y * 0.02).clamp(0.1, 10.0);
                            band.q = new_q;
                            interaction = Some(EqInteraction::BandChanged {
                                band_index: i,
                                freq: band.freq,
                                gain: band.gain,
                                q: new_q,
                            });
                        }
                        DragMode::None => {}
                    }
                }
            }
        }

        if drag_stopped {
            self.selected_band = None;
            self.drag_mode = DragMode::None;
        }

        interaction
    }

    /// Calculate combined frequency response
    fn calculate_response(&self, freq: f32) -> f32 {
        let mut total_db = 0.0;
        for band in &self.bands {
            if band.enabled {
                total_db += self.calculate_band_response(band, freq);
            }
        }
        total_db.clamp(-24.0, 24.0)
    }

    /// Calculate single band response using more accurate biquad approximation
    fn calculate_band_response(&self, band: &EqBand, freq: f32) -> f32 {
        let f0 = band.freq;
        let gain = band.gain;
        let q = band.q.max(0.1);

        match band.band_type {
            BandType::Peak => {
                // More accurate bell curve
                let ratio = freq / f0;
                let log_ratio = ratio.ln();
                let bandwidth = (2.0_f32.ln() / 2.0) / q;
                let x = log_ratio / bandwidth;
                gain * (-x * x * 0.5).exp()
            }
            BandType::LowShelf => {
                let ratio = freq / f0;
                let transition = 1.0 / (1.0 + ratio.powf(2.0 * q));
                gain * transition
            }
            BandType::HighShelf => {
                let ratio = freq / f0;
                let transition = 1.0 / (1.0 + (1.0 / ratio).powf(2.0 * q));
                gain * transition
            }
            BandType::LowPass => {
                let ratio = freq / f0;
                if ratio > 1.0 {
                    -20.0 * ratio.log10() * q.sqrt() * 2.0
                } else {
                    0.0
                }
            }
            BandType::HighPass => {
                let ratio = freq / f0;
                if ratio < 1.0 {
                    20.0 * ratio.log10() * q.sqrt() * 2.0
                } else {
                    0.0
                }
            }
        }
    }

    fn band_color(index: usize) -> Color32 {
        const COLORS: [Color32; 8] = [
            Color32::from_rgb(255, 100, 100),
            Color32::from_rgb(255, 180, 100),
            Color32::from_rgb(255, 255, 100),
            Color32::from_rgb(100, 255, 100),
            Color32::from_rgb(100, 255, 255),
            Color32::from_rgb(100, 150, 255),
            Color32::from_rgb(200, 100, 255),
            Color32::from_rgb(255, 100, 200),
        ];
        COLORS[index % COLORS.len()]
    }
}

impl Default for EqVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

// Free functions for coordinate conversion
fn freq_to_x(freq: f32, rect: Rect) -> f32 {
    let min_log = 20.0_f32.ln();
    let max_log = 20000.0_f32.ln();
    let t = (freq.ln() - min_log) / (max_log - min_log);
    rect.left() + t * rect.width()
}

fn x_to_freq(x: f32, rect: Rect) -> f32 {
    let t = (x - rect.left()) / rect.width();
    let min_log = 20.0_f32.ln();
    let max_log = 20000.0_f32.ln();
    (min_log + t * (max_log - min_log)).exp()
}

fn gain_to_y(gain: f32, rect: Rect) -> f32 {
    let t = (gain + 24.0) / 48.0;
    rect.bottom() - t * rect.height()
}

fn y_to_gain(y: f32, rect: Rect) -> f32 {
    let t = (rect.bottom() - y) / rect.height();
    t * 48.0 - 24.0
}

/// Interaction events from EQ visualizer
#[derive(Debug, Clone)]
pub enum EqInteraction {
    BandChanged {
        band_index: usize,
        freq: f32,
        gain: f32,
        q: f32,
    },
    BandAdded {
        band_index: usize,
        freq: f32,
    },
}
