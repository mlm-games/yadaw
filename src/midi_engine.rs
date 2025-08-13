use crate::constants::{DEFAULT_GRID_SNAP, MIDI_TIMING_SAMPLE_RATE};
use crate::midi_utils::{MidiNoteUtils, MidiVelocity};
use crate::state::{MidiNote, Pattern};
use crate::time_utils::TimeConverter;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MidiEvent {
    pub timestamp: u64,
    pub channel: u8,
    pub event_type: MidiEventType,
}

#[derive(Debug, Clone)]
pub enum MidiEventType {
    NoteOn { pitch: u8, velocity: u8 },
    NoteOff { pitch: u8 },
    ControlChange { controller: u8, value: u8 },
    ProgramChange { program: u8 },
    PitchBend { value: i16 },
    Aftertouch { pressure: u8 },
    PolyAftertouch { pitch: u8, pressure: u8 },
}

pub struct MidiEngine {
    active_notes: HashMap<(u8, u8), u64>, // (channel, pitch) -> start_time
    input_connections: Vec<MidiInputConnection<()>>,
    output_connections: Vec<MidiOutputConnection>,
    recording_buffer: Vec<MidiEvent>,
    is_recording: bool,
    record_start_time: u64,
    input_filter: MidiFilter,
    echo_to_output: bool,
}

#[derive(Debug, Clone)]
pub struct MidiFilter {
    pub channels: Vec<bool>,      // Which channels to accept (1-16)
    pub note_range: (u8, u8),     // Min and max note
    pub velocity_range: (u8, u8), // Min and max velocity
    pub transpose: i8,
}

impl Default for MidiFilter {
    fn default() -> Self {
        Self {
            channels: vec![true; 16],
            note_range: (0, 127),
            velocity_range: (1, 127),
            transpose: 0,
        }
    }
}

impl MidiEngine {
    pub fn new() -> Self {
        Self {
            active_notes: HashMap::new(),
            input_connections: Vec::new(),
            output_connections: Vec::new(),
            recording_buffer: Vec::new(),
            is_recording: false,
            record_start_time: 0,
            input_filter: MidiFilter::default(),
            echo_to_output: true,
        }
    }

    pub fn connect_input(&mut self, port_index: usize) -> Result<(), Box<dyn std::error::Error>> {
        let midi_in = MidiInput::new("YADAW Input")?;

        if port_index >= midi_in.port_count() {
            return Err("Invalid port index".into());
        }

        // Get all ports and select the one we want
        let ports = midi_in.ports();
        let port = ports.get(port_index).ok_or("Port not found")?;

        let connection = midi_in.connect(
            port,
            "yadaw-input",
            move |stamp, message, _| {
                // Process MIDI input
                println!("MIDI IN: {:?} at {}", message, stamp);
            },
            (),
        )?;

        self.input_connections.push(connection);
        Ok(())
    }

    pub fn connect_output(&mut self, port_index: usize) -> Result<(), Box<dyn std::error::Error>> {
        let midi_out = MidiOutput::new("YADAW Output")?;

        if port_index >= midi_out.port_count() {
            return Err("Invalid port index".into());
        }

        // Get all ports and select the one we want
        let ports = midi_out.ports();
        let port = ports.get(port_index).ok_or("Port not found")?;

        let connection = midi_out.connect(port, "yadaw-output")?;
        self.output_connections.push(connection);
        Ok(())
    }

    pub fn list_input_ports() -> Vec<String> {
        let midi_in = MidiInput::new("YADAW Input Scanner").ok();
        midi_in
            .map(|input| {
                let ports = input.ports();
                ports
                    .iter()
                    .filter_map(|port| input.port_name(port).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn list_output_ports() -> Vec<String> {
        let midi_out = MidiOutput::new("YADAW Output Scanner").ok();
        midi_out
            .map(|output| {
                let ports = output.ports();
                ports
                    .iter()
                    .filter_map(|port| output.port_name(port).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn send_note_on(&mut self, channel: u8, pitch: u8, velocity: u8) {
        let message = [0x90 | (channel & 0x0F), pitch & 0x7F, velocity & 0x7F];
        for conn in &mut self.output_connections {
            let _ = conn.send(&message);
        }

        self.active_notes
            .insert((channel, pitch), self.get_current_time());
    }

    pub fn send_note_off(&mut self, channel: u8, pitch: u8) {
        let message = [0x80 | (channel & 0x0F), pitch & 0x7F, 0];
        for conn in &mut self.output_connections {
            let _ = conn.send(&message);
        }

        self.active_notes.remove(&(channel, pitch));
    }

    pub fn send_control_change(&mut self, channel: u8, controller: u8, value: u8) {
        let message = [0xB0 | (channel & 0x0F), controller & 0x7F, value & 0x7F];
        for conn in &mut self.output_connections {
            let _ = conn.send(&message);
        }
    }

    pub fn send_program_change(&mut self, channel: u8, program: u8) {
        let message = [0xC0 | (channel & 0x0F), program & 0x7F];
        for conn in &mut self.output_connections {
            let _ = conn.send(&message);
        }
    }

    pub fn send_pitch_bend(&mut self, channel: u8, value: i16) {
        let value_u14 = ((value + 8192).clamp(0, 16383)) as u16;
        let lsb = (value_u14 & 0x7F) as u8;
        let msb = ((value_u14 >> 7) & 0x7F) as u8;
        let message = [0xE0 | (channel & 0x0F), lsb, msb];
        for conn in &mut self.output_connections {
            let _ = conn.send(&message);
        }
    }

    pub fn panic(&mut self) {
        // Send all notes off on all channels
        for channel in 0..16 {
            // All notes off
            self.send_control_change(channel, 123, 0);
            // All sound off
            self.send_control_change(channel, 120, 0);
        }

        self.active_notes.clear();
    }

    pub fn start_recording(&mut self) {
        self.recording_buffer.clear();
        self.is_recording = true;
        self.record_start_time = self.get_current_time();
    }

    pub fn stop_recording(&mut self) -> Vec<MidiEvent> {
        self.is_recording = false;
        self.recording_buffer.clone()
    }

    pub fn convert_recording_to_pattern(&self, bpm: f32, quantize: bool) -> Pattern {
        let mut notes = Vec::new();
        let mut note_ons: HashMap<u8, (u64, u8)> = HashMap::new();

        for event in &self.recording_buffer {
            match &event.event_type {
                MidiEventType::NoteOn { pitch, velocity } => {
                    note_ons.insert(*pitch, (event.timestamp, *velocity));
                }
                MidiEventType::NoteOff { pitch } => {
                    if let Some((start_time, velocity)) = note_ons.remove(pitch) {
                        let start_beats = self.timestamp_to_beats(start_time, bpm);
                        let end_beats = self.timestamp_to_beats(event.timestamp, bpm);
                        let duration = end_beats - start_beats;

                        let mut note = MidiNote {
                            pitch: *pitch,
                            velocity,
                            start: start_beats,
                            duration,
                        };

                        if quantize {
                            note.start = (note.start / DEFAULT_GRID_SNAP as f64).round()
                                * DEFAULT_GRID_SNAP as f64;
                        }

                        notes.push(note);
                    }
                }
                _ => {}
            }
        }

        notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

        Pattern {
            name: "Recorded Pattern".to_string(),
            length: notes.last().map(|n| n.start + n.duration).unwrap_or(4.0),
            notes,
        }
    }

    fn timestamp_to_beats(&self, timestamp: u64, bpm: f32) -> f64 {
        let converter = TimeConverter::new(MIDI_TIMING_SAMPLE_RATE, bpm);
        converter.microseconds_to_beats(timestamp - self.record_start_time)
    }

    fn get_current_time(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64
    }

    pub fn process_input_event(&mut self, event: MidiEvent) {
        // Apply input filter
        let channel = event.channel;
        if !self.input_filter.channels[channel as usize] {
            return;
        }

        let mut filtered_event = event.clone();

        match &mut filtered_event.event_type {
            MidiEventType::NoteOn { pitch, velocity } => {
                // Apply note range filter
                if *pitch < self.input_filter.note_range.0
                    || *pitch > self.input_filter.note_range.1
                {
                    return;
                }

                // Apply velocity range filter
                if *velocity < self.input_filter.velocity_range.0
                    || *velocity > self.input_filter.velocity_range.1
                {
                    return;
                }

                // Apply transpose
                let transposed = (*pitch as i16 + self.input_filter.transpose as i16).clamp(0, 127);
                *pitch = transposed as u8;
            }
            MidiEventType::NoteOff { pitch } => {
                // Apply transpose to note off as well
                let transposed = (*pitch as i16 + self.input_filter.transpose as i16).clamp(0, 127);
                *pitch = transposed as u8;
            }
            _ => {}
        }

        // Record if recording
        if self.is_recording {
            self.recording_buffer.push(filtered_event.clone());
        }

        // Echo to output if enabled
        if self.echo_to_output {
            match filtered_event.event_type {
                MidiEventType::NoteOn { pitch, velocity } => {
                    self.send_note_on(channel, pitch, velocity);
                }
                MidiEventType::NoteOff { pitch } => {
                    self.send_note_off(channel, pitch);
                }
                MidiEventType::ControlChange { controller, value } => {
                    self.send_control_change(channel, controller, value);
                }
                _ => {}
            }
        }
    }
}
