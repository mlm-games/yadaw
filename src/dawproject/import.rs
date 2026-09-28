use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use roxmltree::{Document, Node};

use yadaw_plugin_api::BackendKind;

use super::{
    METADATA_ENTRY, PROJECT_ENTRY, PluginResolver, Report, TimeUnit, hex_to_rgb,
    normalized_to_velocity, num, parse_bool, parse_f64, plugin_device_id, sanitize_name,
    seconds_to_beats,
};
use crate::audio_import::import_audio_data;
use crate::idgen;
use crate::model::automation::{AutomationLane, AutomationMode, AutomationPoint, AutomationTarget};
use crate::model::clip::{MidiClip, MidiNote, MidiPattern};
use crate::model::group::TrackGroup;
use crate::model::marker::Marker;
use crate::model::plugin::PluginDescriptor;
use crate::model::track::{Send, Track, TrackType};
use crate::project::{AppState, PROJECT_VERSION, Project};

const DEFAULT_BPM: f64 = 120.0;

type El<'a, 'i> = Node<'a, 'i>;

fn children<'a, 'i>(node: El<'a, 'i>) -> impl Iterator<Item = El<'a, 'i>> {
    node.children().filter(|n| n.is_element())
}

fn child<'a, 'i>(node: El<'a, 'i>, tag: &str) -> Option<El<'a, 'i>> {
    children(node).find(|n| n.has_tag_name(tag))
}

fn attr<'a, 'i>(node: El<'a, 'i>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

fn find_descendant<'a, 'i>(node: El<'a, 'i>, tag: &str) -> Option<El<'a, 'i>> {
    node.descendants()
        .find(|n| n.is_element() && n.has_tag_name(tag))
}

fn num_attr(node: El, name: &str) -> Option<f64> {
    parse_f64(node.attribute(name))
}

fn scope_unit(node: El, name: &str, inherited: TimeUnit) -> TimeUnit {
    match node.attribute(name) {
        Some(value) => TimeUnit::parse(Some(value)),
        None => inherited,
    }
}

struct Container {
    archive: zip::ZipArchive<Cursor<Vec<u8>>>,
    names: Vec<String>,
    cache: HashMap<String, Option<Vec<u8>>>,
}

impl Container {
    fn open(bytes: &[u8]) -> Result<Self> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec()))
            .map_err(|e| anyhow!("not a ZIP container: {e}"))?;
        let mut names = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            if let Ok(file) = archive.by_index(index) {
                names.push(file.name().to_string());
            }
        }
        Ok(Container {
            archive,
            names,
            cache: HashMap::new(),
        })
    }

    fn index_of(&self, path: &str) -> Option<usize> {
        let want = path.replace('\\', "/");
        let base = want.rsplit('/').next().unwrap_or(&want);
        let suffix = format!("/{want}");
        self.names
            .iter()
            .position(|n| n == &want)
            .or_else(|| self.names.iter().position(|n| n.ends_with(&suffix)))
            .or_else(|| {
                self.names
                    .iter()
                    .position(|n| n.rsplit('/').next() == Some(base))
            })
    }

    fn read(&mut self, path: &str) -> Option<&[u8]> {
        if !self.cache.contains_key(path) {
            let mut found = None;
            if let Some(index) = self.index_of(path) {
                let mut buf = Vec::new();
                let ok = self
                    .archive
                    .by_index(index)
                    .is_ok_and(|mut file| file.read_to_end(&mut buf).is_ok());
                if ok {
                    found = Some(buf);
                }
            }
            self.cache.insert(path.to_string(), found);
        }
        self.cache.get(path).and_then(|v| v.as_deref())
    }

    fn read_text(&mut self, path: &str) -> Option<String> {
        let data = self.read(path)?;
        Some(String::from_utf8_lossy(data).into_owned())
    }
}

enum ParamTarget {
    Volume,
    Pan,
    Send(u64),
    Plugin(u64, String),
}

pub fn import(bytes: &[u8], resolve: &PluginResolver<'_>) -> Result<(Project, Report)> {
    let mut report = Report::default();
    let mut container = Container::open(bytes)?;

    let title = container
        .read_text(METADATA_ENTRY)
        .and_then(|xml| metadata_title(&xml));
    let xml = container
        .read_text(PROJECT_ENTRY)
        .ok_or_else(|| anyhow!("{PROJECT_ENTRY} is missing from the container"))?;
    let doc = Document::parse(&xml).map_err(|e| anyhow!("invalid {PROJECT_ENTRY}: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("Project") {
        return Err(anyhow!(
            "{PROJECT_ENTRY} root element is <{}>, expected <Project>",
            root.tag_name().name()
        ));
    }

    let mut ids: HashMap<String, El> = HashMap::new();
    for node in root.descendants() {
        if node.is_element()
            && let Some(id) = node.attribute("id")
        {
            ids.entry(id.to_string()).or_insert(node);
        }
    }

    let transport = child(root, "Transport");
    let tempo = transport.and_then(|t| child(t, "Tempo"));
    let bpm = match tempo {
        Some(node) => match node.attribute("unit") {
            None | Some("bpm") => num_attr(node, "value")
                .filter(|v| *v > 0.0)
                .unwrap_or(DEFAULT_BPM),
            Some(unit) => {
                report.note(format!(
                    "Tempo is expressed in '{unit}', which yadaw cannot represent; 120 BPM was used"
                ));
                DEFAULT_BPM
            }
        },
        None => DEFAULT_BPM,
    };

    let time_signature = transport
        .and_then(|t| child(t, "TimeSignature"))
        .and_then(|node| {
            Some((
                num_attr(node, "numerator")?.round().max(1.0) as i32,
                num_attr(node, "denominator")?.round().max(1.0) as i32,
            ))
        })
        .unwrap_or((4, 4));

    if let Some(arrangement) = child(root, "Arrangement") {
        for tag in ["TempoAutomation", "TimeSignatureAutomation"] {
            if let Some(points) = child(arrangement, tag)
                && children(points).next().is_some()
            {
                report.note(format!(
                    "<{tag}> is not supported because yadaw has no tempo map; the project was imported at a constant {bpm} BPM"
                ));
            }
        }
    }

    if let Some(scenes) = child(root, "Scenes")
        && children(scenes).next().is_some()
    {
        report.note("Clip launcher scenes have no yadaw equivalent and were skipped");
    }

    let structure = child(root, "Structure");
    let mut master_volume = AppState::default().master_volume;
    let mut channel_to_track: HashMap<String, u64> = HashMap::new();
    let mut raw_tracks: Vec<RawTrack> = Vec::new();
    let mut groups: Vec<TrackGroup> = Vec::new();

    if let Some(structure) = structure {
        read_structure(
            structure,
            None,
            &mut raw_tracks,
            &mut groups,
            &mut master_volume,
            &mut channel_to_track,
            &mut report,
        );
    }

    for raw in &raw_tracks {
        if let Some(id) = child(raw.node, "Channel").and_then(|c| attr(c, "id")) {
            channel_to_track.insert(id.to_string(), raw.track_id);
        }
    }

    let lanes = collect_lanes(root);

    let mut tracks: Vec<Track> = Vec::new();
    for raw in raw_tracks {
        let Some(channel) = child(raw.node, "Channel") else {
            continue;
        };
        if let Some(id) = attr(channel, "id") {
            channel_to_track
                .entry(id.to_string())
                .or_insert(raw.track_id);
        }

        let mut track = read_track(raw.node, channel, raw.track_id, raw.group_id);
        let mut params: HashMap<String, ParamTarget> = HashMap::new();
        read_channel_params(channel, &mut track, &mut params, resolve, &mut report);
        read_sends(
            channel,
            &mut track,
            &channel_to_track,
            &mut params,
            &mut report,
        );

        if let Some(xml_id) = raw.xml_id.as_deref()
            && let Some(lanes) = lanes.get(xml_id)
        {
            for lane in lanes {
                read_lane(
                    lane,
                    &mut track,
                    &mut container,
                    &ids,
                    bpm,
                    &params,
                    &mut report,
                );
            }
        }

        tracks.push(track);
    }

    let patterns = share_repeated_notes(&mut tracks);

    let mut markers = Vec::new();
    if let Some(arrangement) = child(root, "Arrangement") {
        read_markers(arrangement, TimeUnit::Beats, bpm, &mut markers);
    }
    markers.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    // A file may spell the arrangement's markers both ways; keep one of each.
    markers.dedup_by(|a, b| a.beat == b.beat && a.name == b.name);

    let audio_clips: usize = tracks.iter().map(|t| t.audio_clips.len()).sum();
    let midi_clips: usize = tracks.iter().map(|t| t.midi_clips.len()).sum();
    report.summary = format!(
        "{} tracks, {} audio clips, {} MIDI clips",
        tracks.len(),
        audio_clips,
        midi_clips
    );

    let now = chrono::Utc::now();
    let project = Project {
        version: PROJECT_VERSION.to_string(),
        name: title.unwrap_or_else(|| "Imported DAWproject".to_string()),
        tracks,
        patterns,
        groups,
        markers,
        bpm: bpm as f32,
        tempo_map: Vec::new(),
        time_signature,
        sample_rate: crate::constants::DEFAULT_SAMPLE_RATE as f32,
        master_volume,
        loop_start: 0.0,
        loop_end: crate::constants::DEFAULT_LOOP_LEN,
        loop_enabled: false,
        created_at: now,
        modified_at: now,
    };

    Ok((project, report))
}

struct RawTrack<'a, 'i> {
    xml_id: Option<String>,
    track_id: u64,
    group_id: Option<u64>,
    node: El<'a, 'i>,
}

fn read_master_channel(
    channel: El,
    master_volume: &mut f32,
    channel_to_track: &mut HashMap<String, u64>,
) {
    if attr(channel, "role") != Some("master") {
        return;
    }
    if let Some(volume) = child(channel, "Volume").and_then(|v| num_attr(v, "value")) {
        *master_volume = volume.clamp(0.0, 2.0) as f32;
    }
    if let Some(id) = attr(channel, "id") {
        channel_to_track.insert(id.to_string(), u64::MAX);
    }
}

fn read_structure<'a, 'i>(
    parent: El<'a, 'i>,
    parent_group: Option<u64>,
    raw_tracks: &mut Vec<RawTrack<'a, 'i>>,
    groups: &mut Vec<TrackGroup>,
    master_volume: &mut f32,
    channel_to_track: &mut HashMap<String, u64>,
    report: &mut Report,
) {
    for node in children(parent) {
        if node.has_tag_name("Channel") {
            match attr(node, "role") {
                Some("master") => read_master_channel(node, master_volume, channel_to_track),
                Some("vca") => report.note(
                    "Mixer group (VCA) channels have no yadaw equivalent; their child tracks were imported without grouping",
                ),
                _ => {}
            }
            continue;
        }
        if !node.has_tag_name("Track") {
            continue;
        }

        if content_type(node)
            .split_ascii_whitespace()
            .any(|t| t == "tracks")
        {
            let group_id = read_group(node, parent_group, groups, report);
            read_structure(
                node,
                Some(group_id),
                raw_tracks,
                groups,
                master_volume,
                channel_to_track,
                report,
            );
            continue;
        }

        let Some(channel) = child(node, "Channel") else {
            continue;
        };
        if attr(channel, "role") == Some("master") {
            read_master_channel(channel, master_volume, channel_to_track);
            continue;
        }
        if attr(channel, "role") == Some("vca") {
            report.note(
                "Mixer group (VCA) channels have no yadaw equivalent; their child tracks were imported without grouping",
            );
            continue;
        }

        raw_tracks.push(RawTrack {
            xml_id: attr(node, "id").map(str::to_owned),
            track_id: idgen::next(),
            group_id: parent_group,
            node,
        });
    }
}

fn read_group(
    node: El,
    parent_group: Option<u64>,
    groups: &mut Vec<TrackGroup>,
    report: &mut Report,
) -> u64 {
    let id = idgen::next();
    let name = attr(node, "name")
        .map(sanitize_name)
        .unwrap_or_else(|| "Group".to_string());
    let mut group = TrackGroup::new(id, name.clone());
    group.parent_id = parent_group;
    if let Some(color) = attr(node, "color").and_then(hex_to_rgb) {
        group.color = color;
    }
    if let Some(channel) = child(node, "Channel") {
        if let Some(v) = child(channel, "Volume").and_then(|v| num_attr(v, "value")) {
            group.volume = v.clamp(0.0, 2.0) as f32;
        }
        group.muted = child(channel, "Mute")
            .and_then(|m| parse_bool(attr(m, "value")))
            .unwrap_or(false);
        group.solo = parse_bool(attr(channel, "solo")).unwrap_or(false);
        // A folder channel is a mixer group (audio routed through it) or a VCA
        // (controls channels). yadaw groups are always the latter.
        if attr(channel, "role") == Some("submix") {
            report.note(format!(
                "Group track '{name}' is a mixer group, which routes audio through it; yadaw imported its level, mute and solo as a VCA-style control instead"
            ));
        }
    }
    groups.push(group);
    id
}

fn content_type<'a, 'i>(node: El<'a, 'i>) -> &'a str {
    attr(node, "contentType").unwrap_or_default()
}

struct Lane<'a, 'i> {
    node: El<'a, 'i>,
    unit: TimeUnit,
}

fn collect_lanes<'a, 'i>(root: El<'a, 'i>) -> HashMap<String, Vec<Lane<'a, 'i>>> {
    let mut map: HashMap<String, Vec<Lane>> = HashMap::new();
    visit_lanes(root, TimeUnit::Beats, None, &mut map);
    map
}

fn visit_lanes<'a, 'i>(
    node: El<'a, 'i>,
    inherited: TimeUnit,
    owner: Option<&'a str>,
    map: &mut HashMap<String, Vec<Lane<'a, 'i>>>,
) {
    let (unit, owner) = if node.has_tag_name("Lanes") {
        let unit = scope_unit(node, "timeUnit", inherited);
        let owner = node.attribute("track").or(owner);
        if let Some(owner) = owner {
            map.entry(owner.to_string())
                .or_default()
                .push(Lane { node, unit });
        }
        (unit, owner)
    } else {
        (inherited, owner)
    };

    for child in children(node) {
        visit_lanes(child, unit, owner, map);
    }
}

fn read_track(node: El, channel: El, id: u64, group_id: Option<u64>) -> Track {
    let track_type = if attr(channel, "role") == Some("submix") {
        TrackType::Bus
    } else if content_type(node)
        .split_ascii_whitespace()
        .any(|t| t == "notes")
    {
        TrackType::Midi
    } else {
        TrackType::Audio
    };

    Track {
        id,
        name: attr(node, "name")
            .map(sanitize_name)
            .unwrap_or_else(|| "Track".to_string()),
        color: attr(node, "color").and_then(hex_to_rgb),
        group_id,
        track_type,
        solo: parse_bool(attr(channel, "solo")).unwrap_or(false),
        ..Track::default()
    }
}

fn read_channel_params(
    channel: El,
    track: &mut Track,
    params: &mut HashMap<String, ParamTarget>,
    resolve: &PluginResolver<'_>,
    report: &mut Report,
) {
    track.muted = child(channel, "Mute")
        .and_then(|m| parse_bool(attr(m, "value")))
        .unwrap_or(false);
    track.pan = child(channel, "Pan")
        .and_then(|p| num_attr(p, "value"))
        .map(|v| ((v * 2.0 - 1.0) as f32).clamp(-1.0, 1.0))
        .unwrap_or(track.pan);
    if let Some(volume) = child(channel, "Volume").and_then(|v| num_attr(v, "value")) {
        track.volume = volume.clamp(0.0, 4.0) as f32;
    }
    if let Some(id) = child(channel, "Volume").and_then(|v| attr(v, "id")) {
        params.insert(id.to_string(), ParamTarget::Volume);
    }
    if let Some(id) = child(channel, "Pan").and_then(|v| attr(v, "id")) {
        params.insert(id.to_string(), ParamTarget::Pan);
    }
    read_devices(channel, track, params, resolve, report);
}

fn read_devices(
    channel: El,
    track: &mut Track,
    params: &mut HashMap<String, ParamTarget>,
    resolve: &PluginResolver<'_>,
    report: &mut Report,
) {
    let Some(devices) = child(channel, "Devices") else {
        return;
    };

    for node in children(devices) {
        let backend = match node.tag_name().name() {
            "ClapPlugin" => BackendKind::Clap,
            "Vst3Plugin" => BackendKind::Vst3,
            other => {
                report.note(format!(
                    "<{other}> devices have no yadaw equivalent and were skipped"
                ));
                continue;
            }
        };

        let device_name = attr(node, "deviceName")
            .map(sanitize_name)
            .unwrap_or_else(|| "Plugin".to_string());
        let device_id = attr(node, "deviceID").unwrap_or_default();
        let uri = resolve(backend, device_id, &device_name)
            .or_else(|| (!device_id.is_empty()).then(|| device_id.to_string()))
            .unwrap_or_else(|| device_name.clone());

        if !uri_has_backend(&uri, backend) {
            report.note(format!(
                "Plugin '{device_name}' could not be matched to an installed plugin; it was added unresolved"
            ));
        }

        let enabled = child(node, "Enabled")
            .and_then(|e| parse_bool(attr(e, "value")))
            .unwrap_or(true);
        let loaded = parse_bool(attr(node, "loaded")).unwrap_or(true);

        let id = idgen::next();
        let mut values: HashMap<String, f32> = HashMap::new();
        if let Some(list) = child(node, "Parameters") {
            for param in children(list) {
                let Some(name) = attr(param, "name") else {
                    continue;
                };
                if let Some(value) = read_param_value(param) {
                    values.insert(name.to_string(), value);
                }
                if let Some(param_id) = attr(param, "id") {
                    params.insert(
                        param_id.to_string(),
                        ParamTarget::Plugin(id, name.to_string()),
                    );
                }
            }
        }

        track.plugin_chain.push(PluginDescriptor {
            id,
            uri,
            name: device_name,
            backend,
            bypass: !(enabled && loaded),
            has_editor: false,
            params: values,
            preset_name: None,
            custom_name: None,
        });
    }
}

fn uri_has_backend(uri: &str, backend: BackendKind) -> bool {
    match backend {
        BackendKind::Clap => plugin_device_id(uri).is_some(),
        BackendKind::Vst3 => !uri.contains('#'),
        BackendKind::Lv2 => false,
    }
}
fn read_param_value(node: El) -> Option<f32> {
    let value = num_attr(node, "value")?;
    let result = match node.tag_name().name() {
        "BoolParameter" => {
            if parse_bool(attr(node, "value")).unwrap_or(value != 0.0) {
                1.0
            } else {
                0.0
            }
        }
        _ => value,
    };
    Some(result.clamp(-1.0e9, 1.0e9) as f32)
}

fn read_sends(
    channel: El,
    track: &mut Track,
    channel_to_track: &HashMap<String, u64>,
    params: &mut HashMap<String, ParamTarget>,
    report: &mut Report,
) {
    let Some(sends) = child(channel, "Sends") else {
        return;
    };
    for node in children(sends) {
        if !node.has_tag_name("Send") {
            continue;
        }
        let Some(destination) = attr(node, "destination") else {
            continue;
        };
        match channel_to_track.get(destination) {
            Some(&u64::MAX) | None => {
                report.note(
                    "Sends routed to the master channel have no yadaw equivalent and were skipped",
                );
                continue;
            }
            Some(&target) => {
                let amount = child(node, "Volume")
                    .and_then(|v| num_attr(v, "value"))
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0) as f32;
                let muted = child(node, "Enable")
                    .and_then(|e| parse_bool(attr(e, "value")))
                    .map(|enabled| !enabled)
                    .unwrap_or(false);
                track.sends.push(Send {
                    destination_track: target,
                    amount,
                    pre_fader: attr(node, "type") == Some("pre"),
                    muted,
                });
                if let Some(id) = child(node, "Volume").and_then(|v| attr(v, "id")) {
                    params.insert(
                        id.to_string(),
                        ParamTarget::Send(track.sends.len() as u64 - 1),
                    );
                }
            }
        }
    }
}

fn read_lane(
    lane: &Lane,
    track: &mut Track,
    container: &mut Container,
    ids: &HashMap<String, El>,
    bpm: f64,
    params: &HashMap<String, ParamTarget>,
    report: &mut Report,
) {
    for node in children(lane.node) {
        match node.tag_name().name() {
            "Lanes" => {}
            "Clips" => {
                for clip in children(node) {
                    if clip.has_tag_name("Clip") {
                        read_clip(clip, lane.unit, container, ids, bpm, track, report);
                    }
                }
            }
            "Notes" => read_lane_notes(node, lane.unit, bpm, track, report),
            "Points" => read_points(node, lane.unit, bpm, params, track, report),
            "markers" => {
                report.note("Markers scoped to a single track lane were skipped");
            }
            "Video" | "Warps" => {
                report.note("Video timelines have no yadaw equivalent and were skipped");
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_clip(
    node: El,
    unit: TimeUnit,
    container: &mut Container,
    ids: &HashMap<String, El>,
    bpm: f64,
    track: &mut Track,
    report: &mut Report,
) {
    let content = attr(node, "reference")
        .and_then(|r| ids.get(r))
        .copied()
        .unwrap_or(node);

    if child(content, "markers").is_some() {
        report.note("Markers attached to a clip were skipped");
    }

    let content_unit = TimeUnit::parse(attr(node, "contentTimeUnit"));
    let time = num_attr(node, "time").unwrap_or(0.0);
    let start_beat = unit.to_beats(time, bpm);
    let play_start = num_attr(node, "playStart").unwrap_or(0.0);
    let play_stop = num_attr(node, "playStop");
    let duration = num_attr(node, "duration")
        .or_else(|| play_stop.map(|stop| content_unit.to_beats(stop - play_start, bpm)));
    let length_beats = duration.map(|d| unit.to_beats(d, bpm)).unwrap_or(0.0);
    let name = attr(node, "name").map(sanitize_name);
    let color = attr(node, "color").and_then(hex_to_rgb);
    let muted = parse_bool(attr(node, "enable")) == Some(false);

    let loop_start = num_attr(node, "loopStart");
    let loop_end = num_attr(node, "loopEnd");

    if let Some(notes_node) = find_descendant(content, "Notes") {
        let notes_unit = scope_unit(notes_node, "timeUnit", content_unit);
        let notes = read_notes(notes_node, notes_unit, bpm, report);
        if notes.is_empty() {
            return;
        }
        let content_offset = loop_start
            .or(Some(play_start))
            .map(|v| content_unit.to_beats(v, bpm))
            .unwrap_or(0.0);
        let content_len = match (loop_start, loop_end) {
            (Some(start), Some(end)) => (end - start).max(0.0),
            _ => length_beats,
        };
        let content_len = content_unit.to_beats(content_len, bpm);
        track.midi_clips.push(MidiClip {
            id: idgen::next(),
            name: name.unwrap_or_else(|| "MIDI Clip".to_string()),
            start_beat,
            length_beats: if length_beats > 0.0 {
                length_beats
            } else {
                content_len
            },
            content_len_beats: content_len,
            content_offset_beats: content_offset,
            loop_enabled: content_len > 0.0 && content_len < length_beats - 1.0e-6,
            notes,
            color,
            muted,
            ..MidiClip::default()
        });
        return;
    }

    if let Some(audio) = find_descendant(content, "Audio") {
        read_audio_clip(
            node,
            audio,
            unit,
            content_unit,
            start_beat,
            length_beats,
            name,
            color,
            muted,
            container,
            bpm,
            track,
            report,
        );
    }
}

/// Collect the arrangement's markers. An arrangement declares them as
/// `<Arrangement><Markers>`, and a `Lanes` scope may also carry the global
/// lowercase `<markers>`, so both spellings are read. Yadaw markers belong to the
/// project rather than to a track, so a `Lanes` scope that names a track is left
/// alone; its markers are reported by `read_lane` instead.
fn read_markers<'a, 'i>(scope: El<'a, 'i>, inherited: TimeUnit, bpm: f64, out: &mut Vec<Marker>) {
    let unit = scope_unit(scope, "timeUnit", inherited);
    for node in children(scope) {
        match node.tag_name().name() {
            "markers" | "Markers" => {
                let marker_unit = scope_unit(node, "timeUnit", unit);
                for entry in children(node) {
                    if !entry.has_tag_name("Marker") {
                        continue;
                    }
                    let Some(time) = num_attr(entry, "time") else {
                        continue;
                    };
                    let name = attr(entry, "name")
                        .map(sanitize_name)
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| format!("Marker {}", out.len() + 1));
                    out.push(Marker {
                        id: idgen::next(),
                        beat: marker_unit.to_beats(time, bpm).max(0.0),
                        name,
                        color: attr(entry, "color").and_then(hex_to_rgb),
                        comment: attr(entry, "comment").map(sanitize_name),
                    });
                }
            }
            "Lanes" if node.attribute("track").is_none() => {
                read_markers(node, unit, bpm, out);
            }
            _ => {}
        }
    }
}

fn read_lane_notes(node: El, unit: TimeUnit, bpm: f64, track: &mut Track, report: &mut Report) {
    let unit = scope_unit(node, "timeUnit", unit);
    let notes = read_notes(node, unit, bpm, report);
    if notes.is_empty() {
        return;
    }
    let end = notes
        .iter()
        .map(|n| n.start + n.duration)
        .fold(0.0_f64, f64::max);
    track.midi_clips.push(MidiClip {
        id: idgen::next(),
        name: attr(node, "name")
            .map(sanitize_name)
            .unwrap_or_else(|| "MIDI Clip".to_string()),
        start_beat: 0.0,
        length_beats: end,
        content_len_beats: end,
        notes,
        ..MidiClip::default()
    });
}

fn read_notes(node: El, unit: TimeUnit, bpm: f64, report: &mut Report) -> Vec<MidiNote> {
    let mut notes = Vec::new();
    let mut off_channel = false;
    for note in children(node) {
        if !note.has_tag_name("Note") {
            continue;
        }
        if num_attr(note, "channel").unwrap_or(0.0) != 0.0 {
            off_channel = true;
        }
        let Some(key) = num_attr(note, "key") else {
            continue;
        };
        notes.push(MidiNote {
            id: idgen::next(),
            pitch: key.round().clamp(0.0, 127.0) as u8,
            velocity: normalized_to_velocity(num_attr(note, "vel").unwrap_or(0.8)),
            start: unit.to_beats(num_attr(note, "time").unwrap_or(0.0), bpm),
            duration: unit.to_beats(num_attr(note, "duration").unwrap_or(0.0), bpm),
        });
    }
    if off_channel {
        report.note("MIDI channels were merged; yadaw tracks are single-channel");
    }
    notes
}

#[allow(clippy::too_many_arguments)]
fn read_audio_clip(
    node: El,
    audio: El,
    unit: TimeUnit,
    content_unit: TimeUnit,
    start_beat: f64,
    length_beats: f64,
    name: Option<String>,
    color: Option<(u8, u8, u8)>,
    muted: bool,
    container: &mut Container,
    bpm: f64,
    track: &mut Track,
    report: &mut Report,
) {
    let Some(file) = child(audio, "File") else {
        return;
    };
    let path = attr(file, "path").unwrap_or_default();
    if path.is_empty() {
        return;
    }
    if parse_bool(attr(file, "external")) == Some(true) {
        report.note(
            "Audio files referenced from outside the container are not read; only files inside the .dawproject are loaded",
        );
        return;
    }

    let extension = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let clip_name = name
        .or_else(|| {
            path.rsplit('/')
                .next()
                .map(|f| f.split('.').next().unwrap_or(f))
                .map(sanitize_name)
        })
        .unwrap_or_else(|| "Audio Clip".to_string());

    let data = match container.read(path) {
        Some(data) => data.to_vec(),
        None => {
            report.note(format!("Audio file '{path}' is missing from the container"));
            return;
        }
    };

    let mut clip = match import_audio_data(&clip_name, &data, &extension, bpm as f32) {
        Ok(clip) => clip,
        Err(e) => {
            report.note(format!("Audio file '{path}' could not be decoded: {e}"));
            return;
        }
    };

    let warps = find_descendant(node, "Warps");
    let content_seconds_unit = warps
        .map(|w| TimeUnit::parse(attr(w, "contentTimeUnit")))
        .unwrap_or(content_unit);
    let warps_unit = warps
        .map(|w| scope_unit(w, "timeUnit", unit))
        .unwrap_or(unit);

    let loop_region = match (num_attr(node, "loopStart"), num_attr(node, "loopEnd")) {
        (Some(start), Some(end)) if end > start => Some((
            content_unit.to_seconds(start, bpm),
            content_unit.to_seconds(end, bpm),
        )),
        _ => None,
    };

    let audio_seconds =
        content_seconds_unit.to_seconds(num_attr(audio, "duration").unwrap_or(0.0), bpm);
    let slope = warps.and_then(|w| warp_slope(w, warps_unit, content_seconds_unit, bpm));
    let slope = slope.or_else(|| {
        // A looping clip repeats its loop region at natural speed unless Warps
        // says otherwise, so the whole-file ratio says nothing about it.
        (loop_region.is_none() && length_beats > 0.0 && audio_seconds > 0.0)
            .then(|| audio_seconds / length_beats)
    });

    let start_seconds = content_unit.to_seconds(num_attr(node, "playStart").unwrap_or(0.0), bpm);
    let start_sample = ((start_seconds * f64::from(clip.sample_rate))
        .round()
        .max(0.0) as usize)
        .min(clip.samples.len());
    if start_sample > 0 {
        clip.samples = Arc::new(clip.samples[start_sample..].to_vec());
    }

    if let Some((from, to)) = loop_region {
        let first = ((from * f64::from(clip.sample_rate)).round().max(0.0) as usize)
            .min(clip.samples.len());
        let last = ((to * f64::from(clip.sample_rate)).round().max(0.0) as usize)
            .clamp(first, clip.samples.len());
        if first > 0 || last < clip.samples.len() {
            clip.samples = Arc::new(clip.samples[first..last].to_vec());
        }
    }

    let natural = 60.0 / bpm;
    match slope {
        Some(value) if (value - natural).abs() > 1.0e-9 => {
            clip.warp_mode = true;
            report.note(
                "Time-warped audio was flattened to a single fixed speed mapping, which is all yadaw can store",
            );
        }
        _ => {
            clip.warp_mode = false;
        }
    }

    let content_beats =
        seconds_to_beats(clip.samples.len() as f64 / f64::from(clip.sample_rate), bpm);
    let mut length_beats = if length_beats > 0.0 {
        length_beats
    } else {
        content_beats
    };
    // Warping stretches the material across the whole clip, leaving no room to
    // also repeat a region of it.
    let looped = loop_region.is_some() && content_beats + 1.0e-9 < length_beats;
    if looped && clip.warp_mode {
        report.note(
            "Warped audio clips cannot also repeat a region of their material; that loop was dropped",
        );
    }
    let loop_enabled = looped && !clip.warp_mode;
    if !loop_enabled {
        length_beats = length_beats.min(content_beats);
    }

    let fade_unit = TimeUnit::parse(attr(node, "fadeTimeUnit"));
    clip.fade_in = read_fade(node, "fadeInTime", fade_unit, bpm, false);
    clip.fade_out = read_fade(node, "fadeOutTime", fade_unit, bpm, false);
    clip.crossfade_in = read_fade(node, "fadeInTime", fade_unit, bpm, true);
    clip.crossfade_out = read_fade(node, "fadeOutTime", fade_unit, bpm, true);
    clip.start_beat = start_beat;
    clip.length_beats = length_beats;
    clip.offset_beats = 0.0;
    clip.color = color;
    clip.muted = muted;
    clip.name = clip_name;
    clip.loop_enabled = loop_enabled;

    track.audio_clips.push(clip);
}

fn warp_slope(node: El, unit: TimeUnit, content_unit: TimeUnit, bpm: f64) -> Option<f64> {
    let mut points: Vec<(f64, f64)> = children(node)
        .filter(|n| n.has_tag_name("Warp"))
        .filter_map(|w| Some((num_attr(w, "time")?, num_attr(w, "contentTime")?)))
        .collect();
    if points.len() < 2 {
        return None;
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (t0, c0) = points[0];
    let (t1, c1) = points[points.len() - 1];
    let beats = unit.to_beats(t1, bpm) - unit.to_beats(t0, bpm);
    let seconds = content_unit.to_seconds(c1, bpm) - content_unit.to_seconds(c0, bpm);
    (beats.abs() > 1.0e-12).then(|| seconds / beats)
}

fn read_fade(node: El, attr: &str, unit: TimeUnit, bpm: f64, crossfade: bool) -> Option<f64> {
    let value = num_attr(node, attr)?;
    let is_crossfade = value < 0.0;
    if is_crossfade != crossfade {
        return None;
    }
    Some(unit.to_beats(value.abs(), bpm))
}

fn read_points(
    node: El,
    unit: TimeUnit,
    bpm: f64,
    params: &HashMap<String, ParamTarget>,
    track: &mut Track,
    report: &mut Report,
) {
    let Some(target) = child(node, "Target").and_then(|t| attr(t, "parameter")) else {
        return;
    };
    let Some(target) = params.get(target) else {
        report.note("Automation for a device that is not on this track was skipped");
        return;
    };
    let unit = scope_unit(node, "timeUnit", unit);
    let unit_on_points = node.attribute("unit").unwrap_or("linear");

    let mut points: Vec<AutomationPoint> = Vec::new();
    for point in children(node) {
        let time = num_attr(point, "time");
        let value = match point.tag_name().name() {
            "RealPoint" => num_attr(point, "value"),
            "BoolPoint" => parse_bool(attr(point, "value")).map(|b| if b { 1.0 } else { 0.0 }),
            "IntegerPoint" | "EnumPoint" => num_attr(point, "value"),
            _ => None,
        };
        if let (Some(time), Some(value)) = (time, value) {
            points.push(AutomationPoint {
                beat: unit.to_beats(time, bpm),
                value: value as f32,
            });
        }
    }
    if points.is_empty() {
        return;
    }
    points.sort_by(|a, b| a.beat.total_cmp(&b.beat));

    if matches!(target, ParamTarget::Volume | ParamTarget::Plugin(..)) && unit_on_points != "linear"
    {
        report.note(format!(
            "Automation in '{unit_on_points}' was read as a raw value because yadaw has no unit conversion"
        ));
    }
    if matches!(target, ParamTarget::Send(_)) {
        report.note("Send automation is stored but the audio engine does not evaluate it");
    }

    let parameter = match target {
        ParamTarget::Volume => AutomationTarget::TrackVolume,
        ParamTarget::Pan => AutomationTarget::TrackPan,
        ParamTarget::Send(index) => AutomationTarget::TrackSend(*index),
        ParamTarget::Plugin(id, name) => AutomationTarget::PluginParam {
            plugin_id: *id,
            param_name: name.clone(),
        },
    };

    track.automation_lanes.push(AutomationLane {
        parameter,
        points,
        visible: true,
        height: 60.0,
        color: None,
        write_mode: AutomationMode::Read,
        read_enabled: true,
    });
}

/// Repeated note data becomes one `MidiPattern` so that yadaw's alias clips
/// stay linked after a round trip.
fn share_repeated_notes(tracks: &mut [Track]) -> Vec<MidiPattern> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for track in tracks.iter() {
        for clip in &track.midi_clips {
            *counts.entry(note_key(&clip.notes)).or_default() += 1;
        }
    }

    let mut ids: HashMap<String, u64> = HashMap::new();
    let mut patterns: Vec<MidiPattern> = Vec::new();

    for track in tracks.iter_mut() {
        for clip in &mut track.midi_clips {
            let key = note_key(&clip.notes);
            if counts.get(&key).copied().unwrap_or(0) < 2 {
                continue;
            }
            let id = match ids.get(&key) {
                Some(id) => *id,
                None => {
                    let id = idgen::next();
                    ids.insert(key, id);
                    patterns.push(MidiPattern {
                        id,
                        notes: std::mem::take(&mut clip.notes),
                    });
                    id
                }
            };
            clip.notes.clear();
            clip.pattern_id = Some(id);
        }
    }

    patterns
}

fn note_key(notes: &[MidiNote]) -> String {
    let mut key = String::new();
    for note in notes {
        key.push_str(&num(note.pitch as f64));
        key.push(':');
        key.push_str(&num(note.start));
        key.push(':');
        key.push_str(&num(note.duration));
        key.push(':');
        key.push_str(&num(f64::from(note.velocity)));
        key.push('|');
    }
    key
}

fn metadata_title(xml: &str) -> Option<String> {
    let doc = Document::parse(xml).ok()?;
    let title = doc
        .root()
        .descendants()
        .find(|n| n.is_element() && n.has_tag_name("Title"))?;
    let text = title.text()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}
