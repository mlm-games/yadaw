use crate::constants::DEFAULT_GRID_SNAP;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    pub id: usize,
    pub name: String,
    pub parameter: AutomationTarget,
    pub points: BTreeMap<OrderedFloat, AutomationPoint>,
    pub curve_type: CurveType,
    pub enabled: bool,
    pub visible: bool,
    pub color: (u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OrderedFloat(u64);

impl OrderedFloat {
    pub fn from_f64(value: f64) -> Self {
        OrderedFloat(value.to_bits())
    }

    pub fn to_f64(self) -> f64 {
        f64::from_bits(self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub value: f32,
    pub curve_tension: f32, // For bezier curves
    pub locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CurveType {
    Linear,
    Exponential,
    Logarithmic,
    SCurve,
    Bezier,
    Step,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
pub enum AutomationTarget {
    TrackVolume(usize),
    TrackPan(usize),
    TrackMute(usize),
    TrackSolo(usize),
    SendLevel(usize, usize),              // track_id, send_id
    PluginParameter(usize, usize, usize), // track_id, plugin_id, param_id
    MasterVolume,
    MasterPan,
    Tempo,
}

pub struct AutomationEngine {
    lanes: Vec<AutomationLane>,
    next_lane_id: usize,
    recording: bool,
    record_buffer: Vec<(f64, AutomationTarget, f32)>,
    overdub_mode: bool,
    snap_to_grid: bool,
    grid_size: f64,
}

impl AutomationEngine {
    pub fn new() -> Self {
        Self {
            lanes: Vec::new(),
            next_lane_id: 0,
            recording: false,
            record_buffer: Vec::new(),
            overdub_mode: false,
            snap_to_grid: false,
            grid_size: DEFAULT_GRID_SNAP as f64,
        }
    }

    pub fn create_lane(&mut self, name: String, parameter: AutomationTarget) -> usize {
        let id = self.next_lane_id;
        self.next_lane_id += 1;

        let color = match &parameter {
            AutomationTarget::TrackVolume(_) => (100, 150, 255),
            AutomationTarget::TrackPan(_) => (255, 150, 100),
            AutomationTarget::TrackMute(_) => (255, 100, 100),
            AutomationTarget::TrackSolo(_) => (255, 255, 100),
            AutomationTarget::SendLevel(_, _) => (150, 255, 150),
            AutomationTarget::PluginParameter(_, _, _) => (200, 150, 255),
            AutomationTarget::MasterVolume => (255, 255, 255),
            AutomationTarget::MasterPan => (200, 200, 200),
            AutomationTarget::Tempo => (255, 200, 100),
        };

        let lane = AutomationLane {
            id,
            name,
            parameter,
            points: BTreeMap::new(),
            curve_type: CurveType::Linear,
            enabled: true,
            visible: true,
            color,
        };

        self.lanes.push(lane);
        id
    }

    pub fn add_point(&mut self, lane_id: usize, position: f64, value: f32) {
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == lane_id) {
            let position = if self.snap_to_grid {
                (position / self.grid_size).round() * self.grid_size
            } else {
                position
            };

            let point = AutomationPoint {
                value: value.clamp(0.0, 1.0),
                curve_tension: 0.5,
                locked: false,
            };

            lane.points.insert(OrderedFloat::from_f64(position), point);
        }
    }

    pub fn remove_point(&mut self, lane_id: usize, position: f64) {
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == lane_id) {
            lane.points.remove(&OrderedFloat::from_f64(position));
        }
    }

    pub fn move_point(
        &mut self,
        lane_id: usize,
        old_position: f64,
        new_position: f64,
        new_value: Option<f32>,
    ) {
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == lane_id) {
            if let Some(mut point) = lane.points.remove(&OrderedFloat::from_f64(old_position)) {
                if let Some(value) = new_value {
                    point.value = value.clamp(0.0, 1.0);
                }

                let new_pos = if self.snap_to_grid {
                    (new_position / self.grid_size).round() * self.grid_size
                } else {
                    new_position
                };

                lane.points.insert(OrderedFloat::from_f64(new_pos), point);
            }
        }
    }

    pub fn get_value(&self, lane_id: usize, position: f64) -> Option<f32> {
        let lane = self.lanes.iter().find(|l| l.id == lane_id)?;

        if !lane.enabled || lane.points.is_empty() {
            return None;
        }

        let pos_key = OrderedFloat::from_f64(position);

        // Find surrounding points
        let before = lane.points.range(..=pos_key).last();
        let after = lane.points.range(pos_key..).next();

        match (before, after) {
            (Some((before_pos, before_point)), Some((after_pos, after_point))) => {
                if before_pos == after_pos {
                    Some(before_point.value)
                } else {
                    let before_time = before_pos.to_f64();
                    let after_time = after_pos.to_f64();
                    let t = (position - before_time) / (after_time - before_time);

                    Some(self.interpolate(
                        before_point.value,
                        after_point.value,
                        t as f32,
                        lane.curve_type,
                        before_point.curve_tension,
                    ))
                }
            }
            (Some((_, point)), None) | (None, Some((_, point))) => Some(point.value),
            _ => None,
        }
    }

    fn interpolate(
        &self,
        start: f32,
        end: f32,
        t: f32,
        curve_type: CurveType,
        tension: f32,
    ) -> f32 {
        let t = t.clamp(0.0, 1.0);

        match curve_type {
            CurveType::Linear => start + (end - start) * t,
            CurveType::Exponential => {
                let exp_t = (t * t).clamp(0.0, 1.0);
                start + (end - start) * exp_t
            }
            CurveType::Logarithmic => {
                let log_t = t.sqrt().clamp(0.0, 1.0);
                start + (end - start) * log_t
            }
            CurveType::SCurve => {
                let s_t = t * t * (3.0 - 2.0 * t);
                start + (end - start) * s_t
            }
            CurveType::Bezier => {
                // Simplified bezier using tension as control
                let control1 = start + (end - start) * tension;
                let control2 = end - (end - start) * tension;

                let t2 = t * t;
                let t3 = t2 * t;
                let mt = 1.0 - t;
                let mt2 = mt * mt;
                let mt3 = mt2 * mt;

                mt3 * start + 3.0 * mt2 * t * control1 + 3.0 * mt * t2 * control2 + t3 * end
            }
            CurveType::Step => {
                if t < 1.0 {
                    start
                } else {
                    end
                }
            }
        }
    }

    pub fn start_recording(&mut self) {
        self.recording = true;
        if !self.overdub_mode {
            self.record_buffer.clear();
        }
    }

    pub fn stop_recording(&mut self) -> Vec<(f64, AutomationTarget, f32)> {
        self.recording = false;
        self.record_buffer.clone()
    }

    pub fn record_value(&mut self, position: f64, target: AutomationTarget, value: f32) {
        if self.recording {
            self.record_buffer.push((position, target, value));
        }
    }

    pub fn process_recorded_automation(&mut self) {
        // Group recorded values by target
        let mut grouped: std::collections::HashMap<AutomationTarget, Vec<(f64, f32)>> =
            std::collections::HashMap::new();

        for (position, target, value) in &self.record_buffer {
            grouped
                .entry(target.clone())
                .or_insert_with(Vec::new)
                .push((*position, *value));
        }

        // Create or update lanes for each target
        for (target, values) in grouped {
            // Find or create lane for this target
            let lane_id = self
                .lanes
                .iter()
                .find(|l| l.parameter == target)
                .map(|l| l.id)
                .unwrap_or_else(|| {
                    let name = format!("{:?} Automation", target);
                    self.create_lane(name, target.clone())
                });

            // Add points with thinning to avoid too many points
            let thinned = self.thin_automation_points(values);
            for (position, value) in thinned {
                self.add_point(lane_id, position, value);
            }
        }

        self.record_buffer.clear();
    }

    fn thin_automation_points(&self, points: Vec<(f64, f32)>) -> Vec<(f64, f32)> {
        if points.len() <= 2 {
            return points;
        }

        // Simple Douglas-Peucker-like algorithm for thinning
        let tolerance = 0.01; // Value tolerance for keeping points
        let mut result = vec![points[0]];
        let mut last_kept = 0;

        for i in 1..points.len() - 1 {
            let interpolated = self.interpolate(
                points[last_kept].1,
                points[i + 1].1,
                ((points[i].0 - points[last_kept].0) / (points[i + 1].0 - points[last_kept].0))
                    as f32,
                CurveType::Linear,
                0.5,
            );

            if (points[i].1 - interpolated).abs() > tolerance {
                result.push(points[i]);
                last_kept = i;
            }
        }

        result.push(*points.last().unwrap());
        result
    }

    pub fn get_lanes(&self) -> &[AutomationLane] {
        &self.lanes
    }

    pub fn get_lane_mut(&mut self, lane_id: usize) -> Option<&mut AutomationLane> {
        self.lanes.iter_mut().find(|l| l.id == lane_id)
    }

    pub fn delete_lane(&mut self, lane_id: usize) {
        self.lanes.retain(|l| l.id != lane_id);
    }

    pub fn clear_lane(&mut self, lane_id: usize) {
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == lane_id) {
            lane.points.clear();
        }
    }

    pub fn set_curve_type(&mut self, lane_id: usize, curve_type: CurveType) {
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == lane_id) {
            lane.curve_type = curve_type;
        }
    }

    pub fn set_snap_to_grid(&mut self, enabled: bool) {
        self.snap_to_grid = enabled;
    }

    pub fn set_grid_size(&mut self, size: f64) {
        self.grid_size = size;
    }

    pub fn set_overdub_mode(&mut self, enabled: bool) {
        self.overdub_mode = enabled;
    }
}

// Helper functions for creating common automation curves
pub fn create_fade_in(duration: f64, steps: usize) -> Vec<(f64, f32)> {
    (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            (t * duration, t as f32)
        })
        .collect()
}

pub fn create_fade_out(start: f64, duration: f64, steps: usize) -> Vec<(f64, f32)> {
    (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            (start + t * duration, 1.0 - t as f32)
        })
        .collect()
}

pub fn create_tremolo(start: f64, duration: f64, rate: f64, depth: f32) -> Vec<(f64, f32)> {
    let samples = (duration * rate * 4.0) as usize;
    (0..samples)
        .map(|i| {
            let t = i as f64 / samples as f64;
            let position = start + t * duration;
            let value = 0.5 + 0.5 * depth * (2.0 * std::f64::consts::PI * rate * t).sin() as f32;
            (position, value)
        })
        .collect()
}
