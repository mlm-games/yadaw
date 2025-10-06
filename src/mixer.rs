use crate::audio_utils::{calculate_stereo_gains, soft_clip};
use crate::constants::DEFAULT_TRACK_VOLUME;

#[derive(Debug, Clone, Copy)]
pub struct ChannelStrip {
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub phase_invert: bool,
    pub input_gain: f32,
    pub output_gain: f32,
}

impl Default for ChannelStrip {
    fn default() -> Self {
        Self {
            gain: DEFAULT_TRACK_VOLUME,
            pan: 0.0,
            mute: false,
            solo: false,
            phase_invert: false,
            input_gain: 1.0,
            output_gain: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Bus {
    pub id: usize,
    pub name: String,
    pub strip: ChannelStrip,
    pub sends: Vec<Send>,
    pub input_tracks: Vec<usize>,
    pub output_bus: Option<usize>, // None means master
}

#[derive(Debug, Clone)]
pub struct Send {
    pub destination: SendDestination,
    pub amount: f32,
    pub pre_fader: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub enum SendDestination {
    Bus(usize),
    External(usize), // Hardware output
}

pub struct MixerEngine {
    buses: Vec<Bus>,
    master_strip: ChannelStrip,
    solo_bus: Option<usize>, // AFL/PFL bus
    headphone_mix: HeadphoneMix,
}

#[derive(Debug, Clone)]
pub struct HeadphoneMix {
    pub enabled: bool,
    pub source: HeadphoneSource,
    pub volume: f32,
}

#[derive(Debug, Clone)]
pub enum HeadphoneSource {
    Master,
    Cue(usize),                // Bus ID
    Custom(Vec<(usize, f32)>), // Track IDs with amounts
}

impl MixerEngine {
    pub fn new() -> Self {
        Self {
            buses: Vec::new(),
            master_strip: ChannelStrip::default(),
            solo_bus: None,
            headphone_mix: HeadphoneMix {
                enabled: false,
                source: HeadphoneSource::Master,
                volume: 0.7,
            },
        }
    }

    pub fn create_bus(&mut self, name: String) -> usize {
        let id = self.buses.len();
        self.buses.push(Bus {
            id,
            name,
            strip: ChannelStrip::default(),
            sends: Vec::new(),
            input_tracks: Vec::new(),
            output_bus: None,
        });
        id
    }

    pub fn route_track_to_bus(&mut self, track_id: usize, bus_id: usize) {
        if let Some(bus) = self.buses.get_mut(bus_id) {
            if !bus.input_tracks.contains(&track_id) {
                bus.input_tracks.push(track_id);
            }
        }
    }

    pub fn process_mix(
        &self,
        track_outputs: &[(f32, f32)], // (left, right) for each track
        track_strips: &[ChannelStrip],
        bus_buffers: &mut Vec<(f32, f32)>,
        master_out: &mut (f32, f32),
    ) {
        // Clear bus buffers
        bus_buffers.clear();
        bus_buffers.resize(self.buses.len(), (0.0, 0.0));

        // Process each bus
        for (bus_idx, bus) in self.buses.iter().enumerate() {
            let mut bus_sum = (0.0f32, 0.0f32);

            // Sum input tracks
            for &track_id in &bus.input_tracks {
                if track_id < track_outputs.len() {
                    let (left, right) = track_outputs[track_id];
                    let strip = &track_strips[track_id];

                    if !strip.mute {
                        let (gain_l, gain_r) = calculate_stereo_gains(strip.gain, strip.pan);
                        bus_sum.0 += left * gain_l * strip.output_gain;
                        bus_sum.1 += right * gain_r * strip.output_gain;
                    }
                }
            }

            // Apply bus strip processing
            if !bus.strip.mute {
                let (gain_l, gain_r) = calculate_stereo_gains(bus.strip.gain, bus.strip.pan);
                bus_buffers[bus_idx] = (
                    bus_sum.0 * gain_l * bus.strip.output_gain,
                    bus_sum.1 * gain_r * bus.strip.output_gain,
                );
            }
        }

        // Sum everything to master
        let mut master_sum = (0.0f32, 0.0f32);

        // Add tracks routed directly to master
        for (track_id, &(left, right)) in track_outputs.iter().enumerate() {
            let strip = &track_strips[track_id];

            // Check if track is routed to a bus
            let routed_to_bus = self
                .buses
                .iter()
                .any(|bus| bus.input_tracks.contains(&track_id));

            if !routed_to_bus && !strip.mute {
                let (gain_l, gain_r) = calculate_stereo_gains(strip.gain, strip.pan);
                master_sum.0 += left * gain_l * strip.output_gain;
                master_sum.1 += right * gain_r * strip.output_gain;
            }
        }

        // Add buses to master
        for (bus_idx, bus) in self.buses.iter().enumerate() {
            if bus.output_bus.is_none() {
                // This bus outputs to master
                let (left, right) = bus_buffers[bus_idx];
                master_sum.0 += left;
                master_sum.1 += right;
            }
        }

        // Apply master strip
        let (master_gain_l, master_gain_r) =
            calculate_stereo_gains(self.master_strip.gain, self.master_strip.pan);

        *master_out = (
            master_sum.0 * master_gain_l * self.master_strip.output_gain,
            master_sum.1 * master_gain_r * self.master_strip.output_gain,
        );

        // Apply limiting to prevent clipping
        master_out.0 = soft_clip(master_out.0);
        master_out.1 = soft_clip(master_out.1);
    }
}

// EQ and Dynamics processing structures
#[derive(Debug, Clone)]
pub struct EQBand {
    pub frequency: f32,
    pub gain: f32,
    pub q: f32,
    pub band_type: EQBandType,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum EQBandType {
    HighPass,
    LowPass,
    Bell,
    HighShelf,
    LowShelf,
    Notch,
}

#[derive(Debug, Clone)]
pub struct Compressor {
    pub threshold: f32,   // dB
    pub ratio: f32,       // e.g., 4.0 for 4:1
    pub attack: f32,      // ms
    pub release: f32,     // ms
    pub makeup_gain: f32, // dB
    pub knee: f32,        // dB
    pub enabled: bool,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            threshold: -20.0,
            ratio: 4.0,
            attack: 10.0,
            release: 100.0,
            makeup_gain: 0.0,
            knee: 2.0,
            enabled: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Gate {
    pub threshold: f32, // dB
    pub range: f32,     // dB
    pub attack: f32,    // ms
    pub hold: f32,      // ms
    pub release: f32,   // ms
    pub enabled: bool,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            threshold: -40.0,
            range: -60.0,
            attack: 0.1,
            hold: 10.0,
            release: 100.0,
            enabled: false,
        }
    }
}
