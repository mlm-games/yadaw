use std::sync::Arc;

use yadaw::dawproject;
use yadaw::model::automation::{AutomationLane, AutomationMode, AutomationPoint, AutomationTarget};
use yadaw::model::clip::{AudioClip, MidiClip, MidiNote};
use yadaw::model::plugin::PluginDescriptor;
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
        groups: Vec::new(),
        bpm: 140.0,
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

    let (again, _) = dawproject::export(&imported).expect("re-export succeeds");
    let (reimported, _) = dawproject::import(&again, &no_plugins).expect("re-import succeeds");
    assert_eq!(reimported.tracks.len(), imported.tracks.len());
    assert_eq!(reimported.bpm, imported.bpm);
    assert_eq!(reimported.tracks[0].midi_clips[0].notes.len(), 2);
    assert_eq!(
        reimported.tracks[1].audio_clips[0].samples.len(),
        imported.tracks[1].audio_clips[0].samples.len()
    );
}

#[test]
fn dawproject_imports_bitwig_style_alias_and_nested_clips() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Bitwig Studio" version="5.0"/>
  <Transport>
    <Tempo max="666.000000" min="20.000000" unit="bpm" value="149.000000" id="id0" name="Tempo"/>
    <TimeSignature denominator="4" numerator="4" id="id1"/>
  </Transport>
  <Structure>
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
"#;

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

fn zip_fixture(project_xml: &str) -> (Vec<u8>, String) {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("project.xml", options).unwrap();
        writer.write_all(project_xml.as_bytes()).unwrap();
        writer.finish().unwrap();
    }
    (cursor.into_inner(), project_xml.to_string())
}
