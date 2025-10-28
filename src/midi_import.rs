use anyhow::{Result, anyhow};
use std::{collections::HashMap, path::Path};

use crate::model::clip::{MidiClip, MidiNote};

pub fn import_midi_file(path: &Path, bpm: f32) -> Result<MidiClip> {
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

    let mut abs_ticks: u64;
    let mut max_end_beats: f64 = 0.0;

    // Active note starts can overlap; store stacks per (channel, key)
    let mut active: HashMap<(u8, u8), Vec<(u64 /*start ticks*/, u8 /*velocity*/)>> = HashMap::new();
    let mut notes: Vec<MidiNote> = Vec::new();

    // Helper: convert absolute ticks to beats
    let ticks_to_beats = |t: u64, conv: &TickToBeats| -> f64 {
        match *conv {
            TickToBeats::Ppqn(ppqn) => (t as f64) / ppqn,
            TickToBeats::Smpte { tps, k } => (t as f64 / tps) * k, // seconds * bpm/60
        }
    };

    // Iterate all tracks, merging into a single clip
    for track in &smf.tracks {
        abs_ticks = 0;
        for ev in track {
            abs_ticks = abs_ticks.saturating_add(ev.delta.as_int() as u64);

            // Only channel voice events matter for notes
            use midly::{MetaMessage, TrackEventKind};
            match ev.kind {
                TrackEventKind::Midi { channel, message } => {
                    match message {
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
                                        max_end_beats = max_end_beats.max(end_beats);
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
                                    max_end_beats = max_end_beats.max(end_beats);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // Ignore Meta tempo for beat domain (tempo affects seconds, not beats);
                // the app uses constant BPM for playback anyway.
                TrackEventKind::Meta(MetaMessage::Tempo(_)) => {}
                _ => {}
            }
        }
    }

    // Close any hanging notes at end of file (for a minimal duration)
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
            max_end_beats = max_end_beats.max(end_beats);
        }
    }

    // Sort by start time for nicer UI
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

    // Build the clip
    let clip = MidiClip {
        id: 0,
        name: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported MIDI")
            .to_string(),
        start_beat: 0.0,
        length_beats: max_end_beats.max(0.000001),
        notes,
        color: Some((100, 150, 200)),
        // Defaults
        velocity_offset: 0,
        transpose: 0,
        loop_enabled: false,
        content_len_beats: max_end_beats.max(0.000001),
        pattern_id: None,
        quantize_grid: 0.25,
        quantize_strength: 1.0,
        quantize_enabled: false,
        muted: false,
        locked: false,
        groove: None,
        swing: 0.0,
        humanize: 0.0,
        content_offset_beats: 0.0,
    };

    Ok(clip)
}
