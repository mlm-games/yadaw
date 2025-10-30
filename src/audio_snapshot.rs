//! Build immutable audio snapshots from UI/project model types.
use std::sync::Arc;

use dashmap::DashMap;

use crate::{
    audio_state::{
        AudioClipSnapshot, MidiClipSnapshot, MidiNoteSnapshot, PluginDescriptorSnapshot,
        RtAutomationLaneSnapshot, RtAutomationPoint, RtAutomationTarget, RtCurveType,
        TrackSnapshot,
    },
    model::{
        clip::{AudioClip, MidiClip, MidiNote},
        plugin::PluginDescriptor,
        track::Track,
    },
    project::AppState,
};

/// Build snapshots
pub fn build_track_snapshots(state: &AppState) -> Vec<TrackSnapshot> {
    state
        .track_order
        .iter()
        .filter_map(|&id| state.tracks.get(&id))
        .map(track_to_snapshot)
        .collect()
}

fn track_to_snapshot(t: &Track) -> TrackSnapshot {
    TrackSnapshot {
        track_id: t.id,
        name: t.name.clone(),
        volume: t.volume,
        pan: t.pan,
        muted: t.muted,
        solo: t.solo,
        armed: t.armed,
        track_type: t.track_type,
        monitor_enabled: t.monitor_enabled,
        audio_clips: t.audio_clips.iter().map(audio_clip_to_snapshot).collect(),
        midi_clips: t.midi_clips.iter().map(midi_clip_to_snapshot).collect(),
        plugin_chain: t.plugin_chain.iter().map(plugin_desc_to_snapshot).collect(),
        automation_lanes: t
            .automation_lanes
            .iter()
            .map(automation_lane_to_snapshot)
            .collect(),
        sends: t.sends.clone(),
    }
}

fn audio_clip_to_snapshot(c: &AudioClip) -> AudioClipSnapshot {
    AudioClipSnapshot {
        name: c.name.clone(),
        start_beat: c.start_beat,
        length_beats: c.length_beats,
        offset_beats: c.offset_beats,
        samples: c.samples.clone(),
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
        content_len_beats: if c.content_len_beats > 0.0 {
            c.content_len_beats
        } else {
            c.length_beats.max(0.000001)
        },
        loop_enabled: c.loop_enabled,
        notes: c.notes.iter().map(midi_note_to_snapshot).collect(),
        color: c.color,
        transpose: c.transpose,
        velocity_offset: c.velocity_offset,
        quantize_enabled: c.quantize_enabled,
        quantize_grid: c.quantize_grid.max(0.0),
        quantize_strength: c.quantize_strength.clamp(0.0, 1.0),
        swing: c.swing,
        humanize: c.humanize,
        content_offset_beats: if c.content_len_beats > 0.0 {
            ((c.content_offset_beats % c.content_len_beats) + c.content_len_beats)
                % c.content_len_beats
        } else {
            0.0
        },
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
        plugin_id: p.id,
        uri: p.uri.clone(),
        name: p.name.clone(),
        backend: p.backend,
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
                plugin_id,
                param_name,
            } => RtAutomationTarget::PluginParam {
                plugin_id: *plugin_id,
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
