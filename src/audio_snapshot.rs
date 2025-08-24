//! Build immutable audio snapshots from UI/project model types.
//! Call this on the UI thread and send to the audio thread.
use std::sync::Arc;

use dashmap::DashMap;

use crate::{
    audio_state::{
        AudioClipSnapshot, MidiClipSnapshot, MidiNoteSnapshot, PluginDescriptorSnapshot,
        RtAutomationLaneSnapshot, RtAutomationPoint, RtAutomationTarget, RtCurveType,
        TrackSnapshot,
    },
    automation::AutomationPoint,
    model::{
        automation::{AutomationLane, AutomationTarget},
        clip::{AudioClip, MidiClip, MidiNote},
        plugin::PluginDescriptor,
        track::Track,
    },
};

pub fn build_track_snapshots(tracks: &[Track]) -> Vec<TrackSnapshot> {
    tracks.iter().map(track_to_snapshot).collect()
}

fn track_to_snapshot(t: &Track) -> TrackSnapshot {
    TrackSnapshot {
        name: t.name.clone(),
        volume: t.volume,
        pan: t.pan,
        muted: t.muted,
        solo: t.solo,
        armed: t.armed,
        is_midi: t.is_midi,
        monitor_enabled: t.monitor_enabled,
        audio_clips: t.audio_clips.iter().map(audio_clip_to_snapshot).collect(),
        midi_clips: t.midi_clips.iter().map(midi_clip_to_snapshot).collect(),
        plugin_chain: t.plugin_chain.iter().map(plugin_desc_to_snapshot).collect(),
        automation_lanes: t
            .automation_lanes
            .iter()
            .map(automation_lane_to_snapshot)
            .collect(),
    }
}

fn audio_clip_to_snapshot(c: &AudioClip) -> AudioClipSnapshot {
    AudioClipSnapshot {
        name: c.name.clone(),
        start_beat: c.start_beat,
        length_beats: c.length_beats,
        samples: c.samples.clone(), // consider ref-counted pool if large
        sample_rate: c.sample_rate,
        fade_in: c.fade_in,
        fade_out: c.fade_out,
        gain: c.gain,
    }
}

fn midi_clip_to_snapshot(c: &MidiClip) -> MidiClipSnapshot {
    MidiClipSnapshot {
        name: c.name.clone(),
        start_beat: c.start_beat,
        length_beats: c.length_beats,
        notes: c.notes.iter().map(midi_note_to_snapshot).collect(),
        color: c.color,
    }
}

fn midi_note_to_snapshot(n: &MidiNote) -> MidiNoteSnapshot {
    MidiNoteSnapshot {
        pitch: n.pitch,
        velocity: n.velocity,
        start: n.start,
        duration: n.duration,
    }
}

fn plugin_desc_to_snapshot(p: &PluginDescriptor) -> PluginDescriptorSnapshot {
    let params = Arc::new(DashMap::new());
    for (k, v) in &p.params {
        params.insert(k.clone(), *v);
    }
    PluginDescriptorSnapshot {
        uri: p.uri.clone(),
        name: p.name.clone(),
        bypass: p.bypass,
        params,
    }
}

fn automation_lane_to_snapshot(
    l: &crate::model::automation::AutomationLane,
) -> RtAutomationLaneSnapshot {
    RtAutomationLaneSnapshot {
        parameter: match &l.parameter {
            crate::model::automation::AutomationTarget::TrackVolume => {
                RtAutomationTarget::TrackVolume
            }
            crate::model::automation::AutomationTarget::TrackPan => RtAutomationTarget::TrackPan,
            crate::model::automation::AutomationTarget::TrackSend(i) => {
                RtAutomationTarget::TrackSend(*i)
            }
            crate::model::automation::AutomationTarget::PluginParam {
                plugin_idx,
                param_name,
            } => RtAutomationTarget::PluginParam {
                plugin_idx: *plugin_idx,
                param_name: param_name.clone(),
            },
        },
        points: l
            .points
            .iter()
            .map(|p| RtAutomationPoint {
                beat: p.beat,
                value: p.value,
                curve_type: RtCurveType::Linear,
            })
            .collect(),
        visible: l.visible,
        height: l.height,
        color: l.color,
    }
}
