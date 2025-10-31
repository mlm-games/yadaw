use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::constants::DEFAULT_LOOP_LEN;
use crate::model::Track;
use crate::time_utils::TimeConverter;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppState {
    /// ID-based storage (canonical)
    pub tracks: HashMap<u64, Track>,
    /// Display order (references track IDs)
    pub track_order: Vec<u64>,
    /// Global clip registry for fast lookup
    pub clips_by_id: HashMap<u64, ClipRef>,

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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub tracks: HashMap<u64, Track>,
    pub track_order: Vec<u64>,
    pub master_volume: f32,
    pub bpm: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub sample_rate: f32,
    pub time_signature: (i32, i32),
    pub playing: bool,
    pub recording: bool,
}

impl AppState {
    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            tracks: self.tracks.clone(),
            track_order: self.track_order.clone(),
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            playing: self.playing,
            recording: self.recording,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
        }
    }

    pub fn restore(&mut self, snapshot: AppStateSnapshot) {
        self.tracks = snapshot.tracks;
        self.track_order = snapshot.track_order;
        self.bpm = snapshot.bpm;
        self.time_signature = snapshot.time_signature;
        self.sample_rate = snapshot.sample_rate;
        self.playing = snapshot.playing;
        self.recording = snapshot.recording;
        self.master_volume = snapshot.master_volume;
        self.loop_start = snapshot.loop_start;
        self.loop_end = snapshot.loop_end;
        self.loop_enabled = snapshot.loop_enabled;
        self.rebuild_clip_index();
        self.ensure_ids();
    }

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
        let mut seen_ids = HashSet::new();

        for (&track_id, track) in &self.tracks {
            if !seen_ids.insert(track_id) {
                return Err(anyhow!("Duplicate track ID: {}", track_id));
            }
            for clip in &track.midi_clips {
                if clip.id != 0 && !seen_ids.insert(clip.id) {
                    return Err(anyhow!("Duplicate clip ID: {}", clip.id));
                }
            }
            for clip in &track.audio_clips {
                if clip.id != 0 && !seen_ids.insert(clip.id) {
                    return Err(anyhow!("Duplicate clip ID: {}", clip.id));
                }
            }
        }
        Ok(())
    }

    pub fn load_project(&mut self, project: Project) {
        // Convert Vec<Track> to HashMap with IDs
        self.tracks.clear();
        self.track_order.clear();

        for mut track in project.tracks {
            let track_id = if track.id == 0 {
                self.fresh_id()
            } else {
                track.id
            };
            track.id = track_id;
            self.track_order.push(track_id);
            self.tracks.insert(track_id, track);
        }

        self.bpm = project.bpm;
        self.time_signature = project.time_signature;
        self.sample_rate = project.sample_rate;
        self.master_volume = project.master_volume;
        self.loop_start = project.loop_start;
        self.loop_end = project.loop_end;
        self.loop_enabled = project.loop_enabled;
        self.rebuild_clip_index();
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
            version: "1.0.0".to_string(),
            name: "Untitled Project".to_string(),
            tracks,
            bpm: self.bpm,
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
            master_volume: self.master_volume,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_enabled: self.loop_enabled,
            created_at: chrono::Utc::now(),
            modified_at: chrono::Utc::now(),
        }
    }

    pub fn fresh_id(&mut self) -> u64 {
        if self.next_id == 0 {
            self.reseed_next_id_from_max();
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn ensure_ids(&mut self) {
        // Ensure all tracks have IDs
        let track_ids: Vec<u64> = self.tracks.keys().copied().collect();
        for track_id in &track_ids {
            if let Some(track) = self.tracks.get_mut(track_id) {
                if track.id == 0 {
                    track.id = *track_id;
                }
            }
        }

        // Normalize MIDI clips and assign IDs
        for track_id in &track_ids {
            // First, collect all info needed to generate IDs without holding a mutable borrow on the track.
            let mut ids_to_generate = (0, 0, 0, 0); // (midi_clips, audio_clips, notes, plugin_ids)
            if let Some(track) = self.tracks.get(track_id) {
                for c in &track.midi_clips {
                    if c.id == 0 {
                        ids_to_generate.0 += 1;
                    }
                    for n in &c.notes {
                        if n.id == 0 {
                            ids_to_generate.2 += 1;
                        }
                    }
                }
                for c in &track.audio_clips {
                    if c.id == 0 {
                        ids_to_generate.1 += 1;
                    }
                }
                for c in &track.plugin_chain {
                    if c.id == 0 {
                        ids_to_generate.3 += 1;
                    }
                }
            }

            // Now, generate all IDs at once.
            let mut new_midi_clip_ids: Vec<u64> =
                (0..ids_to_generate.0).map(|_| self.fresh_id()).collect();
            let mut new_audio_clip_ids: Vec<u64> =
                (0..ids_to_generate.1).map(|_| self.fresh_id()).collect();
            let mut new_note_ids: Vec<u64> =
                (0..ids_to_generate.2).map(|_| self.fresh_id()).collect();
            let mut new_plugin_ids: Vec<u64> =
                (0..ids_to_generate.3).map(|_| self.fresh_id()).collect();

            // Finally, apply the new IDs.
            if let Some(track) = self.tracks.get_mut(track_id) {
                for c in &mut track.midi_clips {
                    if c.id == 0 {
                        c.id = new_midi_clip_ids.pop().unwrap();
                    }

                    if !c.content_len_beats.is_finite() || c.content_len_beats <= 0.0 {
                        c.content_len_beats = c.length_beats.max(0.000001);
                    }
                    if !c.content_offset_beats.is_finite() {
                        c.content_offset_beats = 0.0;
                    }
                    let len = c.content_len_beats.max(0.000001);
                    c.content_offset_beats = ((c.content_offset_beats % len) + len) % len;

                    let clip_len = c.length_beats.max(0.0);
                    for n in &mut c.notes {
                        if n.id == 0 {
                            n.id = new_note_ids.pop().unwrap();
                        }
                        if !n.start.is_finite() {
                            n.start = 0.0;
                        }
                        if !n.duration.is_finite() {
                            n.duration = 0.0;
                        }
                        if n.start < 0.0 {
                            n.start = 0.0;
                        }
                        n.duration = n.duration.max(1e-6);
                        if n.start > clip_len {
                            n.start = clip_len;
                        }
                        if n.start + n.duration > clip_len {
                            n.duration = (clip_len - n.start).max(1e-6);
                        }
                    }
                }

                for c in &mut track.audio_clips {
                    if c.id == 0 {
                        c.id = new_audio_clip_ids.pop().unwrap();
                    }
                }
                for c in &mut track.plugin_chain {
                    if c.id == 0 {
                        c.id = new_plugin_ids.pop().unwrap();
                    }
                }
            }
        }

        self.rebuild_clip_index();
    }

    fn reseed_next_id_from_max(&mut self) {
        let mut max_id = 0u64;
        for &track_id in self.track_order.iter() {
            max_id = max_id.max(track_id);
            if let Some(t) = self.tracks.get(&track_id) {
                for c in &t.audio_clips {
                    max_id = max_id.max(c.id);
                }
                for c in &t.midi_clips {
                    max_id = max_id.max(c.id);
                    for n in &c.notes {
                        max_id = max_id.max(n.id);
                    }
                }
            }
        }
        self.next_id = max_id.saturating_add(1).max(1);
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
}

#[derive(Debug, Clone, Copy)]
pub enum ClipLocation {
    Midi(usize),
    Audio(usize),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub version: String,
    pub name: String,
    pub tracks: Vec<Track>, // For serialization compatibility
    pub bpm: f32,
    pub time_signature: (i32, i32),
    pub sample_rate: f32,
    pub master_volume: f32,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

impl From<&AppState> for Project {
    fn from(state: &AppState) -> Self {
        state.to_project()
    }
}
