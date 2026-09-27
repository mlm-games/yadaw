use std::collections::HashSet;

use yadaw::edit_actions::EditProcessor;
use yadaw::input::actions::AppAction;
use yadaw::input::shortcuts::{KeyCode, Keybind, ShortcutRegistry};
use yadaw::midi_utils::MidiNoteUtils;
use yadaw::model::clip::{MidiClip, MidiNote, MidiPattern};
use yadaw::model::group::TrackGroup;
use yadaw::model::marker::Marker;
use yadaw::model::track::{Send, Track, TrackType};
use yadaw::project::{AppState, PROJECT_VERSION, Project};

fn note(id: u64, pitch: u8, start: f64) -> MidiNote {
    MidiNote {
        id,
        pitch,
        velocity: 100,
        start,
        duration: 0.5,
    }
}

fn project_fixture() -> Project {
    let track_a = Track {
        id: 1,
        name: "Keys".to_string(),
        track_type: TrackType::Midi,
        midi_clips: vec![MidiClip {
            id: 11,
            name: "Verse".to_string(),
            start_beat: 0.0,
            length_beats: 4.0,
            pattern_id: Some(101),
            content_len_beats: 4.0,
            ..Default::default()
        }],
        sends: vec![Send {
            destination_track: 2,
            amount: 0.5,
            pre_fader: false,
            muted: false,
        }],
        group_id: Some(201),
        ..Default::default()
    };
    let track_b = Track {
        id: 2,
        name: "Bus".to_string(),
        track_type: TrackType::Bus,
        ..Default::default()
    };
    let pattern = MidiPattern {
        id: 101,
        notes: vec![note(1001, 60, 0.0), note(1002, 64, 1.0)],
    };
    let dup_pattern = MidiPattern {
        id: 101,
        notes: vec![note(1003, 67, 2.0)],
    };
    let group = TrackGroup {
        id: 201,
        name: "Band".to_string(),
        ..Default::default()
    };
    Project {
        version: PROJECT_VERSION.to_string(),
        name: "Golden Song".to_string(),
        tracks: vec![track_a, track_b.clone(), track_b.clone()],
        patterns: vec![pattern, dup_pattern],
        groups: vec![group],
        markers: vec![Marker {
            id: 900,
            beat: 8.0,
            name: "Chorus".to_string(),
            color: Some((241, 196, 15)),
            comment: None,
        }],
        bpm: 120.0,
        time_signature: (4, 4),
        sample_rate: 44100.0,
        master_volume: 0.8,
        loop_start: 0.0,
        loop_end: 16.0,
        loop_enabled: true,
        created_at: chrono::Utc::now(),
        modified_at: chrono::Utc::now(),
    }
}

#[test]
fn golden_project_lifecycle() {
    let mut state = AppState::default();
    state.load_project(project_fixture());

    let track_ids: HashSet<u64> = state.track_order.iter().copied().collect();
    assert_eq!(track_ids.len(), 3, "duplicate track id must be renumbered");
    assert_eq!(state.project_name, "Golden Song");
    assert!(state.patterns.len() == 2, "duplicate pattern id renumbered");
    let clip_pids: Vec<u64> = state
        .track_order
        .iter()
        .filter_map(|id| state.tracks.get(id))
        .flat_map(|t| t.midi_clips.iter().filter_map(|c| c.pattern_id))
        .collect();
    for pid in &clip_pids {
        assert!(
            state.patterns.contains_key(pid),
            "clip pattern {pid} must survive renumber"
        );
    }
    for track in state.tracks.values() {
        if let Some(gid) = track.group_id {
            assert!(state.groups.contains_key(&gid));
        }
        for send in &track.sends {
            assert!(
                state.tracks.contains_key(&send.destination_track),
                "send must point at a live track"
            );
        }
    }
    state
        .validate_before_save()
        .expect("golden state validates");

    assert_eq!(state.markers.len(), 1, "markers load from the project file");
    assert_eq!(state.markers[0].name, "Chorus");

    let snap = state.snapshot();
    let victim = state.track_order[0];
    state.tracks.remove(&victim);
    state.markers.clear();
    state.restore(snap);
    assert!(state.tracks.contains_key(&victim));
    assert_eq!(state.markers.len(), 1, "markers take part in undo");
    state
        .validate_before_save()
        .expect("restored state validates");

    let saved = state.to_project();
    assert_eq!(saved.name, "Golden Song");
    assert_eq!(saved.version, PROJECT_VERSION);
    let json = serde_json::to_string(&saved).expect("project serializes");
    let reparsed: Project = serde_json::from_str(&json).expect("project deserializes");
    let mut reloaded = AppState::default();
    reloaded.load_project(reparsed);
    reloaded
        .validate_before_save()
        .expect("round-tripped state validates");
    assert_eq!(reloaded.project_name, "Golden Song");
    assert_eq!(
        reloaded.markers.len(),
        1,
        "markers survive the save and load round trip"
    );
    assert_eq!(reloaded.markers[0].beat, 8.0);
    assert_eq!(reloaded.markers[0].color, Some((241, 196, 15)));

    let mut notes = vec![note(1, 60, 0.13), note(2, 62, 0.37)];
    EditProcessor::quantize_notes(&mut notes, 0.25, 1.0);
    assert!((notes[0].start - 0.25).abs() < 1e-9);
    assert!((notes[1].start - 0.25).abs() < 1e-9);
    let before = notes.clone();
    EditProcessor::quantize_notes(&mut notes, 0.0, 1.0);
    assert_eq!(notes[0].start, before[0].start);
    assert_eq!(notes[1].start, before[1].start);
    EditProcessor::transpose_notes(&mut notes, 200);
    assert!(notes.iter().all(|n| n.pitch == 127));

    assert_eq!(MidiNoteUtils::from_name("C4"), Some(60));
    assert_eq!(MidiNoteUtils::from_name(""), None);
    assert_eq!(MidiNoteUtils::from_name("é4"), None);
    assert_eq!(MidiNoteUtils::from_name("H4"), None);

    let reg = ShortcutRegistry::default_bindings();
    let mut seen = HashSet::new();
    for binds in reg.bindings.values() {
        for b in binds {
            assert!(seen.insert(*b), "duplicate keybind {b:?}");
        }
    }
    assert_eq!(
        reg.get_action(&Keybind::none(KeyCode::K)),
        Some(AppAction::FastForward)
    );
    assert_eq!(
        reg.get_action(&Keybind::none(KeyCode::L)),
        Some(AppAction::ToggleLoop)
    );
    let uri = yadaw_plugin_host::plugin_facade::validate_plugin_uri(
        yadaw_plugin_api::BackendKind::Lv2,
        "http://example.test/synth",
    )
    .expect("LV2 uri passes");
    assert!(
        yadaw_plugin_host::plugin_facade::validate_plugin_uri(
            yadaw_plugin_api::BackendKind::Lv2,
            "file:///etc/evil.so#x",
        )
        .is_err()
    );
    assert!(
        yadaw_plugin_host::plugin_facade::validate_plugin_uri(
            yadaw_plugin_api::BackendKind::Clap,
            "relative/path.clap#id",
        )
        .is_err()
    );
    let _ = uri;
}

#[test]
fn golden_config_hardening() {
    let mut cfg = yadaw::config::Config::default();
    cfg.audio.buffer_size = 0;
    cfg.audio.sample_rate = f32::NAN;
    cfg.behavior.auto_save_interval_minutes = 999;
    cfg.validate();
    assert_eq!(cfg.audio.buffer_size, 512);
    assert_eq!(cfg.audio.sample_rate, 44100.0);
    assert_eq!(cfg.behavior.auto_save_interval_minutes, 5);

    let dir = std::env::temp_dir().join("yadaw_golden_config");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("config.json");
    yadaw::wasm_persist::save_config_string("config/config.json", &path, "{\"audio\":{}}")
        .expect("config write works");
    let back =
        yadaw::wasm_persist::read_config_string("config/config.json", &path).expect("config read");
    assert!(back.contains("audio"));
    let _ = std::fs::remove_dir_all(&dir);
}
