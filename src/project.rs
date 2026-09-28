use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::constants::DEFAULT_LOOP_LEN;
use crate::model::clip::{AudioClip, MidiPattern, WarpPoint};
use crate::model::{GroupLinkMode, Marker, Track, TrackGroup};
use crate::time_utils::TimeConverter;

/// Current on-disk project schema version.
pub const PROJECT_VERSION: &str = "1.3.0";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppState {
    /// ID-based storage (canonical)
    pub tracks: HashMap<u64, Track>,
    /// Display order (references track IDs)
    pub track_order: Vec<u64>,
    /// Global clip registry for fast lookup
    pub clips_by_id: HashMap<u64, ClipRef>,
    /// Shared MIDI patterns (for alias clips)
    pub patterns: HashMap<u64, MidiPattern>,
    pub groups: HashMap<u64, TrackGroup>,
    /// Group header order, mirroring `track_order` for groups.
    #[serde(default)]
    pub group_order: Vec<u64>,
    /// Kept sorted by beat so the timeline can draw them in order.
    pub markers: Vec<Marker>,

    pub master_volume: f32,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub sample_rate: f32,
    pub buffer_size: usize,
    pub current_position: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub time_signature: (i32, i32),
    pub next_id: u64,
    /// Project name / creation time carried across save/load so
    /// `to_project()` no longer resets them every save.
    #[serde(default = "default_project_name")]
    pub project_name: String,
    #[serde(default = "default_now")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[inline]
fn default_project_name() -> String {
    "Untitled Project".to_string()
}

#[inline]
fn default_now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

/// Reference to where a clip lives
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipRef {
    pub track_id: u64,
    pub is_midi: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            tracks: HashMap::new(),
            track_order: Vec::new(),
            clips_by_id: HashMap::new(),
            patterns: HashMap::new(),
            groups: HashMap::new(),
            group_order: Vec::new(),
            markers: Vec::new(),
            master_volume: 0.8,
            playing: false,
            recording: false,
            bpm: 120.0,
            sample_rate: 44100.0,
            buffer_size: 512,
            current_position: 0.0,
            loop_start: 0.0,
            loop_end: DEFAULT_LOOP_LEN,
            loop_enabled: false,
            time_signature: (4, 4),
            next_id: 1,
            project_name: default_project_name(),
            created_at: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub tracks: HashMap<u64, Track>,
    pub track_order: Vec<u64>,
    pub master_volume: f32,
    pub patterns: HashMap<u64, MidiPattern>,
    pub groups: HashMap<u64, TrackGroup>,
    pub group_order: Vec<u64>,
    pub markers: Vec<Marker>,
    pub bpm: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub sample_rate: f32,
    pub time_signature: (i32, i32),
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub recording: bool,
}

impl AppState {
    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            tracks: self.tracks.clone(),
            track_order: self.track_order.clone(),
            patterns: self.patterns.clone(),
            groups: self.groups.clone(),
            group_order: self.group_order.clone(),
            markers: self.markers.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            playing: false,
            recording: false,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
        }
    }

    pub fn restore(&mut self, snapshot: AppStateSnapshot) {
        self.tracks = snapshot.tracks;
        self.track_order = snapshot.track_order;
        self.patterns = snapshot.patterns;
        self.groups = snapshot.groups;
        self.group_order = snapshot.group_order;
        self.markers = snapshot.markers;
        self.bpm = snapshot.bpm;
        self.time_signature = snapshot.time_signature;
        self.sample_rate = snapshot.sample_rate;
        self.master_volume = snapshot.master_volume;
        self.loop_start = snapshot.loop_start;
        self.loop_end = snapshot.loop_end;
        self.loop_enabled = snapshot.loop_enabled;
        self.track_order.retain(|id| self.tracks.contains_key(id));
        sanitize_group_forest(&mut self.groups);
        self.group_order.retain(|id| self.groups.contains_key(id));
        let track_ids: Vec<u64> = self.tracks.keys().copied().collect();
        for track in self.tracks.values_mut() {
            for clip in &mut track.audio_clips {
                sanitize_warp_points(clip);
            }
            if track
                .group_id
                .is_some_and(|g| !self.groups.contains_key(&g))
            {
                track.group_id = None;
            }
            for send in &mut track.sends {
                if send.destination_track != 0 && !track_ids.contains(&send.destination_track) {
                    send.destination_track = 0;
                }
            }
        }
        self.append_unlisted_groups();
        self.rebuild_clip_index();
        self.rebuild_plugin_indices();
        crate::idgen::seed_from_max(self.max_id_in_project());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangementRow {
    Group(u64),
    Track(u64),
}

/// Puts a clip's warp map back into the shape the audio thread assumes: sorted
/// by beat, no repeated beats, and content time never running backwards. A map
/// that cannot be repaired is dropped so the clip falls back to a synthesised one.
pub fn sanitize_warp_points(clip: &mut AudioClip) {
    let mut points = std::mem::take(&mut clip.warps);
    if points.len() < 2 {
        return;
    }
    if points
        .iter()
        .any(|p| !p.beat.is_finite() || !p.content_seconds.is_finite())
    {
        log::warn!("Warp map holds a non-finite point; dropped");
        return;
    }
    points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    let mut kept: Vec<WarpPoint> = Vec::with_capacity(points.len());
    for point in points {
        match kept.last_mut() {
            Some(last) if last.beat == point.beat => *last = point,
            Some(last) if point.content_seconds < last.content_seconds => {
                log::warn!(
                    "Warp map runs content time backwards at beat {}; dropped",
                    point.beat
                );
                return;
            }
            _ => kept.push(point),
        }
    }
    if kept.len() >= 2 {
        clip.warps = kept;
    }
}

/// Drops dead parents and breaks cycles; each pass severs one cycle edge.
pub fn sanitize_group_forest(groups: &mut HashMap<u64, TrackGroup>) {
    loop {
        let mut severed = false;
        let ids: Vec<u64> = groups.keys().copied().collect();
        for gid in ids {
            let mut chain: Vec<u64> = vec![gid];
            let mut cursor = groups[&gid].parent_id;
            while let Some(pid) = cursor {
                if let Some(pos) = chain.iter().position(|&c| c == pid) {
                    if let Some(entry) = groups.get_mut(&chain[pos]) {
                        entry.parent_id = None;
                        severed = true;
                    }
                    break;
                }
                let Some(parent) = groups.get(&pid) else {
                    if let Some(g) = groups.get_mut(&gid) {
                        g.parent_id = None;
                        severed = true;
                    }
                    break;
                };
                chain.push(pid);
                cursor = parent.parent_id;
            }
        }
        if !severed {
            return;
        }
    }
}

impl AppState {

    pub fn append_unlisted_groups(&mut self) {
        self.group_order = self.ordered_group_ids();
    }

    pub fn ordered_group_ids(&self) -> Vec<u64> {
        let mut order: Vec<u64> = self
            .group_order
            .iter()
            .copied()
            .filter(|id| self.groups.contains_key(id))
            .collect();
        let listed: HashSet<u64> = order.iter().copied().collect();
        let mut missing: Vec<u64> = self
            .groups
            .keys()
            .copied()
            .filter(|id| !listed.contains(id))
            .collect();
        missing.sort_unstable();
        order.extend(missing);
        order
    }

    pub fn arrangement_rows(&self) -> Vec<ArrangementRow> {
        let order = self.ordered_group_ids();
        let mut rows = Vec::with_capacity(order.len() + self.track_order.len());
        let mut walked: HashSet<u64> = HashSet::new();
        for &gid in &order {
            self.push_group_rows(gid, &order, &mut rows, &mut walked);
        }
        for &tid in &self.track_order {
            if self.tracks.contains_key(&tid) && !self.track_in_group(tid) {
                rows.push(ArrangementRow::Track(tid));
            }
        }
        rows
    }

    fn push_group_rows(
        &self,
        gid: u64,
        order: &[u64],
        rows: &mut Vec<ArrangementRow>,
        walked: &mut HashSet<u64>,
    ) {
        if !walked.insert(gid) {
            return;
        }
        rows.push(ArrangementRow::Group(gid));
        for child in self.child_groups_in(gid, order) {
            self.push_group_rows(child, order, rows, walked);
        }
        for &tid in &self.track_order {
            if self
                .tracks
                .get(&tid)
                .is_some_and(|t| t.group_id == Some(gid))
            {
                rows.push(ArrangementRow::Track(tid));
            }
        }
    }

    pub fn child_groups(&self, parent: u64) -> Vec<u64> {
        self.child_groups_in(parent, &self.ordered_group_ids())
    }

    fn child_groups_in(&self, parent: u64, order: &[u64]) -> Vec<u64> {
        order
            .iter()
            .copied()
            .filter(|id| {
                self.groups
                    .get(id)
                    .is_some_and(|g| g.parent_id == Some(parent))
            })
            .collect()
    }

    pub fn track_in_group(&self, track_id: u64) -> bool {
        self.tracks
            .get(&track_id)
            .is_some_and(|t| t.group_id.is_some_and(|g| self.groups.contains_key(&g)))
    }

    pub fn group_descendants(&self, group_id: u64) -> HashSet<u64> {
        let mut found = HashSet::new();
        let mut frontier = vec![group_id];
        while let Some(gid) = frontier.pop() {
            for child in self.child_groups(gid) {
                if found.insert(child) {
                    frontier.push(child);
                }
            }
        }
        found
    }

    pub fn get_group_members(&self, group_id: u64) -> Vec<u64> {
        self.track_order
            .iter()
            .copied()
            .filter(|id| {
                self.tracks
                    .get(id)
                    .is_some_and(|t| t.group_id == Some(group_id))
            })
            .collect()
    }

    pub fn get_group_subtree_tracks(&self, group_id: u64) -> Vec<u64> {
        let mut groups = vec![group_id];
        let mut i = 0;
        while i < groups.len() {
            groups.extend(self.child_groups(groups[i]));
            i += 1;
        }
        let group_set: HashSet<u64> = groups.into_iter().collect();
        self.track_order
            .iter()
            .copied()
            .filter(|id| {
                self.tracks
                    .get(id)
                    .is_some_and(|t| t.group_id.is_some_and(|g| group_set.contains(&g)))
            })
            .collect()
    }
}

impl AppState {
    /// Rebuild the clip index from current track state
    pub fn rebuild_clip_index(&mut self) {
        self.clips_by_id.clear();
        for (&track_id, track) in &self.tracks {
            for clip in &track.audio_clips {
                if clip.id != 0 {
                    self.clips_by_id.insert(
                        clip.id,
                        ClipRef {
                            track_id,
                            is_midi: false,
                        },
                    );
                }
            }
            for clip in &track.midi_clips {
                if clip.id != 0 {
                    self.clips_by_id.insert(
                        clip.id,
                        ClipRef {
                            track_id,
                            is_midi: true,
                        },
                    );
                }
            }
        }
    }

    pub fn rebuild_plugin_indices(&mut self) {
        for track in self.tracks.values_mut() {
            track.rebuild_plugin_index();
        }
    }

    pub fn position_to_beats(&self, position: f64) -> f64 {
        let converter = TimeConverter::new(self.sample_rate, self.bpm);
        converter.samples_to_beats(position)
    }

    pub fn beats_to_samples(&self, beats: f64) -> f64 {
        let converter = TimeConverter::new(self.sample_rate, self.bpm);
        converter.beats_to_samples(beats)
    }

    pub fn validate_before_save(&self) -> Result<()> {
        use std::collections::HashSet;
        let mut tracks = HashSet::new();
        let mut clips = HashSet::new();
        let mut plugins = HashSet::new();
        let mut notes = HashSet::new();
        let mut group_ids = HashSet::new();

        for (&track_id, track) in &self.tracks {
            if track_id == 0 || track.id == 0 {
                return Err(anyhow!("Track with unassigned (0) ID"));
            }
            if track.id != track_id {
                return Err(anyhow!(
                    "Track key/id mismatch: {} != {}",
                    track_id,
                    track.id
                ));
            }
            if !tracks.insert(track_id) {
                return Err(anyhow!("Duplicate track ID: {}", track_id));
            }
            for clip in &track.midi_clips {
                if clip.id == 0 {
                    return Err(anyhow!("MIDI clip with unassigned (0) ID"));
                }
                if !clips.insert(clip.id) {
                    return Err(anyhow!("Duplicate clip ID: {}", clip.id));
                }
                if let Some(pid) = clip.pattern_id
                    && self.patterns.get(&pid).is_none()
                {
                    return Err(anyhow!(
                        "MIDI clip {} references missing pattern {}",
                        clip.id,
                        pid
                    ));
                }
            }
            for clip in &track.audio_clips {
                if clip.id == 0 {
                    return Err(anyhow!("Audio clip with unassigned (0) ID"));
                }
                if !clips.insert(clip.id) {
                    return Err(anyhow!("Duplicate clip ID: {}", clip.id));
                }
            }
            for p in &track.plugin_chain {
                if p.id == 0 {
                    return Err(anyhow!("Plugin with unassigned (0) ID"));
                }
                if !plugins.insert(p.id) {
                    return Err(anyhow!("Duplicate plugin ID: {}", p.id));
                }
            }
        }
        for (&gid, group) in &self.groups {
            if gid == 0 {
                return Err(anyhow!("Group with unassigned (0) ID"));
            }
            if !group_ids.insert(gid) {
                return Err(anyhow!("Duplicate group ID: {}", gid));
            }
            if group.id != gid {
                return Err(anyhow!("Group key/id mismatch: {} != {}", gid, group.id));
            }
            if !group.volume.is_finite() || group.volume < 0.0 {
                return Err(anyhow!("Group {} has an invalid volume {}", gid, group.volume));
            }
            if group.link_mode != GroupLinkMode::Vca {
                return Err(anyhow!(
                    "Group '{}' uses linked faders, which this build does not implement",
                    group.name
                ));
            }
        }
        for &gid in &self.group_order {
            if !self.groups.contains_key(&gid) {
                return Err(anyhow!("Group order references missing group {}", gid));
            }
        }
        for (&gid, group) in &self.groups {
            if let Some(pid) = group.parent_id {
                if pid == gid {
                    return Err(anyhow!("Group {} is its own parent", gid));
                }
                if !self.groups.contains_key(&pid) {
                    return Err(anyhow!("Group {} has missing parent {}", gid, pid));
                }
            }
        }
        for (&gid, _) in &self.groups {
            let mut seen = HashSet::new();
            seen.insert(gid);
            let mut cursor = self.groups[&gid].parent_id;
            while let Some(pid) = cursor {
                if !seen.insert(pid) {
                    return Err(anyhow!("Group {} takes part in a parent cycle", gid));
                }
                cursor = self.groups[&pid].parent_id;
            }
        }
        for (&pid, pat) in &self.patterns {
            if pid == 0 || pat.id == 0 {
                return Err(anyhow!("Pattern with unassigned (0) ID"));
            }
            if pat.id != pid {
                return Err(anyhow!("Pattern key/id mismatch: {} != {}", pid, pat.id));
            }
            for n in &pat.notes {
                if n.id == 0 {
                    return Err(anyhow!("Note with unassigned (0) ID in pattern {}", pid));
                }
                if !notes.insert(n.id) {
                    return Err(anyhow!("Duplicate note ID: {}", n.id));
                }
            }
        }
        Ok(())
    }

    pub fn load_project(&mut self, project: Project) {
        if let Err(e) = check_project_version(&project.version) {
            log::warn!("Project version issue: {e}");
        }
        {
            let mut file_max = 0u64;
            for t in &project.tracks {
                file_max = file_max.max(t.id);
                for c in &t.audio_clips {
                    file_max = file_max.max(c.id);
                }
                for c in &t.midi_clips {
                    file_max = file_max.max(c.id);
                    file_max = file_max.max(c.pattern_id.unwrap_or(0));
                }
                for p in &t.plugin_chain {
                    file_max = file_max.max(p.id);
                }
            }
            for pat in &project.patterns {
                file_max = file_max.max(pat.id);
                for n in &pat.notes {
                    file_max = file_max.max(n.id);
                }
            }
            for g in &project.groups {
                file_max = file_max.max(g.id);
            }
            crate::idgen::seed_from_max(file_max);
        }

        self.tracks.clear();
        self.track_order.clear();

        let mut track_remap: HashMap<u64, u64> = HashMap::new();
        let mut pattern_remap: HashMap<u64, u64> = HashMap::new();
        let mut group_remap: HashMap<u64, u64> = HashMap::new();

        let mut tracks: Vec<Track> = Vec::with_capacity(project.tracks.len());
        for mut track in project.tracks {
            let old_tid = track.id;
            let mut track_id = if track.id == 0 {
                self.fresh_id()
            } else {
                track.id
            };
            if tracks.iter().any(|t: &Track| t.id == track_id) {
                log::warn!(
                    "Duplicate track id {} in project file; renumbering",
                    track_id
                );
                track_id = self.fresh_id();
            }
            if old_tid != track_id {
                track_remap.insert(old_tid, track_id);
            }
            track.id = track_id;
            track.rebuild_plugin_index();
            tracks.push(track);
        }

        let mut patterns: HashMap<u64, MidiPattern> = HashMap::new();
        for mut pat in project.patterns {
            let old_pid = pat.id;
            if pat.id == 0 {
                pat.id = self.fresh_id();
            }
            if patterns.contains_key(&pat.id) {
                log::warn!(
                    "Duplicate pattern id {} in project file; renumbering",
                    pat.id
                );
                pat.id = self.fresh_id();
            }
            if old_pid != pat.id {
                pattern_remap.insert(old_pid, pat.id);
            }
            let pid = pat.id;
            patterns.insert(pid, pat);
        }

        let mut groups: HashMap<u64, TrackGroup> = HashMap::new();
        let mut group_order: Vec<u64> = Vec::with_capacity(project.groups.len());
        for mut group in project.groups {
            let old_gid = group.id;
            if group.id == 0 {
                group.id = self.fresh_id();
            }
            if groups.contains_key(&group.id) {
                log::warn!(
                    "Duplicate group id {} in project file; renumbering",
                    group.id
                );
                group.id = self.fresh_id();
            }
            if old_gid != group.id {
                group_remap.insert(old_gid, group.id);
            }
            let gid = group.id;
            group_order.push(gid);
            groups.insert(gid, group);
        }

        if !track_remap.is_empty() || !pattern_remap.is_empty() || !group_remap.is_empty() {
            for track in &mut tracks {
                for clip in &mut track.midi_clips {
                    if let Some(pid) = clip.pattern_id {
                        if let Some(new_pid) = pattern_remap.get(&pid) {
                            clip.pattern_id = Some(*new_pid);
                        }
                    }
                }
                if let Some(gid) = track.group_id {
                    if let Some(new_gid) = group_remap.get(&gid) {
                        track.group_id = Some(*new_gid);
                    }
                }
                for send in &mut track.sends {
                    if let Some(new_dest) = track_remap.get(&send.destination_track) {
                        send.destination_track = *new_dest;
                    }
                }
            }
            for group in groups.values_mut() {
                if let Some(new_parent) = group.parent_id.and_then(|old| group_remap.get(&old)) {
                    group.parent_id = Some(*new_parent);
                }
            }
        }

        for track in tracks {
            let track_id = track.id;
            self.track_order.push(track_id);
            self.tracks.insert(track_id, track);
        }
        self.patterns = patterns;
        self.groups = groups;
        self.group_order = group_order;
        sanitize_group_forest(&mut self.groups);
        self.append_unlisted_groups();
        self.markers = project.markers;
        let mut seen_marker_ids = std::collections::HashSet::new();
        for marker in &mut self.markers {
            if marker.id == 0 || !seen_marker_ids.insert(marker.id) {
                marker.id = crate::idgen::next();
                seen_marker_ids.insert(marker.id);
            }
        }
        self.markers
            .sort_by(|a, b| a.beat.total_cmp(&b.beat).then_with(|| a.id.cmp(&b.id)));

        for track in self.tracks.values_mut() {
            for clip in &mut track.audio_clips {
                sanitize_warp_points(clip);
            }
        }

        self.project_name = project.name.clone();

        self.bpm = if project.bpm.is_finite() && project.bpm > 0.0 {
            project.bpm
        } else {
            log::warn!("Invalid bpm {} in project file; using 120", project.bpm);
            120.0
        };
        self.time_signature = project.time_signature;
        self.sample_rate = if project.sample_rate.is_finite() && project.sample_rate > 0.0 {
            project.sample_rate
        } else {
            44100.0
        };
        self.master_volume = project.master_volume;
        self.loop_start = project.loop_start;
        self.loop_end = project.loop_end;
        self.loop_enabled = project.loop_enabled;
        self.rebuild_clip_index();
        self.rebuild_plugin_indices();
        crate::idgen::seed_from_max(self.max_id_in_project());
        self.ensure_ids();
    }

    pub fn to_project(&self) -> Project {
        // Convert HashMap back to Vec for serialization
        let tracks: Vec<Track> = self
            .track_order
            .iter()
            .filter_map(|&id| self.tracks.get(&id).cloned())
            .collect();

        Project {
            version: PROJECT_VERSION.to_string(),
            name: self.project_name.clone(),
            tracks,
            patterns: self.patterns.values().cloned().collect(),
            groups: self
                .ordered_group_ids()
                .into_iter()
                .filter_map(|id| self.groups.get(&id).cloned())
                .collect(),
            markers: self.markers.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
            created_at: self.created_at,
            modified_at: chrono::Utc::now(),
        }
    }

    #[inline]
    pub fn fresh_id(&self) -> u64 {
        crate::idgen::next()
    }

    pub fn ensure_ids(&mut self) {
        // Track IDs (stable)
        let track_ids: Vec<u64> = self.tracks.keys().copied().collect();
        for tid in &track_ids {
            if let Some(t) = self.tracks.get_mut(tid) {
                if t.id == 0 {
                    t.id = *tid;
                }
            }
        }

        // Stage new patterns to avoid double-borrows
        struct NewPattern {
            pid: u64,
            notes: Vec<crate::model::clip::MidiNote>,
        }
        let mut staged: Vec<NewPattern> = Vec::new();

        for tid in &track_ids {
            if let Some(track) = self.tracks.get_mut(tid) {
                for c in &mut track.midi_clips {
                    if c.id == 0 {
                        c.id = crate::idgen::next();
                    }
                    if c.pattern_id.is_none() {
                        let pid = crate::idgen::next();
                        let moved = std::mem::take(&mut c.notes);
                        staged.push(NewPattern { pid, notes: moved });
                        c.pattern_id = Some(pid);
                    }

                    if !c.content_len_beats.is_finite() || c.content_len_beats <= 0.0 {
                        c.content_len_beats = c.length_beats.max(0.000001);
                    }
                    if !c.content_offset_beats.is_finite() {
                        c.content_offset_beats = 0.0;
                    }
                    let len = c.content_len_beats.max(0.000001);
                    c.content_offset_beats = ((c.content_offset_beats % len) + len) % len;
                }

                for ac in &mut track.audio_clips {
                    if ac.id == 0 {
                        ac.id = crate::idgen::next();
                    }
                }
                for p in &mut track.plugin_chain {
                    if p.id == 0 {
                        p.id = crate::idgen::next();
                    }
                }
            }
        }

        for np in staged {
            self.patterns.entry(np.pid).or_insert(MidiPattern {
                id: np.pid,
                notes: np.notes,
            });
        }

        for pat in self.patterns.values_mut() {
            for n in &mut pat.notes {
                if n.id == 0 {
                    n.id = crate::idgen::next();
                }
                if !n.start.is_finite() || n.start < 0.0 {
                    n.start = 0.0;
                }
                if !n.duration.is_finite() || n.duration <= 0.0 {
                    n.duration = 1e-6;
                }
            }
        }

        for marker in &mut self.markers {
            if marker.id == 0 {
                marker.id = crate::idgen::next();
            }
            if !marker.beat.is_finite() || marker.beat < 0.0 {
                marker.beat = 0.0;
            }
        }

        self.rebuild_clip_index();
    }

    /// Helper: get ordered track list (for UI iteration)
    pub fn ordered_tracks(&self) -> Vec<&Track> {
        self.track_order
            .iter()
            .filter_map(|&id| self.tracks.get(&id))
            .collect()
    }

    /// Helper: Apply a mutable operation to each track in order.
    pub fn for_each_ordered_track_mut<F>(&mut self, mut op: F)
    where
        F: FnMut(&mut Track),
    {
        let order = self.track_order.clone();
        for id in order {
            if let Some(track) = self.tracks.get_mut(&id) {
                op(track);
            }
        }
    }

    /// Resolve pattern-backed notes into a standalone clip copy.
    pub fn resolve_midi_clip_notes(
        &self,
        clip: &crate::model::clip::MidiClip,
    ) -> Vec<crate::model::clip::MidiNote> {
        if let Some(pid) = clip.pattern_id {
            if let Some(p) = self.patterns.get(&pid) {
                return p.notes.clone();
            }
        }
        clip.notes.clone()
    }

    /// Resolve pattern notes into an independent clip copy with no shared pattern.
    pub fn materialize_midi_clip(
        &self,
        clip: &crate::model::clip::MidiClip,
    ) -> crate::model::clip::MidiClip {
        let mut c = clip.clone();
        c.notes = self.resolve_midi_clip_notes(clip);
        c.pattern_id = None; // independent copy; ensure_ids will mint a fresh pattern
        c
    }

    /// Find clip by ID
    pub fn find_clip(&self, clip_id: u64) -> Option<(&Track, ClipLocation)> {
        let clip_ref = self.clips_by_id.get(&clip_id)?;
        let track = self.tracks.get(&clip_ref.track_id)?;

        if clip_ref.is_midi {
            let idx = track.midi_clips.iter().position(|c| c.id == clip_id)?;
            Some((track, ClipLocation::Midi(idx)))
        } else {
            let idx = track.audio_clips.iter().position(|c| c.id == clip_id)?;
            Some((track, ClipLocation::Audio(idx)))
        }
    }

    /// Find clip mutably
    pub fn find_clip_mut(&mut self, clip_id: u64) -> Option<(&mut Track, ClipLocation)> {
        let clip_ref = self.clips_by_id.get(&clip_id)?;
        let track_id = clip_ref.track_id;
        let is_midi = clip_ref.is_midi;

        let track = self.tracks.get_mut(&track_id)?;

        if is_midi {
            let idx = track.midi_clips.iter().position(|c| c.id == clip_id)?;
            Some((track, ClipLocation::Midi(idx)))
        } else {
            let idx = track.audio_clips.iter().position(|c| c.id == clip_id)?;
            Some((track, ClipLocation::Audio(idx)))
        }
    }

    pub fn find_plugin(&self, track_id: u64, plugin_id: u64) -> Option<(&Track, usize)> {
        let track = self.tracks.get(&track_id)?;
        let idx = track.plugin_chain.iter().position(|p| p.id == plugin_id)?;
        Some((track, idx))
    }

    pub fn find_plugin_mut(
        &mut self,
        track_id: u64,
        plugin_id: u64,
    ) -> Option<(&mut Track, usize)> {
        let track = self.tracks.get_mut(&track_id)?;
        let idx = track.plugin_chain.iter().position(|p| p.id == plugin_id)?;
        Some((track, idx))
    }
    fn max_id_in_project(&self) -> u64 {
        let mut max_id = 0u64;
        for t in self.tracks.values() {
            max_id = max_id.max(t.id);
            for c in &t.audio_clips {
                max_id = max_id.max(c.id);
            }
            for c in &t.midi_clips {
                max_id = max_id.max(c.id);
                max_id = max_id.max(c.pattern_id.unwrap_or(0));
            }
            for p in &t.plugin_chain {
                max_id = max_id.max(p.id);
            }
        }
        for (pid, pat) in &self.patterns {
            max_id = max_id.max(*pid);
            max_id = max_id.max(pat.id);
            for n in &pat.notes {
                max_id = max_id.max(n.id);
            }
        }
        for (gid, _) in &self.groups {
            max_id = max_id.max(*gid);
        }
        for marker in &self.markers {
            max_id = max_id.max(marker.id);
        }
        max_id
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ClipLocation {
    Midi(usize),
    Audio(usize),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    #[serde(default = "default_project_version")]
    pub version: String,
    #[serde(default = "default_project_name")]
    pub name: String,
    #[serde(default)]
    pub tracks: Vec<Track>, // For serialization compatibility
    #[serde(default)]
    pub patterns: Vec<MidiPattern>,
    #[serde(default)]
    pub groups: Vec<TrackGroup>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default = "default_bpm")]
    pub bpm: f32,
    #[serde(default = "default_time_sig")]
    pub time_signature: (i32, i32),
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f32,
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    #[serde(default)]
    pub loop_start: f64,
    #[serde(default)]
    pub loop_end: f64,
    #[serde(default)]
    pub loop_enabled: bool,
    #[serde(default = "default_now")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_now")]
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

#[inline]
fn default_project_version() -> String {
    PROJECT_VERSION.to_string()
}

#[inline]
fn default_bpm() -> f32 {
    120.0
}

#[inline]
fn default_time_sig() -> (i32, i32) {
    (4, 4)
}

#[inline]
fn default_sample_rate() -> f32 {
    44100.0
}

#[inline]
fn default_master_volume() -> f32 {
    0.8
}

/// Validate the on-disk schema version: reject unknown majors (forward
/// incompatible), accept same/older and migrate via serde defaults.
fn check_project_version(version: &str) -> anyhow::Result<()> {
    let parse = |v: &str| -> Option<(u64, u64, u64)> {
        let mut it = v.split('.');
        Some((
            it.next()?.parse().ok()?,
            it.next()?.parse().ok()?,
            it.next()?.parse().ok()?,
        ))
    };
    let (cur_maj, _, _) = parse(PROJECT_VERSION).unwrap_or((1, 1, 0));
    match parse(version) {
        Some((maj, _, _)) if maj == cur_maj => Ok(()),
        Some((maj, _, _)) => Err(anyhow::anyhow!(
            "Project major version {maj} != supported {cur_maj} (got {version})"
        )),
        None => Err(anyhow::anyhow!("Unparseable project version: {version}")),
    }
}

impl From<&AppState> for Project {
    fn from(state: &AppState) -> Self {
        state.to_project()
    }
}
