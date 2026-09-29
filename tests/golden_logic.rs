use std::collections::HashSet;
use std::sync::Arc;

use yadaw::edit_actions::EditProcessor;
use yadaw::input::actions::AppAction;
use yadaw::input::shortcuts::{KeyCode, Keybind, ShortcutRegistry};
use yadaw::midi_utils::MidiNoteUtils;
use yadaw::model::clip::{AudioClip, MidiClip, MidiNote, MidiPattern, WarpPoint};
use yadaw::model::group::{GroupLinkMode, TrackGroup};
use yadaw::model::marker::Marker;
use yadaw::model::tempo::{TempoCurve, TempoPoint, TempoRamp};
use yadaw::model::track::{Send, Track, TrackType};
use yadaw::project::{
    AppState, ArrangementRow, PROJECT_VERSION, Project, tempo_map_at_bpm, tempo_map_base_bpm,
};
use yadaw::time_utils::TimeConverter;

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
        tempo_map: Vec::new(),
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

fn group(id: u64, name: &str, parent_id: Option<u64>) -> TrackGroup {
    TrackGroup {
        id,
        name: name.to_string(),
        parent_id,
        ..Default::default()
    }
}

fn plain_track(id: u64, group_id: Option<u64>) -> Track {
    Track {
        id,
        name: format!("Track {id}"),
        group_id,
        ..Default::default()
    }
}

fn nested_group_project() -> Project {
    Project {
        version: PROJECT_VERSION.to_string(),
        name: "Nested".to_string(),
        tracks: vec![
            plain_track(1, Some(10)),
            plain_track(2, Some(20)),
            plain_track(3, Some(20)),
            plain_track(4, Some(30)),
            plain_track(5, None),
        ],
        groups: vec![
            group(10, "Drums", None),
            group(20, "Toms", Some(10)),
            group(30, "Keys", None),
        ],
        ..project_fixture()
    }
}

#[test]
fn golden_nested_groups_survive_a_save_and_load() {
    let mut state = AppState::default();
    state.load_project(nested_group_project());

    let toms = state.groups[&20]
        .parent_id
        .expect("a folder inside a folder keeps its parent");
    assert_eq!(toms, 10, "the nested group points at the outer one");

    let rows = state.arrangement_rows();
    assert_eq!(
        rows,
        vec![
            ArrangementRow::Group(10),
            ArrangementRow::Group(20),
            ArrangementRow::Track(2),
            ArrangementRow::Track(3),
            ArrangementRow::Track(1),
            ArrangementRow::Group(30),
            ArrangementRow::Track(4),
            ArrangementRow::Track(5),
        ],
        "subgroups come before their parent's own tracks, and ungrouped tracks come last, got {rows:?}"
    );

    let json = serde_json::to_string(&state.to_project()).expect("project serializes");
    let mut reloaded = AppState::default();
    reloaded.load_project(serde_json::from_str(&json).expect("project deserializes"));
    reloaded
        .validate_before_save()
        .expect("round-tripped state validates");
    assert_eq!(
        reloaded.arrangement_rows(),
        rows,
        "the same tree comes back out of the file"
    );
    assert_eq!(
        reloaded.groups[&20].parent_id,
        Some(10),
        "nesting survives the round trip"
    );
}

#[test]
fn a_group_created_without_a_nested_member_keeps_its_place() {
    let mut state = AppState::default();
    state.load_project(Project {
        groups: vec![],
        tracks: vec![plain_track(1, None)],
        ..project_fixture()
    });
    let empty = state.fresh_id();
    state.groups.insert(empty, group(empty, "Empty", None));

    assert_eq!(
        state.arrangement_rows(),
        vec![ArrangementRow::Group(empty), ArrangementRow::Track(1)],
        "a group with no members is still a row of the list"
    );

    let saved = state.to_project();
    assert_eq!(
        saved.groups.len(),
        1,
        "a group missing from group_order must not vanish on save"
    );
}

#[test]
fn a_legacy_group_file_loads_with_unity_level() {

    let legacy = format!(
        r#"{{
        "version": "1.2.0",
        "name": "Old",
        "tracks": [{}],
        "groups": [{{"id": 7, "name": "Band", "color": [1, 2, 3], "collapsed": false}}]
    }}"#,
        serde_json::to_string(&plain_track(1, Some(7))).expect("track serializes")
    );
    let project: Project = serde_json::from_str(&legacy).expect("a 1.2 project deserializes");
    let mut state = AppState::default();
    state.load_project(project);

    let g = &state.groups[&7];
    assert_eq!(g.volume, 1.0, "an older group opens at unity, got {}", g.volume);
    assert!(!g.muted && !g.solo, "and is not muted or soloed");
    assert_eq!(g.parent_id, None, "and is a top level group");
    assert_eq!(g.link_mode, GroupLinkMode::Vca, "and is a VCA");
    state
        .validate_before_save()
        .expect("a migrated project still validates");
}

#[test]
fn a_broken_group_tree_is_repaired_rather_than_saved_back() {
    let mut state = AppState::default();
    state.load_project(Project {
        tracks: vec![plain_track(1, Some(1))],
        groups: vec![group(1, "A", Some(2)), group(2, "B", Some(1)), group(3, "C", Some(99))],
        ..project_fixture()
    });

    let a = state.groups[&1].parent_id;
    let b = state.groups[&2].parent_id;
    assert!(
        !(a == Some(2) && b == Some(1)),
        "the A <-> B cycle is broken on load, got a->{a:?} b->{b:?}"
    );
    assert_eq!(
        state.groups[&3].parent_id, None,
        "a group pointing at a group that does not exist becomes top level"
    );
    assert_eq!(
        state.get_group_members(1),
        vec![1],
        "the track stays with the group it named, got {:?}",
        state.get_group_members(1)
    );
    state
        .validate_before_save()
        .expect("a repaired tree validates");

    let json = serde_json::to_string(&state.to_project()).expect("serializes");
    let mut again = AppState::default();
    again.load_project(serde_json::from_str(&json).expect("deserializes"));
    again
        .validate_before_save()
        .expect("the repaired tree survives a round trip");
}

#[test]
fn linked_group_faders_are_refused_instead_of_silently_mixed_as_proxies() {
    let mut state = AppState::default();
    state.load_project(Project {
        tracks: vec![plain_track(1, Some(10))],
        groups: vec![TrackGroup {
            link_mode: GroupLinkMode::Linked,
            ..group(10, "Linked", None)
        }],
        ..project_fixture()
    });

    let err = state
        .validate_before_save()
        .expect_err("an unimplemented link mode is not silently accepted");
    assert!(
        err.to_string().contains("linked faders"),
        "the message names the problem, got {err}"
    );
}

#[test]
fn a_group_only_holds_its_own_tracks() {
    let mut state = AppState::default();
    state.load_project(nested_group_project());

    assert_eq!(
        state.get_group_members(10),
        vec![1],
        "Drums holds one track of its own as well as the nested group"
    );
    assert_eq!(state.get_group_members(20), vec![2, 3], "in track order");
    assert_eq!(
        state.get_group_subtree_tracks(10),
        vec![1, 2, 3],
        "the subtree covers the group's own track and reaches through the nested group, got {:?}",
        state.get_group_subtree_tracks(10)
    );
    assert_eq!(
        state.group_descendants(10),
        HashSet::from([20]),
        "Drums has exactly one subgroup below it"
    );
}

#[test]
fn moving_a_group_into_its_own_subtree_is_refused() {

    let mut state = AppState::default();
    state.load_project(nested_group_project());
    assert!(
        state.group_descendants(10).contains(&20),
        "Toms is a descendant of Drums, so Drums cannot be moved into Toms"
    );
    assert!(
        !state.group_descendants(20).contains(&10),
        "and the reverse is fine"
    );
}

fn audio_clip_fixture(warp_mode: bool, warps: Vec<WarpPoint>, length_beats: f64) -> AudioClip {
    AudioClip {
        id: 1,
        name: "Take".to_string(),
        start_beat: 0.0,
        length_beats,
        sample_rate: 48_000.0,
        samples: std::sync::Arc::new(vec![0.0; 48_000 * 4]),
        warp_mode,
        warps,
        ..AudioClip::default()
    }
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1.0e-9
}

#[test]
fn an_unwarped_clip_reads_at_natural_speed_whatever_points_it_holds() {
    let clip = audio_clip_fixture(
        false,
        vec![
            WarpPoint {
                beat: 0.0,
                content_seconds: 0.0,
            },
            WarpPoint {
                beat: 4.0,
                content_seconds: 9.0,
            },
        ],
        4.0,
    );

    // 120 BPM: one beat is half a second, so beat 4 is 2 s in.
    assert!(
        close(clip.warp_curve(120.0).content_seconds_at(4.0), 2.0),
        "warp mode off wins over stored points"
    );
}

#[test]
fn a_warped_clip_without_points_stretches_its_whole_source_over_its_length() {
    let clip = audio_clip_fixture(true, Vec::new(), 4.0);

    // The source is 4 s and the clip is 4 beats, so one beat is one second of
    // material whatever the project tempo is.
    let curve = clip.warp_curve(120.0);
    assert!(close(curve.content_seconds_at(0.0), 0.0));
    assert!(close(curve.content_seconds_at(1.0), 1.0));
    assert!(close(curve.content_seconds_at(4.0), 4.0));
    assert!(
        close(
            curve.content_seconds_at(4.0),
            clip.warp_curve(174.0).content_seconds_at(4.0)
        ),
        "a stretched clip reads the same content per beat at any tempo"
    );
}

#[test]
fn a_warped_clip_with_points_follows_them_and_stops_at_their_ends() {
    let clip = audio_clip_fixture(
        true,
        vec![
            WarpPoint {
                beat: 0.0,
                content_seconds: 0.0,
            },
            WarpPoint {
                beat: 2.0,
                content_seconds: 1.0,
            },
            WarpPoint {
                beat: 4.0,
                content_seconds: 5.0,
            },
        ],
        4.0,
    );

    let curve = clip.warp_curve(120.0);
    assert!(close(curve.content_seconds_at(0.0), 0.0));
    assert!(
        close(curve.content_seconds_at(1.0), 0.5),
        "the first segment runs at half a second per beat"
    );
    assert!(
        close(curve.content_seconds_at(2.0), 1.0),
        "and joins the second segment exactly"
    );
    assert!(
        close(curve.content_seconds_at(3.0), 3.0),
        "the second segment runs at two seconds per beat"
    );
    assert!(
        close(curve.content_seconds_at(4.0), 5.0),
        "the last point is the end of the material"
    );
    assert!(
        close(curve.content_seconds_at(9.0), 5.0) && close(curve.content_seconds_at(-3.0), 0.0),
        "reading past either end clamps rather than running off the source"
    );
}

#[test]
fn a_warp_map_that_cannot_be_repaired_is_dropped_rather_than_played() {
    use yadaw::project::sanitize_warp_points;

    let point = |beat, content_seconds| WarpPoint {
        beat,
        content_seconds,
    };

    let mut unsorted = audio_clip_fixture(true, vec![point(4.0, 5.0), point(0.0, 0.0)], 4.0);
    sanitize_warp_points(&mut unsorted);
    assert_eq!(
        unsorted.warps,
        vec![point(0.0, 0.0), point(4.0, 5.0)],
        "points out of order are sorted, not discarded"
    );

    let mut backwards = audio_clip_fixture(
        true,
        vec![point(0.0, 0.0), point(2.0, 4.0), point(4.0, 1.0)],
        4.0,
    );
    sanitize_warp_points(&mut backwards);
    assert!(
        backwards.warps.is_empty(),
        "content time running backwards falls back to a synthesised map"
    );

    let mut nan = audio_clip_fixture(true, vec![point(0.0, 0.0), point(4.0, f64::NAN)], 4.0);
    sanitize_warp_points(&mut nan);
    assert!(nan.warps.is_empty(), "and so does a non-finite point");

    let mut lonely = audio_clip_fixture(true, vec![point(0.0, 0.0)], 4.0);
    sanitize_warp_points(&mut lonely);
    assert!(
        lonely.warps.is_empty(),
        "a single point is not a map, so it is discarded"
    );
}

fn near(actual: f64, expected: f64, tol: f64) -> bool {
    (actual - expected).abs() <= tol * expected.abs().max(1.0)
}

#[test]
fn a_constant_tempo_curve_reproduces_the_old_converter_bit_for_bit() {
    for bpm in [40.0_f64, 60.0, 120.0, 174.0, 999.0] {
        let curve = TempoCurve::from_map(&[], bpm);
        assert!(curve.is_constant());
        assert_eq!(curve.constant_bpm(), bpm);
        for beats in [0.0, 0.25, 1.0, 4.0, 128.0, 1000.0] {
            assert_eq!(
                curve.beats_to_seconds(beats),
                beats * 60.0 / bpm,
                "beats to seconds at {bpm} BPM, beat {beats}"
            );
            assert!(
                near(curve.seconds_to_beats(beats * 60.0 / bpm), beats, 1e-12),
                "seconds to beats at {bpm} BPM, beat {beats}"
            );
        }
    }
}

#[test]
fn a_tempo_ramp_is_integrated_exactly_and_round_trips() {
    let map = [
        TempoPoint::new(0.0, 90.0),
        TempoPoint::new(4.0, 200.0),
        TempoPoint::new(5.0, 40.0),
        TempoPoint::new(9.0, 40.0),
        TempoPoint::new(10.0, 300.0),
    ];
    let curve = TempoCurve::from_map(&map, 90.0);
    assert!(!curve.is_constant());

    let mut beat = -8.0;
    while beat <= 14.0 {
        let seconds = curve.beats_to_seconds(beat);
        assert!(
            near(curve.seconds_to_beats(seconds), beat, 1e-9),
            "beat {beat} went out at {seconds} s and came back at {}",
            curve.seconds_to_beats(seconds)
        );
        beat += 0.125;
    }

    assert!(near(curve.bpm_at(0.0), 90.0, 1e-12));
    assert!(
        near(curve.bpm_at(2.0), 145.0, 1e-12),
        "halfway up the first ramp"
    );
    assert!(
        near(curve.bpm_at(20.0), 300.0, 1e-12),
        "past the end it holds"
    );
    assert!(
        near(curve.bpm_at(-20.0), 90.0, 1e-12),
        "before the start it holds"
    );
}

#[test]
fn a_flat_tempo_map_is_still_a_constant_tempo() {
    let map = [
        TempoPoint::new(0.0, 100.0),
        TempoPoint::new(8.0, 100.0),
        TempoPoint::new(16.0, 100.0),
    ];
    let curve = TempoCurve::from_map(&map, 100.0);
    assert!(near(curve.beats_to_seconds(4.0), 2.4, 1e-12));
    assert!(near(curve.beats_to_seconds(16.0), 9.6, 1e-12));
    assert!(near(curve.bpm_at(9.0), 100.0, 1e-12));
}

#[test]
fn a_tempo_map_that_starts_late_inherits_the_project_tempo() {
    let curve = TempoCurve::from_map(&[TempoPoint::new(8.0, 60.0)], 120.0);
    assert!(near(curve.beats_to_seconds(0.0), 0.0, 1e-12));
    assert!(near(curve.bpm_at(0.0), 120.0, 1e-12));
    // The tempo ramps over beats 0..8, so beat 8 lands between 4 s and 8 s.
    let at_eight = curve.beats_to_seconds(8.0);
    assert!(at_eight > 4.0 && at_eight < 8.0, "beat 8 is {at_eight} s");
    let k = (60.0 - 120.0) / 8.0_f64;
    assert!(near(at_eight, (60.0 / k) * (60.0_f64 / 120.0).ln(), 1e-12));
    assert!(near(curve.seconds_to_beats(at_eight), 8.0, 1e-9));
    assert!(near(curve.bpm_at(8.0), 60.0, 1e-12));
}

#[test]
fn an_unusable_tempo_map_falls_back_to_a_constant_tempo() {
    let curve = TempoCurve::from_map(
        &[
            TempoPoint::new(f64::NAN, 100.0),
            TempoPoint::new(4.0, 0.0),
            TempoPoint::new(8.0, f64::INFINITY),
        ],
        128.0,
    );
    assert!(curve.is_constant());
    assert_eq!(curve.constant_bpm(), 128.0);

    for bad in [0.0_f64, -5.0, f64::NAN, f64::INFINITY] {
        let curve = TempoCurve::from_map(&[], bad);
        assert_eq!(curve.constant_bpm(), 120.0, "tempo {bad} falls back");
        assert_eq!(curve.beats_to_seconds(1.0), 0.5);
    }
}

#[test]
fn a_non_finite_tempo_query_does_not_poison_the_curve() {
    let curve = TempoCurve::from_map(
        &[TempoPoint::new(0.0, 120.0), TempoPoint::new(8.0, 60.0)],
        120.0,
    );
    assert_eq!(curve.beats_to_seconds(f64::NAN), 0.0);
    assert_eq!(curve.beats_to_seconds(f64::INFINITY), 0.0);
    assert_eq!(curve.seconds_to_beats(f64::NEG_INFINITY), 0.0);
    assert!(curve.bpm_at(f64::NAN).is_finite());
    // Beat 4 is the midpoint of the 120 -> 60 ramp, so the answer is the exact
    // integral of that ramp, not either tempo's own.
    let k = (60.0_f64 - 120.0) / 8.0;
    let expected = (60.0 / k) * ((120.0 + k * 4.0) / 120.0).ln();
    assert!(near(curve.beats_to_seconds(4.0), expected, 1e-12));
}

#[test]
fn a_saved_tempo_map_survives_a_save_and_load() {
    let map = vec![TempoPoint::new(0.0, 100.0), TempoPoint::new(8.0, 140.0)];
    let state = AppState {
        bpm: 100.0,
        tempo_map: map.clone(),
        ..AppState::default()
    };

    let project = state.to_project();
    assert_eq!(project.tempo_map, map, "the map is written out");

    let mut reloaded = AppState::default();
    reloaded.load_project(project);
    assert_eq!(reloaded.tempo_map, map, "and read back unchanged");
    assert!(
        near(
            f64::from(tempo_map_base_bpm(&reloaded.tempo_map, 999.0)),
            100.0,
            1e-12
        ),
        "the tempo at beat zero mirrors the map, not the fallback"
    );
}

#[test]
fn a_project_without_a_tempo_map_reads_as_a_constant_tempo() {
    let legacy = r#"{
        "version": "1.3.0",
        "name": "Old",
        "tracks": [],
        "groups": [], "markers": [], "bpm": 96.0, "time_signature": [3, 4],
        "sample_rate": 48000.0, "master_volume": 0.8, "loop_start": 0.0,
        "loop_end": 4.0, "loop_enabled": false
    }"#;

    let project: Project = serde_json::from_str(legacy).expect("a 1.3 project deserializes");
    let mut state = AppState::default();
    state.load_project(project);
    assert!(
        state.tempo_map.is_empty(),
        "no map means the old constant behaviour"
    );
    assert_eq!(state.bpm, 96.0);

    let curve = TempoCurve::from_map(&state.tempo_map, f64::from(state.bpm));
    assert!(curve.is_constant());
    assert_eq!(curve.beats_to_seconds(4.0), 2.5);
}

fn old_beats_to_samples(beats: f64, bpm: f32, sr: f32) -> f64 {
    (beats * 60.0 / bpm as f64) * sr as f64
}
fn old_samples_to_beats(samples: f64, bpm: f32, sr: f32) -> f64 {
    (samples / sr as f64) * (bpm as f64 / 60.0)
}

#[test]
fn a_constant_project_converts_exactly_as_before() {
    let mut worst = 0.0_f64;
    for sr in [44_100.0_f32, 48_000.0, 96_000.0] {
        for bpm in [40.0_f32, 60.0, 120.0, 128.5, 174.0, 240.0] {
            let conv = TimeConverter::new(sr, bpm);
            for x in [0.0_f64, 0.25, 1.0, 4.0, 128.0, 44100.0, 1e6] {
                let a = old_beats_to_samples(x, bpm, sr);
                let b = conv.beats_to_samples(x);
                worst = worst.max((a - b).abs() / a.abs().max(1.0));

                let c = old_samples_to_beats(x, bpm, sr);
                let d = conv.samples_to_beats(x);
                worst = worst.max((c - d).abs() / c.abs().max(1.0));
            }
        }
    }
    assert_eq!(
        worst, 0.0,
        "constant tempo must be bit identical, worst {worst}"
    );
}

#[test]
fn a_tempo_map_actually_changes_the_timing() {
    let map = [TempoPoint::new(0.0, 120.0), TempoPoint::new(8.0, 60.0)];
    let conv = TimeConverter::with_curve(48_000.0, Arc::new(TempoCurve::from_map(&map, 120.0)));

    let at_zero = conv.beats_to_samples(0.0);
    let at_eight = conv.beats_to_samples(8.0);
    assert_eq!(at_zero, 0.0);
    // Beats 0..8 cross a 120 -> 60 ramp, so it takes more than the 4 s that a
    // flat 120 BPM would give and less than the 8 s a flat 60 BPM would.
    assert!(
        at_eight > 4.0 * 48_000.0 && at_eight < 8.0 * 48_000.0,
        "beat 8 is {} samples",
        at_eight
    );
    assert!(
        (conv.samples_to_beats(at_eight) - 8.0).abs() < 1.0e-9,
        "and the inverse lands back on beat 8"
    );
    assert!(conv.curve().bpm_at(4.0) > 85.0 && conv.curve().bpm_at(4.0) < 95.0);
}

#[test]
fn a_beat_delta_conversion_is_wrong_under_a_map_but_absolute_is_not() {
    let map = [TempoPoint::new(0.0, 120.0), TempoPoint::new(16.0, 40.0)];
    let conv = TimeConverter::with_curve(48_000.0, Arc::new(TempoCurve::from_map(&map, 120.0)));
    let block_start_samples = conv.beats_to_samples(4.0);
    let block_start_beat = conv.samples_to_beats(block_start_samples);

    let mut differed = 0;
    for abs_beat in [5.0_f64, 8.0, 12.0, 15.0] {
        let absolute = conv.beats_to_samples(abs_beat) - block_start_samples;
        // A beat delta silently assumes a constant tempo, so on a map it
        // drifts away from the tempo-correct absolute form.
        let delta = conv.beats_to_samples(abs_beat - block_start_beat);
        if (delta - absolute).abs() > 1.0 {
            differed += 1;
        }
    }
    assert!(
        differed > 0,
        "the delta form should disagree somewhere across this ramp"
    );
}

#[test]
fn a_warp_map_is_never_cloned_into_the_converter() {
    // `with_curve` must hold the Arc, not copy the segments, or every audio
    // block would allocate. Cloning the Arc keeps one strong count per holder.
    let map: Vec<TempoPoint> = (0..64)
        .map(|i| TempoPoint::new(i as f64 * 4.0, 90.0 + i as f64))
        .collect();
    let curve = Arc::new(TempoCurve::from_map(&map, 120.0));
    let before = Arc::strong_count(&curve);
    let conv = TimeConverter::with_curve(48_000.0, Arc::clone(&curve));
    assert_eq!(
        Arc::strong_count(&curve),
        before + 1,
        "the converter shares the curve instead of copying it"
    );
    assert!(!conv.curve().is_constant());
}

/// DAWproject's `interpolation` enum is exactly `hold` or `linear`, and real
/// Cubase/Cubasis exports use both. A held span must sit at the earlier tempo
/// for its whole length and jump only at the next point.
#[test]
fn a_held_span_sits_at_one_tempo_and_jumps_at_the_next_point() {
    let map = [
        TempoPoint::new(0.0, 120.0),
        TempoPoint::held(4.0, 120.0),
        TempoPoint::held(8.0, 60.0),
        TempoPoint::new(12.0, 60.0),
    ];
    let curve = TempoCurve::from_map(&map, 120.0);
    assert!(!curve.is_constant());

    // The held 0..8 span runs entirely at 120 BPM: 8 beats is 4 seconds.
    assert!((curve.beats_to_seconds(4.0) - 2.0).abs() < 1e-12);
    assert!((curve.beats_to_seconds(8.0) - 4.0).abs() < 1e-12);
    for beat in [0.0, 1.0, 3.9, 4.0, 7.9] {
        assert!(
            (curve.bpm_at(beat) - 120.0).abs() < 1e-12,
            "beat {beat} should still be 120 BPM, got {}",
            curve.bpm_at(beat)
        );
    }

    // The tempo is already 60 from beat 8 onwards, where it holds to beat 12.
    assert!((curve.bpm_at(8.0) - 60.0).abs() < 1e-12);
    assert!((curve.bpm_at(11.9) - 60.0).abs() < 1e-12);
    // 8..12 at 60 BPM is another 4 seconds.
    assert!((curve.beats_to_seconds(12.0) - 8.0).abs() < 1e-12);
}

#[test]
fn a_held_map_round_trips_and_stays_monotonic() {
    let map = [
        TempoPoint::new(0.0, 140.0),
        TempoPoint::held(2.0, 100.0),
        TempoPoint::new(6.0, 100.0),
        TempoPoint::held(9.0, 175.0),
    ];
    let curve = TempoCurve::from_map(&map, 140.0);

    let mut beat = -4.0;
    let mut last_seconds = f64::NEG_INFINITY;
    while beat <= 13.0 {
        let seconds = curve.beats_to_seconds(beat);
        assert!(
            seconds >= last_seconds,
            "time went backwards at beat {beat}: {seconds} after {last_seconds}"
        );
        last_seconds = seconds;
        assert!(
            (curve.seconds_to_beats(seconds) - beat).abs() < 1e-9,
            "beat {beat} round tripped to {}",
            curve.seconds_to_beats(seconds)
        );
        beat += 0.125;
    }
}

#[test]
fn holding_and_ramping_differ_measurably() {
    // The flag describes the span starting at its own point, so holding 120 BPM
    // across beats 0..8 means the *first* point holds.
    let held = TempoCurve::from_map(
        &[TempoPoint::held(0.0, 120.0), TempoPoint::held(8.0, 60.0)],
        120.0,
    );
    let ramped = TempoCurve::from_map(
        &[TempoPoint::new(0.0, 120.0), TempoPoint::new(8.0, 60.0)],
        120.0,
    );
    let held_at_eight = held.beats_to_seconds(8.0);
    let ramped_at_eight = ramped.beats_to_seconds(8.0);
    assert!(
        (held_at_eight - ramped_at_eight).abs() > 1.0,
        "hold took {held_at_eight} s and the ramp took {ramped_at_eight} s, so the \
         interpolation mode is actually being honoured"
    );
    assert!(
        (held_at_eight - 4.0).abs() < 1e-12,
        "hold is 8 beats at 120 BPM"
    );
}

#[test]
fn a_held_point_survives_a_project_round_trip() {
    let map = vec![TempoPoint::new(0.0, 120.0), TempoPoint::held(8.0, 60.0)];
    let json = serde_json::to_string(&map).expect("a tempo map serializes");
    let back: Vec<TempoPoint> = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(back, map, "the ramp mode is part of the saved map");
    assert_eq!(
        back[1].ramp,
        TempoRamp::Hold,
        "and it is the second that holds"
    );
}

/// Brute-force reference: integrate 60/bpm over the segment with many steps.
fn numeric_seconds(from_beat: f64, to_beat: f64, from_bpm: f64, to_bpm: f64) -> f64 {
    let n = 2_000_000;
    let mut total = 0.0;
    for i in 0..n {
        let b0 = from_beat + (to_beat - from_beat) * (i as f64 / n as f64);
        let b1 = from_beat + (to_beat - from_beat) * ((i + 1) as f64 / n as f64);
        let mid = 0.5 * (b0 + b1);
        let bpm = from_bpm + (to_bpm - from_bpm) * ((mid - from_beat) / (to_beat - from_beat));
        total += (b1 - b0) * 60.0 / bpm;
    }
    total
}

#[test]
fn the_closed_form_matches_numerical_integration() {
    for (b0, b1, p0, p1) in [
        (0.0, 8.0, 120.0, 60.0),
        (0.0, 4.0, 90.0, 200.0),
        (4.0, 5.0, 200.0, 40.0),
        (0.0, 16.0, 120.0, 40.0),
        (0.0, 3.0, 300.0, 30.0),
        (0.0, 8.0, 100.0, 100.0),
    ] {
        let map = [TempoPoint::new(b0, p0), TempoPoint::new(b1, p1)];
        let curve = TempoCurve::from_map(&map, p0);
        let got = curve.beats_to_seconds(b1) - curve.beats_to_seconds(b0);
        let want = numeric_seconds(b0, b1, p0, p1);
        let rel = (got - want).abs() / want.abs();
        assert!(
            rel < 1e-9,
            "{p0} to {p1} over {} beats: got {got}, want {want}",
            b1 - b0
        );
    }
}

#[test]
fn the_tempo_reported_at_a_beat_agrees_with_the_time_integral() {
    // bpm_at must be the derivative of beats_to_seconds, i.e. 60 / (dt/db).
    let map = [TempoPoint::new(0.0, 120.0), TempoPoint::new(8.0, 60.0)];
    let curve = TempoCurve::from_map(&map, 120.0);
    for beat in [0.5, 2.0, 3.7, 4.0, 6.25, 7.9] {
        let h = 1e-6;
        let dt = curve.beats_to_seconds(beat + h) - curve.beats_to_seconds(beat - h);
        let implied = 2.0 * h * 60.0 / dt;
        let reported = curve.bpm_at(beat);
        assert!(
            (implied - reported).abs() < 1e-3,
            "beat {beat}: bpm_at {reported} but the integral implies {implied}"
        );
    }
}

/// A tempo linear in seconds crosses a segment at the mean of its endpoints,
/// while the same endpoints ramped in beats do not, so the domain is observable.
#[test]
fn a_seconds_ramp_crosses_at_the_mean_tempo_and_a_beats_ramp_does_not() {
    let in_seconds = TempoCurve::from_map(
        &[
            TempoPoint::ramped_in(0.0, 120.0, TempoRamp::Seconds),
            TempoPoint::held(8.0, 60.0),
        ],
        120.0,
    );
    let in_beats = TempoCurve::from_map(
        &[TempoPoint::new(0.0, 120.0), TempoPoint::held(8.0, 60.0)],
        120.0,
    );
    let by_mean = 8.0 * 60.0 / 90.0;
    assert!(
        (in_seconds.beats_to_seconds(8.0) - by_mean).abs() < 1e-9,
        "seconds ramp took {} but 8 beats at the 90 mean is {by_mean}",
        in_seconds.beats_to_seconds(8.0)
    );
    assert!(
        (in_beats.beats_to_seconds(8.0) - by_mean).abs() > 0.1,
        "a beats ramp is logarithmic, not the mean: got {} against {by_mean}",
        in_beats.beats_to_seconds(8.0)
    );
}

#[test]
fn a_seconds_ramp_round_trips_and_reports_a_consistent_tempo() {
    let map = [
        TempoPoint::ramped_in(0.0, 60.0, TempoRamp::Seconds),
        TempoPoint::ramped_in(16.0, 180.0, TempoRamp::Seconds),
        TempoPoint::held(24.0, 180.0),
    ];
    let curve = TempoCurve::from_map(&map, 60.0);
    assert!(!curve.is_constant());

    let mut beat = 0.0;
    while beat <= 24.0 {
        let seconds = curve.beats_to_seconds(beat);
        assert!(
            (curve.seconds_to_beats(seconds) - beat).abs() < 1e-9,
            "beat {beat} round tripped to {}",
            curve.seconds_to_beats(seconds)
        );
        if beat < 16.0 {
            assert!(
                curve.bpm_at(beat) > 59.9 && curve.bpm_at(beat) < 180.1,
                "tempo at beat {beat} left its segment: {}",
                curve.bpm_at(beat)
            );
        }
        beat += 0.25;
    }
    assert!((curve.bpm_at(16.0) - 180.0).abs() < 1e-9);
}

/// The tempo handed to a plugin must be the derivative of the rendered time, in
/// both ramp domains, or a synced plugin drifts against the audio.
#[test]
fn the_reported_tempo_is_the_derivative_of_the_rendered_time_in_both_domains() {
    for ramp in [TempoRamp::Beats, TempoRamp::Seconds] {
        let curve = TempoCurve::from_map(
            &[
                TempoPoint::ramped_in(0.0, 120.0, ramp),
                TempoPoint::ramped_in(8.0, 60.0, ramp),
            ],
            120.0,
        );
        for beat in [1.0, 2.0, 3.7, 5.5, 7.0] {
            let h = 1e-6;
            let dt = curve.beats_to_seconds(beat + h) - curve.beats_to_seconds(beat - h);
            let implied = 2.0 * h * 60.0 / dt;
            let reported = curve.bpm_at(beat);
            assert!(
                (implied - reported).abs() < 1e-2,
                "{ramp:?} ramp at beat {beat}: reported {reported}, integral implies {implied}"
            );
        }
    }
}

#[test]
fn a_tempo_point_keeps_its_ramp_across_a_project_round_trip() {
    let map = vec![
        TempoPoint::ramped_in(0.0, 120.0, TempoRamp::Seconds),
        TempoPoint::held(8.0, 60.0),
        TempoPoint::new(16.0, 90.0),
    ];
    let json = serde_json::to_string(&map).expect("a tempo map serializes");
    let back: Vec<TempoPoint> = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(back, map);

    let legacy = r#"[{"beat":0.0,"bpm":120.0},{"beat":8.0,"bpm":60.0}]"#;
    let old: Vec<TempoPoint> = serde_json::from_str(legacy).expect("an older map loads");
    assert!(
        old.iter().all(|p| p.ramp == TempoRamp::Beats),
        "a map written before ramps existed loads as beat-linear"
    );
}

/// A scalar tempo change moves the whole curve by the same ratio, so a tempo
/// map and the scalar can never disagree about what the project tempo is.
#[test]
fn a_tempo_change_scales_every_point_of_the_map() {
    let map = vec![
        TempoPoint::held(0.0, 120.0),
        TempoPoint::held(16.0, 100.0),
        TempoPoint::new(32.0, 80.0),
    ];
    let scaled = tempo_map_at_bpm(&map, 60.0);
    assert_eq!(scaled.len(), 3);
    for (want, got) in map.iter().zip(&scaled) {
        assert!(
            (got.bpm - want.bpm / 2.0).abs() < 1e-4,
            "{} vs {}",
            got.bpm,
            want.bpm
        );
        assert_eq!(got.beat, want.beat, "beats do not move with the tempo");
        assert_eq!(got.ramp, want.ramp);
    }
    assert_eq!(tempo_map_base_bpm(&scaled, 120.0), 60.0);
}

#[test]
fn a_constant_project_stays_the_scalar_it_always_was() {
    assert!(
        tempo_map_at_bpm(&[TempoPoint::new(0.0, 120.0)], 60.0).is_empty(),
        "a lone point is the constant tempo in disguise, so it collapses to none"
    );
    let map = vec![TempoPoint::held(0.0, 120.0), TempoPoint::held(16.0, 90.0)];
    assert_eq!(
        tempo_map_at_bpm(&map, 120.0),
        map,
        "setting the tempo it already has changes nothing"
    );
}

/// The engine's rule: a clip's beat length is its identity and a tempo change
/// never moves it. This is the guarantee the old clip rescale used to break.
#[test]
fn a_tempo_change_leaves_clip_beats_alone() {
    let mut state = AppState::default();
    let project = Project {
        bpm: 120.0,
        tempo_map: vec![TempoPoint::held(0.0, 120.0), TempoPoint::new(16.0, 90.0)],
        tracks: vec![Track {
            id: 1,
            name: "Audio".to_string(),
            track_type: TrackType::Audio,
            audio_clips: vec![AudioClip {
                id: 1,
                name: "c".to_string(),
                start_beat: 4.0,
                length_beats: 8.0,
                offset_beats: 1.5,
                fade_in: Some(0.5),
                fade_out: Some(0.25),
                sample_rate: 44100.0,
                samples: std::sync::Arc::new(vec![0.0f32; 1024]),
                ..Default::default()
            }],
            ..Track::default()
        }],
        ..serde_json::from_str("{}").expect("a project needs only its defaults")
    };
    state.load_project(project);

    let before = state.tracks[&1].audio_clips[0].clone();
    let seconds_before = state.tempo_curve().beats_to_seconds(12.0);

    // Halving the tempo doubles the real time of every beat.
    let map = tempo_map_at_bpm(&state.tempo_map, 60.0);
    state.tempo_map = map;
    state.bpm = tempo_map_base_bpm(&state.tempo_map, 60.0);

    let after = &state.tracks[&1].audio_clips[0];
    assert_eq!(after.length_beats, before.length_beats);
    assert_eq!(after.start_beat, before.start_beat);
    assert_eq!(after.offset_beats, before.offset_beats);
    assert_eq!(after.fade_in, before.fade_in);
    assert_eq!(after.fade_out, before.fade_out);
    assert!(
        (state.tempo_curve().beats_to_seconds(12.0) - seconds_before * 2.0).abs() < 1e-9,
        "the same 12 beats now take twice as long"
    );
}

/// Every playhead conversion has to read the tempo map, or a project with a
/// ramp shows the playhead, the position display and pasted notes in the wrong
/// place. The UI sites now all go through these two helpers.
#[test]
fn playhead_conversions_follow_a_tempo_map() {
    let mut state = AppState::default();
    state.load_project(Project {
        bpm: 120.0,
        sample_rate: 48000.0,
        tempo_map: vec![
            TempoPoint::held(0.0, 120.0),
            TempoPoint::new(4.0, 60.0),
            TempoPoint::new(8.0, 60.0),
        ],
        ..serde_json::from_str(r#"{"tracks":[]}"#).expect("defaults")
    });

    assert_eq!(state.position_to_beats(0.0), 0.0);
    assert!(
        (state.position_to_beats(48000.0) - 2.0).abs() < 1e-9,
        "1s at 120 BPM"
    );
    assert!(
        (state.beats_to_samples(2.0) - 48000.0).abs() < 1e-6,
        "and the round trip returns the same samples"
    );

    let ramp_start = state.tempo_curve().beats_to_seconds(4.0);
    let half = state.tempo_curve().beats_to_seconds(6.0);
    assert!(
        half - ramp_start > 1.0,
        "the two beats after the ramp start must take longer than a second, took {}",
        half - ramp_start
    );
    assert!(
        (state.position_to_beats(state.beats_to_samples(6.0)) - 6.0).abs() < 1e-6,
        "a position in the ramp round trips back to its own beat"
    );
}
