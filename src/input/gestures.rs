use egui::{Pos2, Vec2};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum GestureAction {
    Pan { delta: Vec2 },
    PinchZoom { scale: f32, center: Pos2 },
    Tap { pos: Pos2 },
    DoubleTap { pos: Pos2 },
    LongPress { pos: Pos2 },
}

pub struct GestureRecognizer {
    // Touch tracking
    touch_points: Vec<TouchPoint>,

    // Gesture state
    last_centroid: Option<Pos2>,
    last_distance: Option<f32>,

    // Tap detection
    tap_times: Vec<Instant>,
    last_tap_pos: Option<Pos2>,

    // Long press
    press_start: Option<(Pos2, Instant)>,

    // Configuration
    config: GestureConfig,
}

#[derive(Clone, Copy)]
struct TouchPoint {
    id: u64,
    pos: Pos2,
}

pub struct GestureConfig {
    pub double_tap_max_interval: f32, // 0.3s
    pub double_tap_max_distance: f32, // 20px
    pub long_press_duration: f32,     // 0.5s
    pub long_press_max_movement: f32, // 10px
    pub pan_min_distance: f32,        // 5px (dead zone)
    pub pinch_min_scale_change: f32,  // 0.05 (5%)
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            double_tap_max_interval: 0.3,
            double_tap_max_distance: 20.0,
            long_press_duration: 0.5,
            long_press_max_movement: 10.0,
            pan_min_distance: 5.0,
            pinch_min_scale_change: 0.05,
        }
    }
}

impl GestureRecognizer {
    pub fn new() -> Self {
        Self {
            touch_points: Vec::new(),
            last_centroid: None,
            last_distance: None,
            tap_times: Vec::new(),
            last_tap_pos: None,
            press_start: None,
            config: GestureConfig::default(),
        }
    }

    /// Process egui touch events, returns recognized gestures
    pub fn process(&mut self, ctx: &egui::Context) -> Vec<GestureAction> {
        let mut actions = Vec::new();

        // Extract touch points from egui events
        self.update_touch_points(ctx);

        // Multi-touch gestures (priority: pinch > pan)
        if self.touch_points.len() >= 2 {
            if let Some(gesture) = self.detect_pinch() {
                actions.push(gesture);
            } else if let Some(gesture) = self.detect_pan() {
                actions.push(gesture);
            }
        }
        // Single-touch gestures
        else if self.touch_points.len() == 1 {
            if let Some(gesture) = self.detect_pan() {
                actions.push(gesture);
            }

            // Check for long press
            if let Some(gesture) = self.detect_long_press() {
                actions.push(gesture);
            }
        }
        // Touch ended
        else if self.touch_points.is_empty() {
            // Check for tap/double-tap
            if let Some(gesture) = self.detect_tap() {
                actions.push(gesture);
            }

            // Reset state
            self.last_centroid = None;
            self.last_distance = None;
            self.press_start = None;
        }

        actions
    }

    fn update_touch_points(&mut self, ctx: &egui::Context) {
        self.touch_points.clear();

        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Touch { id, pos, phase, .. } = event {
                    match phase {
                        egui::TouchPhase::Start | egui::TouchPhase::Move => {
                            self.touch_points.push(TouchPoint {
                                id: id.0,
                                pos: *pos,
                            });

                            // Track press start for long-press
                            if *phase == egui::TouchPhase::Start && self.press_start.is_none() {
                                self.press_start = Some((*pos, Instant::now()));
                            }
                        }
                        egui::TouchPhase::End => {
                            // Touch ended - prepare for tap detection
                            if let Some((start_pos, _)) = self.press_start {
                                self.last_tap_pos = Some(start_pos);
                                self.tap_times.push(Instant::now());

                                // Keep only recent taps
                                self.tap_times.retain(|t| {
                                    t.elapsed().as_secs_f32() < self.config.double_tap_max_interval
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    fn detect_pinch(&mut self) -> Option<GestureAction> {
        if self.touch_points.len() < 2 {
            return None;
        }

        let centroid = self.compute_centroid();
        let distance = (self.touch_points[0].pos - self.touch_points[1].pos).length();

        if let (Some(last_c), Some(last_d)) = (self.last_centroid, self.last_distance) {
            let scale = (distance / last_d).clamp(0.5, 2.0);

            if (scale - 1.0).abs() > self.config.pinch_min_scale_change {
                self.last_centroid = Some(centroid);
                self.last_distance = Some(distance);
                return Some(GestureAction::PinchZoom {
                    scale,
                    center: centroid,
                });
            }
        }

        self.last_centroid = Some(centroid);
        self.last_distance = Some(distance);
        None
    }

    fn detect_pan(&mut self) -> Option<GestureAction> {
        if self.touch_points.is_empty() {
            return None;
        }

        let centroid = self.compute_centroid();

        if let Some(last_c) = self.last_centroid {
            let delta = centroid - last_c;

            if delta.length() > self.config.pan_min_distance {
                self.last_centroid = Some(centroid);
                return Some(GestureAction::Pan { delta });
            }
        }

        self.last_centroid = Some(centroid);
        None
    }

    fn detect_tap(&mut self) -> Option<GestureAction> {
        if self.tap_times.len() >= 2 {
            let last_two = &self.tap_times[self.tap_times.len() - 2..];
            let interval = last_two[1].duration_since(last_two[0]).as_secs_f32();

            if interval < self.config.double_tap_max_interval {
                if let Some(pos) = self.last_tap_pos {
                    self.tap_times.clear();
                    return Some(GestureAction::DoubleTap { pos });
                }
            }
        } else if self.tap_times.len() == 1 {
            // Single tap confirmed after timeout
            if self.tap_times[0].elapsed().as_secs_f32() > self.config.double_tap_max_interval {
                if let Some(pos) = self.last_tap_pos {
                    self.tap_times.clear();
                    return Some(GestureAction::Tap { pos });
                }
            }
        }

        None
    }

    fn detect_long_press(&mut self) -> Option<GestureAction> {
        if let Some((start_pos, start_time)) = self.press_start {
            if start_time.elapsed().as_secs_f32() > self.config.long_press_duration {
                if let Some(current_point) = self.touch_points.first() {
                    let movement = (current_point.pos - start_pos).length();

                    if movement < self.config.long_press_max_movement {
                        self.press_start = None; // Consume the gesture
                        return Some(GestureAction::LongPress {
                            pos: current_point.pos,
                        });
                    }
                }
            }
        }

        None
    }

    fn compute_centroid(&self) -> Pos2 {
        if self.touch_points.is_empty() {
            return Pos2::ZERO;
        }

        let sum: Vec2 = self
            .touch_points
            .iter()
            .map(|p| p.pos.to_vec2())
            .fold(Vec2::ZERO, |acc, v| acc + v);

        Pos2::ZERO + sum / self.touch_points.len() as f32
    }
}
