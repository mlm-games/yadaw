use std::collections::HashMap;
use std::io::{Cursor, Write};

use anyhow::{Context, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use yadaw_plugin_api::BackendKind;

use super::{
    AUDIO_DIR, DAWPROJECT_VERSION, METADATA_ENTRY, PROJECT_ENTRY, Report, TimeUnit,
    beats_to_seconds, device_id, num, pan_to_normalized, rgb_to_hex, sanitize_name,
    velocity_to_normalized, xml_text,
};
use crate::model::automation::{AutomationPoint, AutomationTarget};
use crate::model::clip::{AudioClip, MidiClip, MidiNote};
use crate::model::plugin::PluginDescriptor;
use crate::model::track::{Send, Track, TrackType};
use crate::project::Project;

const MIN_BPM: f64 = 20.0;
const MAX_BPM: f64 = 666.0;

type Attrs = Vec<(&'static str, String)>;

fn attrs(pairs: impl IntoIterator<Item = (&'static str, String)>) -> Attrs {
    pairs.into_iter().collect()
}

#[derive(Default)]
struct Xml {
    buf: String,
    depth: usize,
    stack: Vec<(&'static str, bool)>,
}

impl Xml {
    fn new() -> Self {
        Xml {
            buf: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n"),
            depth: 0,
            stack: Vec::new(),
        }
    }

    fn open(&mut self, tag: &'static str, a: Attrs) {
        self.touch();
        self.buf.push_str(&"  ".repeat(self.depth));
        self.buf.push('<');
        self.buf.push_str(tag);
        self.write_attrs(&a);
        self.buf.push('>');
        self.buf.push('\n');
        self.stack.push((tag, false));
        self.depth += 1;
    }

    fn close(&mut self) {
        let Some((tag, has_child)) = self.stack.pop() else {
            return;
        };
        self.depth -= 1;
        if has_child {
            self.buf.push_str(&"  ".repeat(self.depth));
            self.buf.push_str("</");
            self.buf.push_str(tag);
            self.buf.push_str(">\n");
        } else {
            self.buf.truncate(self.buf.len() - 2);
            self.buf.push_str("/>\n");
        }
    }

    fn leaf(&mut self, tag: &'static str, a: Attrs) {
        self.touch();
        self.buf.push_str(&"  ".repeat(self.depth));
        self.buf.push('<');
        self.buf.push_str(tag);
        self.write_attrs(&a);
        self.buf.push_str("/>\n");
    }

    fn finish(self) -> String {
        debug_assert!(self.stack.is_empty(), "unbalanced XML elements");
        self.buf
    }

    fn touch(&mut self) {
        if let Some(last) = self.stack.last_mut() {
            last.1 = true;
        }
    }

    fn write_attrs(&mut self, a: &Attrs) {
        for (key, value) in a {
            self.buf.push(' ');
            self.buf.push_str(key);
            self.buf.push_str("=\"");
            self.buf.push_str(&xml_text(value));
            self.buf.push('"');
        }
    }
}

struct Ids(u32);

impl Ids {
    fn next(&mut self) -> String {
        let id = format!("id{}", self.0);
        self.0 += 1;
        id
    }
}

#[derive(Clone, Default)]
struct Automation {
    volume_id: String,
    pan_id: String,
    send_volume_ids: HashMap<u64, String>,
    param_ids: HashMap<(usize, String), String>,
    lanes: Vec<(&'static str, String, Vec<AutomationPoint>)>,
}

struct Ctx<'a> {
    project: &'a Project,
    bpm: f64,
    xml: Xml,
    ids: Ids,
    media: Vec<(String, Vec<u8>)>,
    report: Report,
    track_ids: HashMap<u64, String>,
    channel_ids: HashMap<u64, String>,
    automation: HashMap<u64, Automation>,
}

pub fn export(project: &Project) -> Result<(Vec<u8>, Report)> {
    let mut ctx = Ctx {
        project,
        bpm: if project.bpm > 0.0 {
            f64::from(project.bpm)
        } else {
            120.0
        },
        xml: Xml::new(),
        ids: Ids(0),
        media: Vec::new(),
        report: Report::default(),
        track_ids: HashMap::new(),
        channel_ids: HashMap::new(),
        automation: HashMap::new(),
    };
    ctx.write_project()?;
    Ok((ctx.zip()?, ctx.report))
}

impl Ctx<'_> {
    fn write_project(&mut self) -> Result<()> {
        self.xml.open(
            "Project",
            attrs([("version", DAWPROJECT_VERSION.to_string())]),
        );
        self.xml.leaf(
            "Application",
            attrs([
                ("name", "Yadaw".to_string()),
                ("version", env!("CARGO_PKG_VERSION").to_string()),
            ]),
        );
        self.write_transport();
        self.write_structure();
        self.write_arrangement();
        self.xml.leaf("Scenes", attrs([]));
        self.xml.close();
        Ok(())
    }

    fn zip(&mut self) -> Result<Vec<u8>> {
        let project_xml = std::mem::take(&mut self.xml).finish().into_bytes();
        let mut entries: Vec<(String, Vec<u8>)> = vec![
            (PROJECT_ENTRY.to_string(), project_xml),
            (
                METADATA_ENTRY.to_string(),
                metadata_xml(&self.project.name).into_bytes(),
            ),
        ];
        entries.append(&mut self.media);
        write_zip(entries)
    }

    fn write_transport(&mut self) {
        let numerator = self.project.time_signature.0.max(1);
        let denominator = self.project.time_signature.1.max(1);
        self.xml.open("Transport", attrs([]));
        self.xml.leaf(
            "Tempo",
            attrs([
                ("max", num(MAX_BPM)),
                ("min", num(MIN_BPM)),
                ("unit", "bpm".to_string()),
                ("value", num(self.bpm)),
                ("id", self.ids.next()),
                ("name", "Tempo".to_string()),
            ]),
        );
        self.xml.leaf(
            "TimeSignature",
            attrs([
                ("denominator", denominator.to_string()),
                ("numerator", numerator.to_string()),
                ("id", self.ids.next()),
            ]),
        );
        self.xml.close();
    }

    fn write_structure(&mut self) {
        for track in &self.project.tracks {
            self.track_ids.insert(track.id, self.ids.next());
            self.channel_ids.insert(track.id, self.ids.next());
        }
        let master_track_id = self.ids.next();
        let master_channel_id = self.ids.next();
        let master_volume = f64::from(self.project.master_volume);

        self.xml.open("Structure", attrs([]));
        for track in self.project.tracks.clone() {
            let auto = self.collect_automation(&track);
            self.automation.insert(track.id, auto);
            self.write_track(&track, &master_channel_id);
        }

        self.xml.open(
            "Track",
            attrs([
                ("id", master_track_id),
                ("name", "Master".to_string()),
                ("contentType", "audio notes".to_string()),
                ("loaded", "true".to_string()),
            ]),
        );
        self.xml.open(
            "Channel",
            attrs([
                ("audioChannels", "2".to_string()),
                ("id", master_channel_id),
                ("role", "master".to_string()),
                ("solo", "false".to_string()),
            ]),
        );
        self.write_mute(None, false);
        self.write_pan(None, 0.5);
        self.write_volume(None, master_volume);
        self.xml.close();
        self.xml.close();
        self.xml.close();

        if !self.project.groups.is_empty() {
            self.report
                .note("Track groups have no DAWproject equivalent and were not exported");
        }
        if self.project.loop_enabled {
            self.report
                .note("The loop region has no DAWproject equivalent and was not exported");
        }
        if !self.project.patterns.is_empty() {
            self.report
                .note("Shared MIDI patterns were written into every clip that uses them");
        }

        let audio_clips: usize = self
            .project
            .tracks
            .iter()
            .map(|t| t.audio_clips.len())
            .sum();
        let midi_clips: usize = self.project.tracks.iter().map(|t| t.midi_clips.len()).sum();
        self.report.summary = format!(
            "{} tracks, {} audio clips, {} MIDI clips",
            self.project.tracks.len(),
            audio_clips,
            midi_clips
        );
    }

    fn write_mute(&mut self, _auto: Option<&Automation>, muted: bool) {
        self.xml.leaf(
            "Mute",
            attrs([
                ("value", muted.to_string()),
                ("id", self.ids.next()),
                ("name", "Mute".to_string()),
            ]),
        );
    }

    fn write_pan(&mut self, auto: Option<&Automation>, pan: f64) {
        let id = match auto {
            Some(a) => a.pan_id.clone(),
            None => self.ids.next(),
        };
        self.xml.leaf(
            "Pan",
            attrs([
                ("max", "1".to_string()),
                ("min", "0".to_string()),
                ("unit", "normalized".to_string()),
                ("value", num(pan)),
                ("id", id),
                ("name", "Pan".to_string()),
            ]),
        );
    }

    fn write_volume(&mut self, auto: Option<&Automation>, volume: f64) {
        let id = match auto {
            Some(a) => a.volume_id.clone(),
            None => self.ids.next(),
        };
        self.xml.leaf(
            "Volume",
            attrs([
                ("max", "2".to_string()),
                ("min", "0".to_string()),
                ("unit", "linear".to_string()),
                ("value", num(volume)),
                ("id", id),
                ("name", "Volume".to_string()),
            ]),
        );
    }

    fn write_track(&mut self, track: &Track, master_channel_id: &str) {
        let Some(auto) = self.automation.get(&track.id).cloned() else {
            return;
        };
        let role = if track.track_type == TrackType::Bus {
            "submix"
        } else {
            "regular"
        };

        let mut a = attrs([("id", self.track_ids[&track.id].clone())]);
        a.push(("name", sanitize_name(&track.name)));
        if let Some(color) = track.color {
            a.push(("color", rgb_to_hex(color)));
        }
        let content_type = track_content_type(track);
        if !content_type.is_empty() {
            a.push(("contentType", content_type));
        }
        a.push(("loaded", "true".to_string()));
        self.xml.open("Track", a);
        self.xml.open(
            "Channel",
            attrs([
                ("id", self.channel_ids[&track.id].clone()),
                ("audioChannels", "2".to_string()),
                ("destination", master_channel_id.to_string()),
                ("role", role.to_string()),
                ("solo", track.solo.to_string()),
            ]),
        );

        if !track.plugin_chain.is_empty() {
            self.xml.open("Devices", attrs([]));
            for (index, plugin) in track.plugin_chain.iter().enumerate() {
                self.write_device(track, index, plugin, &auto);
            }
            self.xml.close();
        }

        self.write_mute(Some(&auto), track.muted);
        self.write_pan(Some(&auto), pan_to_normalized(track.pan));

        let routable: Vec<(u64, Send)> = track
            .sends
            .iter()
            .enumerate()
            .filter(|(_, s)| self.channel_ids.contains_key(&s.destination_track))
            .map(|(index, s)| (index as u64, s.clone()))
            .collect();
        if routable.len() < track.sends.len() {
            self.report
                .note("Sends pointing at tracks outside the project were not exported");
        }
        if !routable.is_empty() {
            self.xml.open("Sends", attrs([]));
            for (index, send) in &routable {
                self.write_send(send, *index, &auto);
            }
            self.xml.close();
        }

        self.write_volume(Some(&auto), f64::from(track.volume));

        self.xml.close();
        self.xml.close();
    }

    fn write_send(&mut self, send: &Send, send_index: u64, auto: &Automation) {
        let Some(destination) = self.channel_ids.get(&send.destination_track).cloned() else {
            return;
        };
        self.xml.open(
            "Send",
            attrs([
                ("destination", destination),
                (
                    "type",
                    if send.pre_fader { "pre" } else { "post" }.to_string(),
                ),
                ("id", self.ids.next()),
            ]),
        );
        self.xml.leaf(
            "Enable",
            attrs([
                ("value", (!send.muted).to_string()),
                ("id", self.ids.next()),
                ("name", "On/Off".to_string()),
            ]),
        );
        let volume_id = auto
            .send_volume_ids
            .get(&send_index)
            .cloned()
            .unwrap_or_else(|| self.ids.next());
        self.xml.leaf(
            "Volume",
            attrs([
                ("max", "1".to_string()),
                ("min", "0".to_string()),
                ("unit", "normalized".to_string()),
                ("value", num(f64::from(send.amount))),
                ("id", volume_id),
                ("name", "Volume".to_string()),
            ]),
        );
        self.xml.close();
    }

    fn write_device(
        &mut self,
        track: &Track,
        index: usize,
        plugin: &PluginDescriptor,
        auto: &Automation,
    ) {
        let tag = match plugin.backend {
            BackendKind::Clap => "ClapPlugin",
            BackendKind::Vst3 => "Vst3Plugin",
            BackendKind::Lv2 => {
                self.report.count("LV2 plugins were not exported", 1);
                return;
            }
        };

        let role = if track.track_type == TrackType::Midi && index == 0 {
            "instrument"
        } else {
            "audioFX"
        };

        let mut a = attrs([("id", self.ids.next())]);
        a.push(("deviceID", device_id(&plugin.uri)));
        a.push(("deviceName", sanitize_name(&plugin.name)));
        a.push(("deviceRole", role.to_string()));
        a.push(("loaded", (!plugin.bypass).to_string()));
        self.xml.open(tag, a);

        if !plugin.params.is_empty() {
            let mut names: Vec<&String> = plugin.params.keys().collect();
            names.sort();
            self.xml.open("Parameters", attrs([]));
            for name in names {
                let id = auto
                    .param_ids
                    .get(&(index, (*name).clone()))
                    .cloned()
                    .unwrap_or_else(|| self.ids.next());
                self.xml.leaf(
                    "RealParameter",
                    attrs([
                        ("name", (*name).clone()),
                        ("value", num(f64::from(plugin.params[name.as_str()]))),
                        ("unit", "linear".to_string()),
                        ("id", id),
                    ]),
                );
            }
            self.xml.close();
        }

        self.xml.leaf(
            "Enabled",
            attrs([
                ("value", (!plugin.bypass).to_string()),
                ("id", self.ids.next()),
                ("name", "On/Off".to_string()),
            ]),
        );
        self.xml.close();
    }

    fn write_arrangement(&mut self) {
        self.xml
            .open("Arrangement", attrs([("id", self.ids.next())]));
        self.xml.open(
            "Lanes",
            attrs([("timeUnit", "beats".to_string()), ("id", self.ids.next())]),
        );

        for track in self.project.tracks.clone() {
            let (Some(track_id), Some(auto)) = (
                self.track_ids.get(&track.id).cloned(),
                self.automation.get(&track.id).cloned(),
            ) else {
                continue;
            };

            self.xml.open(
                "Lanes",
                attrs([("track", track_id), ("id", self.ids.next())]),
            );

            if !track.audio_clips.is_empty() || !track.midi_clips.is_empty() {
                self.xml.open("Clips", attrs([("id", self.ids.next())]));
                for clip in &track.audio_clips {
                    if let Err(e) = self.write_audio_clip(clip) {
                        self.report.note(format!("Audio clip skipped: {e}"));
                    }
                }
                for clip in &track.midi_clips {
                    self.write_midi_clip(clip);
                }
                self.xml.close();
            }

            for (unit, target, points) in &auto.lanes {
                self.xml.open(
                    "Points",
                    attrs([("unit", unit.to_string()), ("id", self.ids.next())]),
                );
                self.xml
                    .leaf("Target", attrs([("parameter", target.clone())]));
                for point in points {
                    self.xml.leaf(
                        "RealPoint",
                        attrs([
                            ("time", num(point.beat)),
                            ("value", num(f64::from(point.value))),
                            ("interpolation", "linear".to_string()),
                        ]),
                    );
                }
                self.xml.close();
            }

            self.xml.close();
        }

        self.xml.close();
        self.xml.close();
    }

    fn write_audio_clip(&mut self, clip: &AudioClip) -> Result<()> {
        if clip.samples.is_empty() || clip.sample_rate <= 0.0 || clip.length_beats <= 0.0 {
            self.report.count("empty audio clips were skipped", 1);
            return Ok(());
        }
        if clip.loop_enabled {
            self.report
                .note("Audio clip looping is not expressible in DAWproject and was not exported");
        }

        let (start, end) = source_range(clip, self.bpm);
        let region = &clip.samples[start..end];
        let region_seconds = region.len() as f64 / f64::from(clip.sample_rate);

        let path = format!("{AUDIO_DIR}/clip_{:04}.wav", self.media.len());
        self.media
            .push((path.clone(), wav_bytes(region, clip.sample_rate)?));

        let stretched =
            (region_seconds - beats_to_seconds(clip.length_beats, self.bpm)).abs() > 1e-6;

        let mut a = attrs([
            ("time", num(clip.start_beat)),
            ("duration", num(clip.length_beats)),
            ("fadeTimeUnit", TimeUnit::Beats.name().to_string()),
            ("enable", (!clip.muted).to_string()),
            ("name", sanitize_name(&clip.name)),
        ]);
        if let Some(color) = clip.color {
            a.push(("color", rgb_to_hex(color)));
        }
        if let Some(fade) = fade_attr(clip.crossfade_in, clip.fade_in) {
            a.push(("fadeInTime", fade));
        }
        if let Some(fade) = fade_attr(clip.crossfade_out, clip.fade_out) {
            a.push(("fadeOutTime", fade));
        }

        self.xml.open("Clip", a);
        self.xml.open(
            "Warps",
            attrs([
                ("contentTimeUnit", "seconds".to_string()),
                ("timeUnit", "beats".to_string()),
                ("id", self.ids.next()),
            ]),
        );
        let mut audio = attrs([
            ("id", self.ids.next()),
            ("channels", "1".to_string()),
            ("duration", num(region_seconds)),
            ("sampleRate", clip.sample_rate.round().to_string()),
        ]);
        if stretched {
            audio.push(("algorithm", "stretch".to_string()));
        }
        self.xml.open("Audio", audio);
        self.xml.leaf("File", attrs([("path", path)]));
        self.xml.close();
        self.xml.leaf(
            "Warp",
            attrs([("time", "0".to_string()), ("contentTime", "0".to_string())]),
        );
        self.xml.leaf(
            "Warp",
            attrs([
                ("time", num(clip.length_beats)),
                ("contentTime", num(region_seconds)),
            ]),
        );
        self.xml.close();
        self.xml.close();
        Ok(())
    }

    fn write_midi_clip(&mut self, clip: &MidiClip) {
        let notes: Vec<MidiNote> = clip
            .pattern_id
            .and_then(|pid| self.project.patterns.iter().find(|p| p.id == pid))
            .map(|p| p.notes.clone())
            .unwrap_or_else(|| clip.notes.clone());

        let mut a = attrs([
            ("time", num(clip.start_beat)),
            ("duration", num(clip.length_beats)),
            ("contentTimeUnit", "beats".to_string()),
            ("playStart", num(clip.content_offset_beats)),
            ("loopStart", num(clip.content_offset_beats)),
            (
                "loopEnd",
                num(clip.content_offset_beats + clip.content_len_beats),
            ),
            ("enable", (!clip.muted).to_string()),
            ("name", sanitize_name(&clip.name)),
        ]);
        if let Some(color) = clip.color {
            a.push(("color", rgb_to_hex(color)));
        }

        self.xml.open("Clip", a);
        self.xml.open("Notes", attrs([("id", self.ids.next())]));
        for note in &notes {
            self.xml.leaf(
                "Note",
                attrs([
                    ("time", num(note.start)),
                    ("duration", num(note.duration)),
                    ("channel", "0".to_string()),
                    ("key", note.pitch.to_string()),
                    ("vel", num(velocity_to_normalized(note.velocity))),
                ]),
            );
        }
        self.xml.close();
        self.xml.close();
    }

    fn collect_automation(&mut self, track: &Track) -> Automation {
        let mut auto = Automation {
            volume_id: self.ids.next(),
            pan_id: self.ids.next(),
            ..Default::default()
        };
        for lane in &track.automation_lanes {
            if lane.points.is_empty() {
                continue;
            }
            let points = lane.points.clone();
            match &lane.parameter {
                AutomationTarget::TrackVolume => {
                    auto.lanes
                        .push(("linear", auto.volume_id.clone(), points));
                }
                AutomationTarget::TrackPan => {
                    auto.lanes
                        .push(("normalized", auto.pan_id.clone(), points));
                }
                AutomationTarget::TrackSend(index) => {
                    let id = auto
                        .send_volume_ids
                        .entry(*index)
                        .or_insert_with(|| self.ids.next())
                        .clone();
                    auto.lanes.push(("normalized", id, points));
                }
                AutomationTarget::PluginParam {
                    plugin_id,
                    param_name,
                } => match track
                    .plugin_chain
                    .iter()
                    .position(|p| p.id == *plugin_id)
                {
                    Some(index) => {
                        let id = auto
                            .param_ids
                            .entry((index, param_name.clone()))
                            .or_insert_with(|| self.ids.next())
                            .clone();
                        auto.lanes.push(("linear", id, points));
                    }
                    None => self.report.note(format!(
                        "Automation for unknown plugin parameter '{param_name}' on track '{}' was not exported",
                        track.name
                    )),
                },
            }
        }
        auto
    }
}

/// DAWproject spells a cross-fade as a negative fade time.
fn fade_attr(crossfade: Option<f64>, fade: Option<f64>) -> Option<String> {
    match (crossfade, fade) {
        (Some(value), _) => Some(num(-value.abs())),
        (None, Some(value)) => Some(num(value.abs())),
        (None, None) => None,
    }
}

fn track_content_type(track: &Track) -> String {
    if track.track_type == TrackType::Bus {
        return String::new();
    }
    let has_audio = track.track_type == TrackType::Audio || !track.audio_clips.is_empty();
    let has_notes = track.track_type == TrackType::Midi || !track.midi_clips.is_empty();
    match (has_audio, has_notes) {
        (true, true) => "audio notes".to_string(),
        (false, true) => "notes".to_string(),
        _ => "audio".to_string(),
    }
}

fn source_range(clip: &AudioClip, bpm: f64) -> (usize, usize) {
    let total = clip.samples.len();
    let audio_seconds = total as f64 / f64::from(clip.sample_rate);
    let start_seconds = if clip.warp_mode {
        if clip.length_beats > 0.0 {
            clip.offset_beats * audio_seconds / clip.length_beats
        } else {
            0.0
        }
    } else {
        beats_to_seconds(clip.offset_beats, bpm)
    };
    let start = (start_seconds * f64::from(clip.sample_rate))
        .round()
        .max(0.0)
        .min(total as f64) as usize;
    (start, total)
}

fn wav_bytes(samples: &[f32], sample_rate: f32) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate.round().max(1.0) as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for sample in samples {
            writer.write_sample(sample.clamp(-1.0, 1.0))?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

fn metadata_xml(title: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<MetaData>\n  <Title>{}</Title>\n</MetaData>\n",
        xml_text(title)
    )
}

fn write_zip(entries: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut cursor);
        for (name, data) in &entries {
            writer
                .start_file(name.as_str(), options)
                .with_context(|| format!("zip entry {name}"))?;
            writer.write_all(data)?;
        }
        writer.finish()?;
    }
    Ok(cursor.into_inner())
}
