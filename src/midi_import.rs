use anyhow::{Result, anyhow};
use std::{collections::HashMap, path::Path};

use crate::model::clip::MidiNote;

#[derive(Clone)]
pub struct ImportedTrack {
    pub name: String,
    pub notes: Vec<MidiNote>,
    pub program: Option<u8>,
}

pub fn import_midi_file(path: &Path, bpm: f32) -> Result<Vec<ImportedTrack>> {
    let data = std::fs::read(path)?;
    let smf = midly::Smf::parse(&data).map_err(|e| anyhow!("MIDI parse failed: {e}"))?;

    enum TickToBeats {
        Ppqn(f64),                  // ticks_per_beat
        Smpte { tps: f64, k: f64 }, // ticks_per_second = tps; beats = (ticks/tps) * (bpm/60)
    }

    let conv = match smf.header.timing {
        midly::Timing::Metrical(div) => {
            let ppqn = div.as_int() as f64;
            TickToBeats::Ppqn(ppqn)
        }
        midly::Timing::Timecode(fps, subframe) => {
            // ticks per second = fps * subframe
            let tps = (fps.as_f32() as f64 * subframe as f64).max(1.0);
            let k = (bpm as f64) / 60.0;
            TickToBeats::Smpte { tps, k }
        }
    };

    // Helper: convert absolute ticks to beats
    let ticks_to_beats = |t: u64, conv: &TickToBeats| -> f64 {
        match *conv {
            TickToBeats::Ppqn(ppqn) => (t as f64) / ppqn,
            TickToBeats::Smpte { tps, k } => (t as f64 / tps) * k, // seconds * bpm/60
        }
    };

    let mut result_tracks = Vec::new();

    // Iterate all tracks
    for (i, track) in smf.tracks.iter().enumerate() {
        let mut abs_ticks: u64 = 0;
        // Active note starts can overlap; store stacks per (channel, key)
        let mut active: HashMap<(u8, u8), Vec<(u64 /*start ticks*/, u8 /*velocity*/)>> =
            HashMap::new();
        let mut notes: Vec<MidiNote> = Vec::new();
        let mut program: Option<u8> = None;
        let mut track_name: Option<String> = None;

        for ev in track {
            abs_ticks = abs_ticks.saturating_add(ev.delta.as_int() as u64);

            use midly::{MetaMessage, TrackEventKind};
            match ev.kind {
                TrackEventKind::Midi { channel, message } => {
                    match message {
                        midly::MidiMessage::ProgramChange { program: p } => {
                            // Only keep the first program change as the track instrument for now
                            if program.is_none() {
                                program = Some(p.as_int());
                            }
                        }
                        midly::MidiMessage::NoteOn { key, vel } => {
                            let ch = channel.as_int();
                            let k = key.as_int();
                            let v = vel.as_int();
                            if v == 0 {
                                // Treat as NoteOff
                                if let Some(stack) = active.get_mut(&(ch, k)) {
                                    if let Some((start_ticks, start_vel)) = stack.pop() {
                                        let start_beats = ticks_to_beats(start_ticks, &conv);
                                        let end_beats = ticks_to_beats(abs_ticks, &conv);
                                        let dur = (end_beats - start_beats).max(1e-6);
                                        notes.push(MidiNote {
                                            id: 0,
                                            pitch: k,
                                            velocity: start_vel,
                                            start: start_beats,
                                            duration: dur,
                                        });
                                    }
                                }
                            } else {
                                active.entry((ch, k)).or_default().push((abs_ticks, v));
                            }
                        }
                        midly::MidiMessage::NoteOff { key, vel: _ } => {
                            let ch = channel.as_int();
                            let k = key.as_int();
                            if let Some(stack) = active.get_mut(&(ch, k)) {
                                if let Some((start_ticks, start_vel)) = stack.pop() {
                                    let start_beats = ticks_to_beats(start_ticks, &conv);
                                    let end_beats = ticks_to_beats(abs_ticks, &conv);
                                    let dur = (end_beats - start_beats).max(1e-6);
                                    notes.push(MidiNote {
                                        id: 0,
                                        pitch: k,
                                        velocity: start_vel,
                                        start: start_beats,
                                        duration: dur,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                    if let Ok(n) = std::str::from_utf8(name) {
                        track_name = Some(n.to_string());
                    }
                }
                _ => {}
            }
        }

        // Close any hanging notes at end of track
        for ((_ch, _k), stack) in active.into_iter() {
            for (start_ticks, start_vel) in stack {
                let start_beats = ticks_to_beats(start_ticks, &conv);
                let end_beats = (start_beats + 0.25).max(start_beats + 1e-6);
                notes.push(MidiNote {
                    id: 0,
                    pitch: _k,
                    velocity: start_vel,
                    start: start_beats,
                    duration: end_beats - start_beats,
                });
            }
        }

        if !notes.is_empty() {
            // Sort by start time
            notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

            let name = track_name.unwrap_or_else(|| {
                if let Some(p) = program {
                    format!("Track {} (Prog {})", i + 1, p)
                } else {
                    format!("Track {}", i + 1)
                }
            });

            result_tracks.push(ImportedTrack {
                name,
                notes,
                program,
            });
        }
    }

    // If no tracks with notes found, maybe it was a Type 0 file or empty?
    Ok(result_tracks)
}
