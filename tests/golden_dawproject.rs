use std::sync::Arc;

use yadaw::dawproject;
use yadaw::model::automation::{AutomationLane, AutomationMode, AutomationPoint, AutomationTarget};
use yadaw::model::clip::{AudioClip, MidiClip, MidiNote, WarpPoint};
use yadaw::model::group::TrackGroup;
use yadaw::model::marker::Marker;
use yadaw::model::plugin::PluginDescriptor;
use yadaw::model::tempo::{TempoPoint, TempoRamp};
use yadaw::model::track::{Send, Track, TrackType};
use yadaw::project::{PROJECT_VERSION, Project};
use yadaw_plugin_api::BackendKind;

fn tone(len: usize) -> Arc<Vec<f32>> {
    Arc::new(
        (0..len)
            .map(|i| (i as f32 * 0.01).sin() * 0.5)
            .collect::<Vec<f32>>(),
    )
}

fn fixture() -> Project {
    let keys = Track {
        id: 1,
        name: "Keys & <Drums>".to_string(),
        track_type: TrackType::Midi,
        group_id: Some(10),
        color: Some((0xa2, 0xea, 0xbf)),
        volume: 0.66,
        pan: -0.25,
        muted: false,
        solo: true,
        midi_clips: vec![MidiClip {
            id: 11,
            name: "Verse".to_string(),
            start_beat: 0.0,
            length_beats: 8.0,
            content_len_beats: 4.0,
            content_offset_beats: 0.0,
            loop_enabled: true,
            notes: vec![
                MidiNote {
                    id: 1,
                    pitch: 60,
                    velocity: 100,
                    start: 0.0,
                    duration: 0.5,
                },
                MidiNote {
                    id: 2,
                    pitch: 64,
                    velocity: 64,
                    start: 1.0,
                    duration: 0.25,
                },
            ],
            ..Default::default()
        }],
        plugin_chain: vec![PluginDescriptor {
            id: 77,
            uri: "file:///usr/lib/liba.so#org.example.synth".to_string(),
            name: "Example Synth".to_string(),
            backend: BackendKind::Clap,
            bypass: false,
            has_editor: true,
            params: [("cutoff".to_string(), 1200.0)].into_iter().collect(),
            preset_name: None,
            custom_name: None,
        }],
        automation_lanes: vec![AutomationLane {
            parameter: AutomationTarget::TrackVolume,
            points: vec![
                AutomationPoint {
                    beat: 0.0,
                    value: 0.2,
                },
                AutomationPoint {
                    beat: 4.0,
                    value: 0.9,
                },
            ],
            visible: true,
            height: 60.0,
            color: None,
            write_mode: AutomationMode::Read,
            read_enabled: true,
        }],
        sends: vec![Send {
            destination_track: 2,
            amount: 0.4,
            pre_fader: true,
            muted: false,
        }],
        ..Default::default()
    };

    let drums = Track {
        id: 2,
        name: "Drums".to_string(),
        track_type: TrackType::Audio,
        group_id: Some(10),
        volume: 0.5,
        pan: 0.5,
        audio_clips: vec![AudioClip {
            id: 21,
            name: "loop.wav".to_string(),
            start_beat: 4.0,
            length_beats: 4.0,
            offset_beats: 0.0,
            samples: tone(75_600),
            sample_rate: 44100.0,
            fade_in: Some(0.25),
            fade_out: Some(0.5),
            ..Default::default()
        }],
        ..Default::default()
    };

    let bus = Track {
        id: 3,
        name: "Bus".to_string(),
        track_type: TrackType::Bus,
        ..Default::default()
    };

    Project {
        version: PROJECT_VERSION.to_string(),
        name: "Round Trip".to_string(),
        tracks: vec![keys, drums, bus],
        patterns: Vec::new(),
        groups: vec![
            TrackGroup {
                id: 10,
                name: "Rhythm".to_string(),
                color: (230, 126, 34),
                ..Default::default()
            },
            TrackGroup {
                id: 11,
                name: "Spare".to_string(),
                ..Default::default()
            },
        ],
        markers: vec![
            Marker {
                id: 900,
                beat: 8.0,
                name: "Chorus".to_string(),
                color: Some((241, 196, 15)),
                comment: Some("guitars in".to_string()),
            },
            Marker {
                id: 901,
                beat: 0.0,
                name: "Intro".to_string(),
                color: None,
                comment: None,
            },
        ],
        bpm: 140.0,
        tempo_map: Vec::new(),
        time_signature: (3, 4),
        sample_rate: 44100.0,
        master_volume: 0.75,
        loop_start: 0.0,
        loop_end: 16.0,
        loop_enabled: false,
        created_at: chrono::Utc::now(),
        modified_at: chrono::Utc::now(),
    }
}

fn no_plugins(_backend: BackendKind, _device_id: &str, _device_name: &str) -> Option<String> {
    None
}

#[test]
fn golden_dawproject_round_trip() {
    let original = fixture();
    let (bytes, export_report) = dawproject::export(&original).expect("export succeeds");
    assert!(!bytes.is_empty());
    assert!(!export_report.summary.is_empty());

    let (imported, import_report) =
        dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert!(!import_report.summary.is_empty());

    assert_eq!(imported.bpm, 140.0);
    assert_eq!(imported.time_signature, (3, 4));
    assert_eq!(imported.master_volume, 0.75);
    assert_eq!(imported.name, "Round Trip");
    assert_eq!(imported.tracks.len(), 3, "master channel is not a track");

    let keys = imported
        .tracks
        .iter()
        .find(|t| t.name == "Keys & <Drums>")
        .expect("keys track survives xml escaping");
    assert_eq!(keys.track_type, TrackType::Midi);
    assert_eq!(keys.color, Some((0xa2, 0xea, 0xbf)));
    assert!(keys.solo);
    assert_eq!(keys.volume, 0.66);
    assert_eq!(keys.pan, -0.25);

    let clip = &keys.midi_clips[0];
    assert_eq!(clip.start_beat, 0.0);
    assert_eq!(clip.length_beats, 8.0);
    assert_eq!(clip.content_len_beats, 4.0);
    assert!(
        clip.loop_enabled,
        "loop length below clip length means a loop"
    );
    assert_eq!(clip.notes.len(), 2);
    assert_eq!(clip.notes[0].pitch, 60);
    assert_eq!(clip.notes[0].velocity, 100);
    assert_eq!(clip.notes[1].start, 1.0);
    assert_eq!(clip.notes[1].duration, 0.25);

    assert_eq!(keys.sends.len(), 1);
    assert!(keys.sends[0].pre_fader);
    assert_eq!(keys.sends[0].amount, 0.4);

    let plugin = &keys.plugin_chain[0];
    assert_eq!(plugin.backend, BackendKind::Clap);
    assert_eq!(plugin.name, "Example Synth");
    assert_eq!(plugin.params.get("cutoff"), Some(&1200.0));

    let volume_lane = keys
        .automation_lanes
        .iter()
        .find(|l| l.parameter == AutomationTarget::TrackVolume)
        .expect("volume automation survives");
    assert_eq!(volume_lane.points.len(), 2);
    assert_eq!(volume_lane.points[1].beat, 4.0);
    assert_eq!(volume_lane.points[1].value, 0.9);

    let drums = imported
        .tracks
        .iter()
        .find(|t| t.name == "Drums")
        .expect("audio track survives");
    let audio = &drums.audio_clips[0];
    assert_eq!(audio.start_beat, 4.0);
    assert_eq!(audio.length_beats, 4.0);
    assert!((audio.sample_rate - 44100.0).abs() < 1.0);
    assert!(
        (audio.samples.len() as f64 - 75_600.0).abs() < 2.0,
        "embedded wav must round-trip the sample region, got {}",
        audio.samples.len()
    );
    assert_eq!(audio.fade_in, Some(0.25));
    assert_eq!(audio.fade_out, Some(0.5));

    assert_eq!(imported.tracks[2].track_type, TrackType::Bus);

    assert_eq!(imported.markers.len(), 2, "markers survive the round trip");
    assert_eq!(imported.markers[0].name, "Intro", "sorted by beat");
    assert_eq!(imported.markers[0].beat, 0.0);
    assert_eq!(imported.markers[0].color, None);
    assert_eq!(imported.markers[1].name, "Chorus");
    assert_eq!(imported.markers[1].beat, 8.0);
    assert_eq!(imported.markers[1].color, Some((241, 196, 15)));
    assert_eq!(
        imported.markers[1].comment.as_deref(),
        Some("guitars in"),
        "a marker comment survives"
    );

    let rhythm = imported
        .groups
        .iter()
        .find(|g| g.name == "Rhythm")
        .expect("the group survives the round trip");
    assert_eq!(rhythm.color, (230, 126, 34));
    assert_eq!(keys.group_id, Some(rhythm.id));
    assert_eq!(drums.group_id, Some(rhythm.id));
    assert_eq!(
        imported.tracks[2].group_id, None,
        "a track outside every group stays top level"
    );
    assert!(
        imported.groups.iter().any(|g| g.name == "Spare"),
        "a group with no tracks is still written and read back"
    );

    let (again, _) = dawproject::export(&imported).expect("re-export succeeds");
    let (reimported, _) = dawproject::import(&again, &no_plugins).expect("re-import succeeds");
    assert_eq!(reimported.tracks.len(), imported.tracks.len());
    assert_eq!(reimported.bpm, imported.bpm);
    assert_eq!(reimported.tracks[0].midi_clips[0].notes.len(), 2);
    assert_eq!(
        reimported.tracks[1].audio_clips[0].samples.len(),
        imported.tracks[1].audio_clips[0].samples.len()
    );
    assert_eq!(reimported.groups.len(), imported.groups.len());
    assert_eq!(
        reimported.markers.len(),
        imported.markers.len(),
        "markers are stable across a second round trip"
    );
    assert_eq!(reimported.markers[1].name, "Chorus");
}

#[test]
fn dawproject_exports_groups_as_nested_tracks() {
    let (bytes, _) = dawproject::export(&fixture()).expect("export succeeds");
    let xml = project_xml(&bytes);

    let structure = xml
        .split_once("<Structure>")
        .and_then(|(_, rest)| rest.split_once("</Structure>"))
        .expect("a Structure element is written")
        .0;

    let top_level: Vec<&str> = structure
        .lines()
        .filter(|l| l.starts_with("    <Track"))
        .map(str::trim)
        .collect();
    assert_eq!(
        top_level.len(),
        4,
        "the Rhythm group, the Bus, the empty Spare group and the Master, got {top_level:?}"
    );
    assert!(
        top_level[0].contains("contentType=\"tracks\""),
        "a group is a contentType=\"tracks\" Track: {}",
        top_level[0]
    );
    assert!(top_level[0].contains("name=\"Rhythm\""));
    assert!(top_level[0].contains("color=\"#e67e22\""));
    assert!(
        top_level[1].contains("name=\"Bus\""),
        "an ungrouped track stays top level: {}",
        top_level[1]
    );
    assert!(
        top_level[2].contains("name=\"Spare\""),
        "a group with no members still gets an element: {}",
        top_level[2]
    );
    assert!(
        top_level[3].contains("name=\"Master\""),
        "the master comes after the last group: {}",
        top_level[3]
    );
    assert!(
        structure.contains("name=\"Spare\" color=\"#3498db\" contentType=\"tracks\" loaded=\"true\">\n      <Channel"),
        "even a member-less group carries its own channel, so it is not self-closing"
    );

    let members: Vec<&str> = structure
        .lines()
        .filter(|l| l.starts_with("      <Track"))
        .map(str::trim)
        .collect();
    assert_eq!(
        members.len(),
        2,
        "both members of Rhythm nest inside it, got {members:?}"
    );
    assert!(members[0].contains("name=\"Keys &amp; &lt;Drums&gt;\""));
    assert!(members[1].contains("name=\"Drums\""));
    assert!(
        structure.contains("\n    </Track>\n    <Track"),
        "the group closes before the next top-level Track opens"
    );
}

#[test]
fn dawproject_round_trips_looped_audio_clips() {
    let project = Project {
        version: PROJECT_VERSION.to_string(),
        name: "Looped".to_string(),
        tracks: vec![Track {
            id: 1,
            name: "Drums".to_string(),
            track_type: TrackType::Audio,
            audio_clips: vec![AudioClip {
                id: 21,
                name: "riser.wav".to_string(),
                start_beat: 0.0,
                length_beats: 8.0,
                offset_beats: 0.0,
                samples: tone(44_100),
                sample_rate: 44100.0,
                loop_enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        patterns: Vec::new(),
        groups: Vec::new(),
        markers: vec![Marker {
            id: 900,
            beat: 8.0,
            name: "Chorus".to_string(),
            color: Some((241, 196, 15)),
            comment: Some("guitars in".to_string()),
        }],
        bpm: 120.0,
        tempo_map: Vec::new(),
        time_signature: (4, 4),
        sample_rate: 44100.0,
        master_volume: 0.8,
        loop_start: 0.0,
        loop_end: 16.0,
        loop_enabled: false,
        created_at: chrono::Utc::now(),
        modified_at: chrono::Utc::now(),
    };

    let (bytes, _) = dawproject::export(&project).expect("export succeeds");
    let clip_xml = project_xml(&bytes)
        .lines()
        .find(|l| l.contains("<Clip "))
        .expect("an audio clip is written")
        .to_string();
    assert!(
        clip_xml.contains("loopStart=") && clip_xml.contains("loopEnd="),
        "a looping clip declares its loop region: {clip_xml}"
    );

    let (imported, report) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    let clip = &imported.tracks[0].audio_clips[0];
    assert!(clip.loop_enabled, "clip looping survives the round trip");
    assert_eq!(
        clip.samples.len(),
        44_100,
        "the material that repeats is kept whole"
    );
    assert_eq!(
        clip.length_beats, 8.0,
        "a clip longer than its material is not truncated, got {}",
        clip.length_beats
    );
    assert!(
        !report.notes.iter().any(|n| n.contains("looped audio clip")),
        "nothing was dropped: {:?}",
        report.notes
    );
}

#[test]
fn dawproject_imports_a_partial_audio_loop_region() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <Tempo max="666.000000" min="20.000000" unit="bpm" value="120.000000" id="id0" name="Tempo"/>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track contentType="audio" loaded="true" id="id2" name="Drums">
      <Channel audioChannels="2" role="regular" solo="false" id="id3">
        <Volume max="2.000000" min="0.000000" unit="linear" value="1.000000" id="id4" name="Volume"/>
      </Channel>
    </Track>
  </Structure>
  <Arrangement id="id5">
    <Lanes timeUnit="beats" id="id6">
      <Lanes track="id2" id="id7">
        <Clips id="id8">
          <Clip time="0.0" duration="4.0" contentTimeUnit="seconds" loopStart="0.5" loopEnd="1.0" name="riser.wav">
            <Audio channels="1" duration="2.0" sampleRate="44100" id="id9">
              <File path="Audio/riser.wav" id="id10"/>
            </Audio>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let media = wav(&tone(88_200));
    let (bytes, _) = zip_fixture_with_media(xml, &[("Audio/riser.wav", media)]);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");

    let clip = &project.tracks[0].audio_clips[0];
    assert!(clip.loop_enabled, "a sub-region loop still loops");
    assert_eq!(
        clip.samples.len(),
        22_050,
        "the material is cut down to the 0.5s..1.0s loop region, got {}",
        clip.samples.len()
    );
    assert_eq!(
        clip.length_beats, 4.0,
        "the arrangement length outlasts the material it repeats"
    );
    assert!(!clip.warp_mode, "a loop region is not a time warp");
}

#[test]
fn dawproject_imports_bitwig_style_alias_and_nested_clips() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Bitwig Studio" version="5.0"/>
  <Transport>
    <Tempo max="666.000000" min="20.000000" unit="bpm" value="149.000000" id="id0" name="Tempo"/>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track color="#e67e22" contentType="tracks" id="id40" loaded="true" name="Rhythm">
      <Channel audioChannels="2" destination="id15" role="submix" solo="false" id="id41"/>
      <Track contentType="notes" loaded="true" id="id2" name="Bass">
        <Channel audioChannels="2" destination="id15" role="regular" solo="false" id="id3">
          <Devices>
            <ClapPlugin deviceID="org.surge-synth-team.surge-xt" deviceName="Surge XT" deviceRole="instrument" loaded="true" id="id7" name="Surge XT">
              <Parameters>
                <RealParameter name="Gain" value="0.5" unit="normalized" id="id20"/>
              </Parameters>
              <Enabled value="true" id="id8" name="On/Off"/>
            </ClapPlugin>
          </Devices>
          <Mute value="false" id="id6" name="Mute"/>
          <Pan max="1.000000" min="0.000000" unit="normalized" value="0.500000" id="id5" name="Pan"/>
          <Volume max="2.000000" min="0.000000" unit="linear" value="0.659140" id="id4" name="Volume"/>
        </Channel>
      </Track>
    </Track>
    <Track contentType="audio notes" loaded="true" id="id14" name="Master">
      <Channel audioChannels="2" role="master" solo="false" id="id15">
        <Volume max="2.000000" min="0.000000" unit="linear" value="1.000000" id="id16" name="Volume"/>
      </Channel>
    </Track>
  </Structure>
  <Arrangement id="id19">
    <Lanes timeUnit="beats" id="id20">
      <Lanes track="id2" id="id21">
        <Clips id="id22">
          <Clip time="0.0" duration="8.0" contentTimeUnit="beats" playStart="0.0" loopStart="0.0" loopEnd="8.0">
            <Notes id="id23">
              <Note time="0.0" duration="0.25" channel="0" key="65" vel="0.787402"/>
              <Note time="1.5" duration="2.5" channel="0" key="53" vel="0.787402"/>
            </Notes>
          </Clip>
          <Clip time="8.0" duration="8.0" reference="id22"/>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, report) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");

    assert_eq!(project.bpm, 149.0);
    assert_eq!(
        project.tracks.len(),
        1,
        "the master track is not imported as a track"
    );
    assert_eq!(project.master_volume, 1.0);

    let bass = &project.tracks[0];
    assert_eq!(bass.name, "Bass");

    let rhythm = &project.groups[0];
    assert_eq!(rhythm.name, "Rhythm");
    assert_eq!(rhythm.color, (230, 126, 34));
    assert_eq!(
        bass.group_id,
        Some(rhythm.id),
        "a nested Track joins the group that holds it"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("Group track 'Rhythm' is a mixer group")),
        "a submix folder channel is not the same as a VCA and must be reported, got {:?}",
        report.notes
    );

    assert!((bass.volume - 0.65914).abs() < 1e-4);
    assert_eq!(
        bass.midi_clips.len(),
        2,
        "an alias clip resolves to real notes"
    );
    assert_eq!(bass.midi_clips[1].start_beat, 8.0);
    assert_eq!(
        bass.midi_clips[0].notes.len(),
        0,
        "shared notes move into a pattern"
    );
    assert_eq!(
        project.patterns.len(),
        1,
        "aliased notes become one shared pattern"
    );
    let pattern = &project.patterns[0];
    assert_eq!(pattern.notes.len(), 2);
    assert_eq!(pattern.notes[1].pitch, 53);
    assert_eq!(bass.midi_clips[0].pattern_id, Some(pattern.id));
    assert_eq!(bass.midi_clips[1].pattern_id, Some(pattern.id));
    assert_eq!(bass.plugin_chain.len(), 1);
    assert_eq!(bass.plugin_chain[0].uri, "org.surge-synth-team.surge-xt");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("could not be matched")),
        "an unresolved plugin must be reported"
    );
}

#[test]
fn dawproject_keeps_a_folder_inside_a_folder() {

    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <Tempo max="666" min="20" unit="bpm" value="120" id="id0" name="Tempo"/>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track color="#e67e22" contentType="tracks" id="id10" loaded="true" name="Drums">
      <Track color="#3498db" contentType="tracks" id="id11" loaded="true" name="Toms">
        <Track contentType="audio" loaded="true" id="id12" name="Floor">
          <Channel audioChannels="2" destination="id20" role="regular" solo="false" id="id13">
            <Volume max="2" min="0" unit="linear" value="1" id="id14" name="Volume"/>
          </Channel>
        </Track>
      </Track>
    </Track>
    <Track contentType="notes" loaded="true" id="id15" name="Lead">
      <Channel audioChannels="2" destination="id20" role="regular" solo="false" id="id16">
        <Volume max="2" min="0" unit="linear" value="1" id="id17" name="Volume"/>
      </Channel>
    </Track>
    <Track contentType="audio notes" loaded="true" id="id19" name="Master">
      <Channel audioChannels="2" role="master" solo="false" id="id20">
        <Volume max="2" min="0" unit="linear" value="1" id="id21" name="Volume"/>
      </Channel>
    </Track>
  </Structure>
  <Arrangement id="id30">
    <Lanes timeUnit="beats" id="id31">
      <Lanes track="id12" id="id32"/>
      <Lanes track="id15" id="id33"/>
    </Lanes>
  </Arrangement>
  <AudioRegions/>
  <MidiRegions/>
</Project>
"##;
    let (bytes, _) = zip_fixture(xml);
    let (project, report) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");

    let drums = project
        .groups
        .iter()
        .find(|g| g.name == "Drums")
        .expect("the outer folder is a group");
    let toms = project
        .groups
        .iter()
        .find(|g| g.name == "Toms")
        .expect("the inner folder is a group");

    assert_eq!(
        toms.parent_id,
        Some(drums.id),
        "the inner folder keeps the outer one as its parent, got {:?}",
        toms.parent_id
    );
    assert_eq!(
        drums.parent_id, None,
        "the outer folder stays at the top level"
    );

    let floor = project
        .tracks
        .iter()
        .find(|t| t.name == "Floor")
        .expect("the track inside both folders is imported");
    assert_eq!(
        floor.group_id,
        Some(toms.id),
        "a track joins the folder that directly contains it"
    );
    assert_eq!(
        project
            .tracks
            .iter()
            .find(|t| t.name == "Lead")
            .expect("an ungrouped track is imported")
            .group_id,
        None,
        "a track outside every folder stays top level"
    );
    let _ = report;

    let (bytes, _) = dawproject::export(&project).expect("export succeeds");
    let out = project_xml(&bytes);
    let outer = out.find("name=\"Drums\"").expect("Drums is written");
    let inner = out.find("name=\"Toms\"").expect("Toms is written");
    let floor_at = out.find("name=\"Floor\"").expect("Floor is written");
    let lead_at = out.find("name=\"Lead\"").expect("Lead is written");
    assert!(
        outer < inner && inner < floor_at,
        "Toms nests inside Drums and Floor nests inside Toms"
    );
    assert!(
        floor_at < lead_at,
        "and Lead, which is in no folder, comes after the closed Drums folder"
    );

    let (re_bytes, _) = zip_fixture(&out);
    let (again, _) = dawproject::import(&re_bytes, &no_plugins).expect("re-import succeeds");
    let drums2 = again.groups.iter().find(|g| g.name == "Drums").unwrap();
    let toms2 = again.groups.iter().find(|g| g.name == "Toms").unwrap();
    assert_eq!(
        toms2.parent_id,
        Some(drums2.id),
        "a round trip keeps the folder inside the folder"
    );
}

#[test]
fn dawproject_carries_a_groups_level_mute_and_solo() {

    let original = Project {
        groups: vec![
            TrackGroup {
                id: 10,
                name: "Drums".to_string(),
                volume: 0.25,
                muted: true,
                solo: true,
                ..Default::default()
            },
            TrackGroup {
                id: 11,
                name: "Keys".to_string(),
                ..Default::default()
            },
        ],
        ..fixture()
    };

    let (bytes, report) = dawproject::export(&original).expect("export succeeds");
    let xml = project_xml(&bytes);
    assert!(
        !xml.contains("VCA-style control links"),
        "the lossy note is no longer accurate now that level is written"
    );

    let drums_channel = xml
        .split_once("name=\"Drums\"")
        .and_then(|(_, rest)| rest.split_once("</Channel>"))
        .map(|(head, _)| head)
        .expect("a Drums channel is written");
    assert!(
        drums_channel.contains("role=\"vca\""),
        "the group channel is a vca, which controls other channels: {drums_channel}"
    );
    assert!(
        drums_channel.contains("solo=\"true\""),
        "group solo is written: {drums_channel}"
    );
    assert!(
        drums_channel.contains("<Mute value=\"true\""),
        "group mute is written: {drums_channel}"
    );
    assert!(
        drums_channel.contains("value=\"0.25\""),
        "group level is written: {drums_channel}"
    );
    let _ = report;

    let (reimported, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    let drums = reimported
        .groups
        .iter()
        .find(|g| g.name == "Drums")
        .expect("Drums is a group again");
    assert!(
        (drums.volume - 0.25).abs() < 1e-6,
        "the group fader survives the round trip, got {}",
        drums.volume
    );
    assert!(drums.muted, "group mute survives");
    assert!(drums.solo, "group solo survives");

    let keys = reimported.groups.iter().find(|g| g.name == "Keys").unwrap();
    assert_eq!(keys.volume, 1.0, "an untouched group stays at unity");
    assert!(!keys.muted && !keys.solo, "and is neither muted nor soloed");
}

#[test]
fn dawproject_reads_a_bare_master_channel() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <Tempo max="666" min="20" unit="bpm" value="100" id="id0" name="Tempo"/>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Channel audioChannels="2" role="master" solo="false" id="id2">
      <Volume max="2" min="0" unit="linear" value="0.5" id="id3" name="Volume"/>
    </Channel>
    <Track contentType="notes" loaded="true" id="id4" name="Pad">
      <Channel audioChannels="2" role="regular" solo="false" id="id5">
        <Volume max="2" min="0" unit="linear" value="1" id="id6" name="Volume"/>
      </Channel>
    </Track>
  </Structure>
  <Arrangement id="id7">
    <Lanes timeUnit="beats" id="id8">
      <Lanes track="id4" id="id9">
        <Clips id="id10">
          <Clip time="0.0" duration="4.0" contentTimeUnit="beats" playStart="0.0">
            <Notes id="id11">
              <Note time="0.0" duration="1.0" channel="0" key="60" vel="0.5"/>
            </Notes>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");

    assert_eq!(project.bpm, 100.0);
    assert_eq!(
        project.master_volume, 0.5,
        "a master Channel directly under Structure carries the master volume"
    );
    assert_eq!(project.tracks.len(), 1);
    assert_eq!(project.tracks[0].name, "Pad");
    assert_eq!(project.tracks[0].group_id, None);
}

#[test]
fn dawproject_reads_a_tempo_track_when_transport_is_silent() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Studio One" version="6.5"/>
  <Transport>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad">
      <Channel audioChannels="2" role="regular" solo="false" id="id3"/>
    </Track>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="beats" id="id10">
        <RealPoint time="0" value="100" interpolation="hold"/>
        <RealPoint time="8" value="150" interpolation="hold"/>
    </TempoAutomation>
    <Lanes timeUnit="beats" id="id6">
      <Lanes track="id2" id="id7">
        <Clips id="id8">
          <Clip time="0.0" duration="4.0" contentTimeUnit="seconds" name="pad">
            <Notes id="id9">
              <Note key="60" time="0.0" duration="0.5" vel="0.8"/>
            </Notes>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");

    assert_eq!(
        project.bpm, 100.0,
        "the track's first point is the tempo the project opens at"
    );
    let clip = &project.tracks[0].midi_clips[0];
    assert_eq!(clip.length_beats, 4.0);
    let note = &clip.notes[0];
    assert_eq!(
        note.duration,
        0.5 * 100.0 / 60.0,
        "the note is timed at the track's tempo, not at 120"
    );
    assert_eq!(
        project.tempo_map.len(),
        2,
        "both points of the track are kept, not just its first tempo"
    );
    assert_eq!(project.tempo_map[1].beat, 8.0);
}

#[test]
fn dawproject_falls_through_an_unconvertible_transport_tempo() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <Tempo unit="smpte" value="25" id="id0" name="Tempo"/>
  </Transport>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad"/>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="beats" id="id10">
        <RealPoint time="0" value="132" interpolation="hold"/>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert_eq!(project.bpm, 132.0, "the tempo track beats an unusable unit");
}

#[test]
fn dawproject_prefers_an_explicit_transport_tempo_over_the_track() {
    let with_points = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <Tempo unit="bpm" value="140" id="id0" name="Tempo"/>
  </Transport>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad"/>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="beats" id="id10">
        <RealPoint time="0" value="100" interpolation="hold"/>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;
    let empty_points = with_points.replace(
        r#"<RealPoint time="0" value="100" interpolation="hold"/>"#,
        "",
    );

    for (label, xml) in [("populated", with_points), ("empty", &empty_points)] {
        let (bytes, _) = zip_fixture(xml);
        let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
        assert_eq!(
            project.bpm, 140.0,
            "an explicit {label} transport tempo wins"
        );
    }
}

#[test]
fn dawproject_falls_back_to_120_when_no_tempo_exists_at_all() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad"/>
  </Structure>
  <Arrangement id="id5"/>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert_eq!(project.bpm, 120.0);
}

#[test]
fn dawproject_ignores_unusable_tempo_track_values() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad"/>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="beats" id="id10">
        <RealPoint time="0" value="0" interpolation="hold"/>
        <RealPoint time="4" value="-80" interpolation="hold"/>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert_eq!(
        project.bpm, 120.0,
        "no usable tempo means the 120 BPM default"
    );
}

#[test]
fn dawproject_takes_the_earliest_tempo_track_point() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad"/>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="seconds" id="id10">
        <RealPoint time="8" value="150" interpolation="hold"/>
        <RealPoint time="0" value="90" interpolation="hold"/>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert_eq!(
        project.bpm, 90.0,
        "time 0 is the tempo the project opens at"
    );
}

#[test]
fn dawproject_round_trips_a_multi_point_warp() {
    let clip = AudioClip {
        id: 1,
        name: "ramp".to_string(),
        start_beat: 0.0,
        length_beats: 8.0,
        sample_rate: 44_100.0,
        samples: tone(4 * 44_100),
        warp_mode: true,
        warps: vec![
            WarpPoint {
                beat: 0.0,
                content_seconds: 0.0,
            },
            WarpPoint {
                beat: 4.0,
                content_seconds: 1.0,
            },
            WarpPoint {
                beat: 8.0,
                content_seconds: 4.0,
            },
        ],
        ..Default::default()
    };
    let project = Project {
        bpm: 120.0,
        tracks: vec![Track {
            id: 1,
            name: "Audio".to_string(),
            track_type: TrackType::Audio,
            audio_clips: vec![clip],
            ..Track::default()
        }],
        ..fixture()
    };

    let (bytes, _) = dawproject::export(&project).expect("export succeeds");
    let xml = project_xml(&bytes);
    let count = xml.matches("<Warp ").count();
    assert!(
        count >= 3,
        "the middle warp point survives export, found {count} <Warp> elements"
    );

    let (reimported, _) = dawproject::import(&bytes, &no_plugins).expect("re-import succeeds");
    let clip = &reimported.tracks[0].audio_clips[0];
    assert!(clip.warp_mode);
    assert_eq!(clip.warps.len(), 3, "all three points come back");
    for (want, got) in clip.warps.iter().zip([
        WarpPoint {
            beat: 0.0,
            content_seconds: 0.0,
        },
        WarpPoint {
            beat: 4.0,
            content_seconds: 1.0,
        },
        WarpPoint {
            beat: 8.0,
            content_seconds: 4.0,
        },
    ]) {
        assert!(
            (got.beat - want.beat).abs() < 1e-6,
            "beat {} vs {}",
            got.beat,
            want.beat
        );
        assert!(
            (got.content_seconds - want.content_seconds).abs() < 1e-6,
            "content {} vs {}",
            got.content_seconds,
            want.content_seconds
        );
    }
}

#[test]
fn dawproject_keeps_a_loop_only_when_the_warp_fits_inside_the_clip() {
    let warped = |beat_len: f64, last_beat: f64| {
        let xml = format!(
            r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <Tempo max="666" min="20" unit="bpm" value="120" id="id0" name="Tempo"/>
  </Transport>
  <Structure>
    <Track contentType="audio" loaded="true" id="id2" name="Audio">
      <Channel audioChannels="2" role="regular" solo="false" id="id3"/>
    </Track>
  </Structure>
  <Arrangement id="id5">
    <Lanes timeUnit="beats" id="id6">
      <Lanes track="id2" id="id7">
        <Clips id="id8">
          <Clip time="0.0" duration="{beat_len}" contentTimeUnit="seconds" loopStart="0.0" loopEnd="1.0" name="riser.wav">
            <Audio channels="1" duration="4.0" sampleRate="44100" id="id9">
              <File path="Audio/riser.wav" id="id10"/>
            </Audio>
            <Warps contentTimeUnit="seconds" timeUnit="beats" id="id11">
              <Warp time="0" contentTime="0"/>
              <Warp time="{last_beat}" contentTime="1.0"/>
            </Warps>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
  <Scenes/>
</Project>
"##
        );
        let media = wav(&tone(176_400));
        let (bytes, _) = zip_fixture_with_media(&xml, &[("Audio/riser.wav", media)]);
        dawproject::import(&bytes, &no_plugins).expect("import succeeds")
    };

    let (fits, _) = warped(16.0, 2.0);
    let clip = &fits.tracks[0].audio_clips[0];
    assert!(clip.warp_mode, "the clip is time-warped");
    assert!(clip.loop_enabled, "a warp that fits inside the clip loops");

    let (stretched, report) = warped(4.0, 40.0);
    let clip = &stretched.tracks[0].audio_clips[0];
    assert!(
        !clip.loop_enabled,
        "a warp that outruns the clip drops the loop"
    );
    assert!(
        report.notes.iter().any(|n| n.contains("warp needs")),
        "and the reason is reported, got {:?}",
        report.notes
    );
}

fn project_xml(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("export is a zip");
    let mut file = archive
        .by_name("project.xml")
        .expect("the container holds project.xml");
    let mut xml = String::new();
    file.read_to_string(&mut xml).expect("project.xml is utf-8");
    xml
}

fn zip_fixture(project_xml: &str) -> (Vec<u8>, String) {
    zip_fixture_with_media(project_xml, &[])
}

fn zip_fixture_with_media(project_xml: &str, media: &[(&str, Vec<u8>)]) -> (Vec<u8>, String) {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("project.xml", options).unwrap();
        writer.write_all(project_xml.as_bytes()).unwrap();
        for (name, data) in media {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    (cursor.into_inner(), project_xml.to_string())
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }
    cursor.into_inner()
}

/// Cubase and Cubasis write the tempo track in seconds, where a linear ramp runs
/// against time rather than against beats. Both domains have to survive import.
#[test]
fn dawproject_imports_a_tempo_track_in_seconds() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Cubase" version="14.0"/>
  <Transport>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad">
      <Channel audioChannels="2" role="regular" solo="false" id="id3"/>
    </Track>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="seconds" id="id10">
        <RealPoint time="0" value="120" interpolation="linear"/>
        <RealPoint time="4" value="60" interpolation="hold"/>
        <RealPoint time="8" value="60" interpolation="hold"/>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");

    assert_eq!(project.tempo_map.len(), 3, "every point is kept");
    assert_eq!(
        project.tempo_map[0].ramp,
        TempoRamp::Seconds,
        "and knows its domain"
    );
    assert_eq!(project.tempo_map[1].ramp, TempoRamp::Hold);

    // 4 seconds ramping 120 -> 60 averages 90 BPM, so that span is 6 beats, and
    // the following 4 seconds at 60 BPM is 4 more.
    assert!(
        (project.tempo_map[1].beat - 6.0).abs() < 1e-9,
        "{}",
        project.tempo_map[1].beat
    );
    assert!(
        (project.tempo_map[2].beat - 10.0).abs() < 1e-9,
        "{}",
        project.tempo_map[2].beat
    );
}

/// A beat-unit track is already in beats and must not be integrated.
#[test]
fn dawproject_keeps_a_beat_tempo_track_in_beats() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Bitwig Studio" version="5.0"/>
  <Transport>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad">
      <Channel audioChannels="2" role="regular" solo="false" id="id3"/>
    </Track>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="beats" id="id10">
        <RealPoint time="0" value="120" interpolation="hold"/>
        <RealPoint time="16" value="60" interpolation="hold"/>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert_eq!(project.tempo_map[1].beat, 16.0, "beats are left alone");
    assert_eq!(project.tempo_map[0].ramp, TempoRamp::Hold);
}

/// A saved tempo map has to come back out of the container, and every point has
/// to name its interpolation, because DAWproject reads a missing one as a hold.
#[test]
fn dawproject_round_trips_a_tempo_map() {
    for (ramp, unit) in [(TempoRamp::Beats, "beats"), (TempoRamp::Seconds, "seconds")] {
        let want = vec![
            TempoPoint::held(0.0, 120.0),
            TempoPoint::ramped_in(16.0, 90.0, ramp),
            TempoPoint::held(32.0, 60.0),
        ];
        let project = Project {
            bpm: 120.0,
            tempo_map: want.clone(),
            ..fixture()
        };
        let (bytes, _) = dawproject::export(&project).expect("export succeeds");
        let xml = project_xml(&bytes);
        let track = xml
            .split_once("<TempoAutomation")
            .and_then(|(_, rest)| rest.split_once("</TempoAutomation>"))
            .map(|(body, _)| body)
            .expect("the tempo track is written");
        assert!(
            track.contains(&format!("timeUnit=\"{unit}\"")),
            "a {ramp:?} ramp is written in {unit}"
        );
        assert_eq!(
            track.matches("<RealPoint").count(),
            3,
            "one point per anchor"
        );
        assert_eq!(
            track.matches("interpolation=\"hold\"").count(),
            2,
            "and every point names its interpolation, since a missing one means hold"
        );

        let (back, _) = dawproject::import(&bytes, &no_plugins).expect("re-import succeeds");
        assert_eq!(back.tempo_map.len(), 3);
        for (want, got) in want.iter().zip(&back.tempo_map) {
            assert!(
                (got.beat - want.beat).abs() < 1e-6,
                "beat {} vs {}",
                got.beat,
                want.beat
            );
            assert!(
                (got.bpm - want.bpm).abs() < 1e-4,
                "bpm {} vs {}",
                got.bpm,
                want.bpm
            );
            assert_eq!(got.ramp, want.ramp, "ramp mode at beat {}", want.beat);
        }
    }
}

/// A ramp linear in beats is not linear in seconds, so a map using both domains
/// has no faithful DAWproject encoding and is refused rather than mistranslated.
#[test]
fn dawproject_refuses_a_tempo_map_that_mixes_ramp_domains() {
    let project = Project {
        bpm: 120.0,
        tempo_map: vec![
            TempoPoint::held(0.0, 120.0),
            TempoPoint::ramped_in(16.0, 90.0, TempoRamp::Beats),
            TempoPoint::ramped_in(32.0, 60.0, TempoRamp::Seconds),
        ],
        ..fixture()
    };
    let (bytes, report) = dawproject::export(&project).expect("export succeeds");
    assert!(
        !project_xml(&bytes).contains("<TempoAutomation"),
        "a mistranslatable tempo curve is not written"
    );
    assert!(
        report.notes.iter().any(|n| n.contains("mixes")),
        "and the reason is reported, got {:?}",
        report.notes
    );
}

/// A single-point map is a constant tempo, which `Transport/Tempo` already says.
#[test]
fn dawproject_writes_no_tempo_track_for_a_constant_project() {
    let project = Project {
        bpm: 120.0,
        tempo_map: vec![TempoPoint::new(0.0, 120.0)],
        ..fixture()
    };
    let (bytes, _) = dawproject::export(&project).expect("export succeeds");
    assert!(
        !project_xml(&bytes).contains("<TempoAutomation"),
        "a constant tempo stays in Transport only"
    );
}

/// A `TempoAutomation` element written as a wrapper around a `Points` child is
/// what the spec's own example shows, so both shapes have to be read.
#[test]
fn dawproject_reads_a_tempo_track_wrapped_in_points() {
    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Yadaw" version="0.10.9"/>
  <Transport>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
    <Track contentType="notes" loaded="true" id="id2" name="Pad">
      <Channel audioChannels="2" role="regular" solo="false" id="id3"/>
    </Track>
  </Structure>
  <Arrangement id="id5">
    <TempoAutomation timeUnit="beats" id="id10">
      <Points id="id11">
        <RealPoint time="0" value="128" interpolation="hold"/>
        <RealPoint time="16" value="96" interpolation="hold"/>
      </Points>
    </TempoAutomation>
  </Arrangement>
  <Scenes/>
</Project>
"##;

    let (bytes, _) = zip_fixture(xml);
    let (project, _) = dawproject::import(&bytes, &no_plugins).expect("import succeeds");
    assert_eq!(project.bpm, 128.0);
    assert_eq!(
        project.tempo_map.len(),
        2,
        "the wrapped points are still read"
    );
}
