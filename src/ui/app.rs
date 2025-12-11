use super::*;
use crate::audio_state::AudioState;
use crate::config::Config;
use crate::constants::{DEFAULT_GRID_SNAP, DEFAULT_MIN_PROJECT_BEATS};
use crate::edit_actions::EditProcessor;
use crate::error::{ResultExt, UserNotification, common};
use crate::input::InputManager;
use crate::input::actions::{ActionContext, AppAction};
use crate::messages::PluginParamInfo;
use crate::midi_input::MidiInputHandler;
use crate::model::automation::AutomationTarget;
use crate::model::plugin_api::UnifiedPluginInfo;
use crate::model::track::TrackType;
use crate::model::{AudioClip, MidiNote, Track};
use crate::paths::{current_theme_path, custom_themes_path, shortcuts_path};
use crate::performance::{PerformanceMetrics, PerformanceMonitor};
use crate::project::{AppState, AppStateSnapshot, ClipLocation};
use crate::project_manager::ProjectManager;

use crate::track_manager::{TrackManager, UITrackType};
use crate::transport::Transport;
use crossbeam_channel::{Receiver, Sender};

use eframe::egui;
use egui::ahash::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum ActiveEditTarget {
    Clips,
    Notes,
}

pub struct YadawApp {
    // Core state
    pub(super) state: Arc<Mutex<AppState>>,
    pub(super) audio_state: Arc<AudioState>,
    pub(super) command_tx: Sender<AudioCommand>,
    pub(super) ui_rx: Receiver<UIUpdate>,

    // Configuration
    pub(super) config: Config,
    pub(super) theme_manager: super::theme::ThemeManager,

    // UI Components
    pub(super) transport_ui: super::transport::TransportUI,
    pub(super) tracks_ui: super::tracks::TracksPanel,
    pub(super) timeline_ui: super::timeline::TimelineView,
    pub(super) mixer_ui: super::mixer::MixerWindow,
    pub(super) menu_bar: super::menu_bar::MenuBar,
    pub(super) piano_roll_view: super::piano_roll_view::PianoRollView,

    // Dialogs
    pub(super) dialogs: super::dialogs::DialogManager,

    // Plugin management
    pub(super) available_plugins: HashMap<String, UnifiedPluginInfo>,
    pub(super) selected_track_for_plugin: Option<u64>,
    pub(super) clap_param_meta: std::collections::HashMap<(u64, usize), Vec<PluginParamInfo>>,

    // Selection state
    pub(super) selected_track: u64,
    pub(super) selected_pattern: u64,
    pub(super) selected_clips: Vec<u64>,

    // Undo/Redo
    pub(super) undo_stack: VecDeque<AppStateSnapshot>,
    pub(super) redo_stack: VecDeque<AppStateSnapshot>,

    // Other state
    pub(super) project_path: Option<String>,
    pub(super) clipboard: Option<Vec<AudioClip>>,
    pub(super) midi_clipboard: Option<Vec<crate::model::clip::MidiClip>>,
    pub(super) show_performance: bool,
    pub(super) performance_monitor: PerformanceMonitor,
    pub(super) track_manager: TrackManager,
    pub(super) project_manager: ProjectManager,

    // Touch support
    touch_state: TouchState,

    pub input_manager: InputManager,

    pub(super) note_clipboard: Option<Vec<MidiNote>>,
    pub(super) active_edit_target: ActiveEditTarget,
    pub last_real_metrics_at: Option<Instant>,

    pub is_recording_ui: bool,

    last_autosave: Instant,
    autosave_interval: Duration,
    pub show_close_confirmation: bool,

    pub midi_input_handler: Option<Arc<MidiInputHandler>>,
    pub available_midi_ports: Vec<String>,

    pub last_active_clip_per_track: HashMap<u64, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileDialogPurpose {
    None,
    OpenProject,
    SaveProject,
    ImportAudio,
    ExportAudio,
    BrowsePluginPath,
    SaveTheme,
    LoadTheme,
    LoadLayout,
    SaveLayout,
}

struct TouchState {
    last_touch_pos: Option<egui::Pos2>,
    pinch_distance: Option<f32>,
    gesture_start_time: Option<Instant>,
    tap_times: Vec<Instant>,
}

impl YadawApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        audio_state: Arc<AudioState>,
        command_tx: Sender<AudioCommand>,
        ui_rx: Receiver<UIUpdate>,
        available_plugins: Vec<UnifiedPluginInfo>,
        config: Config,
        midi_input_handler: Option<Arc<MidiInputHandler>>,
    ) -> Self {
        let transport = Transport::new(audio_state.clone(), command_tx.clone());
        let theme = match config.ui.theme {
            crate::config::Theme::Dark => super::theme::Theme::Dark,
            crate::config::Theme::Light => super::theme::Theme::Light,
        };
        let mut theme_manager = super::theme::ThemeManager::new(theme);

        let initial_track_id = {
            let state_guard = state.lock().unwrap();
            state_guard.track_order.first().copied().unwrap_or(0)
        };
        let available_plugins_map = available_plugins
            .into_iter()
            .map(|p| (p.uri.clone(), p))
            .collect();

        let available_midi_ports = midi_input_handler
            .as_ref()
            .map(|h| h.list_ports())
            .unwrap_or_default();

        let mut input_manager = InputManager::new();

        let mut project_manager = ProjectManager::new();

        // Load custom themes
        let themes_path = custom_themes_path();
        let _ = theme_manager.load_custom_themes(&themes_path);

        let theme_path = current_theme_path();
        let _ = theme_manager.load_current_theme(&theme_path);

        // Load custom shortcuts if they exist

        let shortcuts_path = shortcuts_path();
        let _ = input_manager.load_shortcuts(&shortcuts_path);

        project_manager.set_auto_save(config.behavior.auto_save);

        Self {
            transport_ui: super::transport::TransportUI::new(transport),
            tracks_ui: super::tracks::TracksPanel::new(),
            timeline_ui: super::timeline::TimelineView::new(),
            mixer_ui: super::mixer::MixerWindow::new(),
            menu_bar: super::menu_bar::MenuBar::new(),
            piano_roll_view: super::piano_roll_view::PianoRollView::new(),
            dialogs: super::dialogs::DialogManager::new(),
            theme_manager,

            state,
            audio_state,
            command_tx,
            ui_rx,
            config: config.clone(),
            available_plugins: available_plugins_map,
            clap_param_meta: std::collections::HashMap::new(),

            selected_track: initial_track_id,
            selected_pattern: 0,
            selected_clips: Vec::new(),
            selected_track_for_plugin: None,

            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),

            project_path: None,
            clipboard: None,
            midi_clipboard: None,
            note_clipboard: None,

            active_edit_target: ActiveEditTarget::Clips,

            show_performance: false,
            performance_monitor: PerformanceMonitor::new(),
            track_manager: TrackManager::new(),
            project_manager,

            touch_state: TouchState {
                last_touch_pos: None,
                pinch_distance: None,
                gesture_start_time: None,
                tap_times: Vec::new(),
            },

            input_manager,
            last_real_metrics_at: None,
            is_recording_ui: false,

            last_autosave: Instant::now(),
            autosave_interval: Duration::from_secs(
                config.behavior.auto_save_interval_minutes as u64 * 60,
            ),
            show_close_confirmation: false,

            midi_input_handler,
            available_midi_ports,

            last_active_clip_per_track: HashMap::default(),
        }
    }

    // Core functionality methods
    pub fn push_undo(&mut self) {
        let state = self.state.lock().unwrap();
        self.undo_stack.push_back(state.snapshot());
        self.redo_stack.clear();

        if self.undo_stack.len() > 100 {
            self.undo_stack.pop_front();
        }

        self.project_manager.mark_dirty();
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop_back() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.redo_stack.push_back(current);
            state.restore(snapshot);
            drop(state);

            self.sync_views_after_model_change();
            let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop_back() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.undo_stack.push_back(current);
            state.restore(snapshot);
            drop(state);

            self.sync_views_after_model_change();
            let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        }
    }

    // Track management
    pub fn add_audio_track(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        let track_id = state.fresh_id();
        let mut track = self.track_manager.create_track(UITrackType::Audio, None);
        track.id = track_id;
        state.track_order.push(track_id);
        state.tracks.insert(track_id, track);
        state.ensure_ids();

        drop(state);
        self.select_track(track_id);

        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_midi_track(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        let track_id = state.fresh_id();
        let mut track = self.track_manager.create_track(UITrackType::Midi, None);
        track.id = track_id;
        state.track_order.push(track_id);
        state.tracks.insert(track_id, track);
        state.ensure_ids();

        drop(state);
        self.select_track(track_id);

        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_bus_track(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        let track_id = state.fresh_id();
        let mut track = self.track_manager.create_track(UITrackType::Bus, None);
        track.id = track_id;
        state.track_order.push(track_id);
        state.tracks.insert(track_id, track);
        state.ensure_ids();
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_automation_lane_by_id(&mut self, track_id: u64, target: AutomationTarget) {
        self.push_undo();
        let _ = self
            .command_tx
            .send(AudioCommand::AddAutomationPoint(track_id, target, 0.0, 0.5));
    }

    pub fn show_plugin_browser_for_track(&mut self, track_id: u64) {
        self.selected_track_for_plugin = Some(track_id);
        self.dialogs.show_plugin_browser();
    }

    // Update clipboard operations to use IDs
    pub fn cut_selected(&mut self) {
        self.copy_selected();
        self.delete_selected();
    }

    /// Copy selected clips (ID-based)
    pub fn copy_selected(&mut self) {
        let state = self.state.lock().unwrap();
        let mut audio = Vec::new();
        let mut midi = Vec::new();

        for &clip_id in &self.selected_clips {
            if let Some((track, loc)) = state.find_clip(clip_id) {
                match loc {
                    crate::project::ClipLocation::Midi(idx) => {
                        if let Some(clip) = track.midi_clips.get(idx) {
                            midi.push(clip.clone());
                        }
                    }
                    crate::project::ClipLocation::Audio(idx) => {
                        if let Some(clip) = track.audio_clips.get(idx) {
                            audio.push(clip.clone());
                        }
                    }
                }
            }
        }

        self.clipboard = if audio.is_empty() { None } else { Some(audio) };
        self.midi_clipboard = if midi.is_empty() { None } else { Some(midi) };
    }

    pub fn duplicate_selected_track(&mut self) {
        self.push_undo();

        let new_track_id = {
            let mut state = self.state.lock().unwrap();

            if let Some(track) = state.tracks.get(&self.selected_track).cloned() {
                // Resolve each MIDI clip's notes from pattern into clip.notes before duplicating
                let mut resolved = track.clone();
                for mc in &mut resolved.midi_clips {
                    if let Some(pid) = mc.pattern_id {
                        if let Some(p) = state.patterns.get(&pid) {
                            mc.notes = p.notes.clone();
                        }
                    }
                }

                let mut new_track = self.track_manager.duplicate_track(&resolved);
                let new_track_id = state.fresh_id();
                new_track.id = new_track_id;

                // Insert after current track in order
                if let Some(pos) = state
                    .track_order
                    .iter()
                    .position(|&id| id == self.selected_track)
                {
                    state.track_order.insert(pos + 1, new_track_id);
                } else {
                    state.track_order.push(new_track_id);
                }

                state.tracks.insert(new_track_id, new_track);
                state.ensure_ids();

                Some(new_track_id)
            } else {
                None
            }
        };

        if let Some(track_id) = new_track_id {
            self.select_track(track_id);
        }

        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        let _ = self.command_tx.send(AudioCommand::RebuildAllRtChains);
    }

    pub fn delete_selected_track(&mut self) {
        let state = self.state.lock().unwrap();
        if state.track_order.len() <= 1 {
            drop(state);
            self.dialogs.show_message("Cannot delete the last track");
            return;
        }
        drop(state);

        self.push_undo();

        let new_selected = {
            let mut state = self.state.lock().unwrap();

            if let Some(pos) = state
                .track_order
                .iter()
                .position(|&id| id == self.selected_track)
            {
                state.track_order.remove(pos);
                state.tracks.remove(&self.selected_track);

                // Remove clips from index
                state
                    .clips_by_id
                    .retain(|_, clip_ref| clip_ref.track_id != self.selected_track);

                let new_selected = if pos > 0 {
                    state.track_order.get(pos - 1).copied()
                } else {
                    state.track_order.first().copied()
                };

                new_selected
            } else {
                None
            }
        };

        if let Some(track_id) = new_selected {
            self.select_track(track_id);
        }
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn paste_at_playhead(&mut self) {
        self.push_undo();

        let current_beat = {
            let position = self.audio_state.get_position();
            let sample_rate = self.audio_state.sample_rate.load();
            let bpm = self.audio_state.bpm.load();
            (position / sample_rate as f64) * (bpm as f64 / 60.0)
        };

        let track_id = self.selected_track;
        let midi_clipboard = self.midi_clipboard.clone();
        let audio_clipboard = self.clipboard.clone();

        let mut prepared_midi_clips: Vec<crate::model::clip::MidiClip> = Vec::new();
        let mut prepared_audio_clips: Vec<crate::model::clip::AudioClip> = Vec::new();
        let mut required_ids: usize = 0;

        let is_midi_track = self
            .state
            .lock()
            .unwrap()
            .tracks
            .get(&track_id)
            .map_or(false, |t| matches!(t.track_type, TrackType::Midi));

        if is_midi_track {
            if let Some(clips_src) = midi_clipboard {
                required_ids += clips_src.len();
                for clip in &clips_src {
                    required_ids += clip.notes.len();
                }
                prepared_midi_clips = clips_src;
            }
        } else {
            if let Some(clips_src) = audio_clipboard {
                required_ids += clips_src.len();
                prepared_audio_clips = clips_src;
            }
        }

        if required_ids == 0 {
            return; // Nothing to paste
        }

        let new_ids: Vec<u64> = {
            let mut state = self.state.lock().unwrap();
            (0..required_ids).map(|_| state.fresh_id()).collect()
        };
        let mut id_iter = new_ids.into_iter();

        for clip in &mut prepared_midi_clips {
            clip.id = id_iter.next().unwrap();
            clip.start_beat = current_beat;
            for n in &mut clip.notes {
                n.id = id_iter.next().unwrap();
            }
        }
        for clip in &mut prepared_audio_clips {
            clip.id = id_iter.next().unwrap();
            clip.start_beat = current_beat;
        }

        {
            let mut state = self.state.lock().unwrap();
            if let Some(track) = state.tracks.get_mut(&track_id) {
                if is_midi_track {
                    track.midi_clips.extend(prepared_midi_clips.iter().cloned());
                    for c in &prepared_midi_clips {
                        state.clips_by_id.insert(
                            c.id,
                            crate::project::ClipRef {
                                track_id,
                                is_midi: true,
                            },
                        );
                    }
                } else {
                    track
                        .audio_clips
                        .extend(prepared_audio_clips.iter().cloned());
                    for c in &prepared_audio_clips {
                        state.clips_by_id.insert(
                            c.id,
                            crate::project::ClipRef {
                                track_id,
                                is_midi: false,
                            },
                        );
                    }
                }
            }
        }

        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    /// Delete selected clips (ID-based)
    pub fn delete_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();

        self.last_active_clip_per_track
            .retain(|_, v| !self.selected_clips.contains(v));

        if let Some(curr) = self.piano_roll_view.selected_clip {
            if self.selected_clips.contains(&curr) {
                self.piano_roll_view.selected_clip = None;
            }
        }

        let clip_ids = self.selected_clips.clone();
        let mut state = self.state.lock().unwrap();

        for clip_id in clip_ids {
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                match loc {
                    crate::project::ClipLocation::Midi(idx) => {
                        track.midi_clips.remove(idx);
                    }
                    crate::project::ClipLocation::Audio(idx) => {
                        track.audio_clips.remove(idx);
                    }
                }
                state.clips_by_id.remove(&clip_id);
            }
        }

        self.selected_clips.clear();
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    // Selection
    pub fn select_all(&mut self) {
        let state = self.state.lock().unwrap();
        self.selected_clips.clear();

        for track in state.tracks.values() {
            for clip in &track.audio_clips {
                if clip.id != 0 {
                    self.selected_clips.push(clip.id);
                }
            }
            for clip in &track.midi_clips {
                if clip.id != 0 {
                    self.selected_clips.push(clip.id);
                }
            }
        }
    }

    pub fn deselect_all(&mut self) {
        self.selected_clips.clear();
    }

    // Project management
    pub fn new_project(&mut self) {
        let mut state = self.state.lock().unwrap();
        *state = AppState::default();
        drop(state);

        self.project_path = None;
        self.select_track(0);
        self.selected_clips.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();

        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn save_project(&mut self) {
        let save_path = &self.project_path.clone();

        if let Some(path) = save_path {
            self.save_project_to_path(Path::new(path));
        } else {
            self.dialogs.show_save_dialog();
        }
    }

    pub fn save_project_to_path(&mut self, path: &Path) {
        let state = self.state.lock().unwrap();
        self.project_manager
            .save_project(&state, path)
            .map_err(common::project_save_failed)
            .map(|_| {
                self.project_path = Some(path.to_string_lossy().to_string());
                self.dialogs.show_success("Project saved successfully");
            })
            .notify_user(&mut self.dialogs);
    }

    pub fn load_project_from_path(&mut self, path: &Path) {
        self.project_manager
            .load_project(path)
            .map_err(common::project_load_failed)
            .map(|project| {
                let mut state = self.state.lock().unwrap();
                state.load_project(project);

                self.audio_state.bpm.store(state.bpm);
                self.audio_state.loop_start.store(state.loop_start);
                self.audio_state.loop_end.store(state.loop_end);
                self.audio_state
                    .loop_enabled
                    .store(state.loop_enabled, Ordering::Relaxed);

                self.transport_ui.bpm_input = format!("{:.1}", state.bpm);
                self.transport_ui.loop_start_input = format!("{:.1}", state.loop_start);
                self.transport_ui.loop_end_input = format!("{:.1}", state.loop_end);

                state.ensure_ids();
                drop(state);

                self.project_path = Some(path.to_string_lossy().to_string());
                self.select_track(0);
                self.selected_clips.clear();
                self.undo_stack.clear();
                self.redo_stack.clear();

                let _ = self.command_tx.send(AudioCommand::UpdateTracks);
                let _ = self.command_tx.send(AudioCommand::RebuildAllRtChains);
            })
            .notify_user(&mut self.dialogs);
    }

    // Audio operations
    pub fn normalize_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        for &clip_id in &self.selected_clips {
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        let peak = clip.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                        if peak > 0.0 {
                            let gain = crate::constants::NORMALIZE_TARGET_LINEAR / peak;
                            for s in &mut clip.samples {
                                *s *= gain;
                            }
                        }
                    }
                }
            }
        }
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn reverse_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        for &clip_id in &self.selected_clips {
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        clip.samples.reverse();
                    }
                }
            }
        }
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn apply_fade_in(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        let bpm = state.bpm;
        for &clip_id in &self.selected_clips {
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        EditProcessor::apply_fade_in(clip, 0.25, bpm);
                    }
                }
            }
        }
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn apply_fade_out(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        let bpm = state.bpm;
        for &clip_id in &self.selected_clips {
            if let Some((track, loc)) = state.find_clip_mut(clip_id) {
                if let crate::project::ClipLocation::Audio(idx) = loc {
                    if let Some(clip) = track.audio_clips.get_mut(idx) {
                        EditProcessor::apply_fade_out(clip, 0.25, bpm);
                    }
                }
            }
        }
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn split_selected_at_playhead(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        self.push_undo();

        let current_beat = {
            let position = self.audio_state.get_position();
            let sample_rate = self.audio_state.sample_rate.load();
            let bpm = self.audio_state.bpm.load();
            (position / sample_rate as f64) * (bpm as f64 / 60.0)
        };

        let selected_clips = self.selected_clips.clone();
        let mut state = self.state.lock().unwrap();
        let bpm = state.bpm;

        for clip_id in selected_clips {
            let (track_id, clip_to_split, clip_ref_opt, original_idx) = {
                if let Some((track, loc)) = state.find_clip(clip_id) {
                    if let crate::project::ClipLocation::Audio(idx) = loc {
                        let clip_ref = state.clips_by_id.get(&clip_id).cloned();
                        (
                            Some(track.id),
                            track.audio_clips.get(idx).cloned(),
                            clip_ref,
                            Some(idx),
                        )
                    } else {
                        (None, None, None, None)
                    }
                } else {
                    (None, None, None, None)
                }
            };

            if let (Some(track_id), Some(clip), Some(clip_ref), Some(idx)) =
                (track_id, clip_to_split, clip_ref_opt, original_idx)
            {
                if let Some((first_half, mut second_half)) =
                    EditProcessor::split_clip(&clip, current_beat, bpm)
                {
                    // Now we can generate an ID because we don't hold a mutable borrow
                    let new_id = state.fresh_id();
                    second_half.id = new_id;

                    // Re-acquire mutable borrow to perform the update
                    if let Some(track) = state.tracks.get_mut(&track_id) {
                        track.audio_clips[idx] = first_half;
                        track.audio_clips.insert(idx + 1, second_half.clone());

                        state.clips_by_id.insert(
                            new_id,
                            crate::project::ClipRef {
                                track_id: clip_ref.track_id,
                                is_midi: false,
                            },
                        );
                    }
                }
            }
        }

        drop(state);
        self.selected_clips.clear();
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn set_loop_to_selection(&mut self) {
        self.push_undo();

        if self.selected_clips.is_empty() {
            // Use visible timeline region
            let visible_start = self.timeline_ui.scroll_x / self.timeline_ui.zoom_x;
            let visible_end = visible_start + (800.0 / self.timeline_ui.zoom_x);

            self.audio_state.loop_start.store(visible_start as f64);
            self.audio_state.loop_end.store(visible_end as f64);
            self.audio_state.loop_enabled.store(true, Ordering::Relaxed);

            let _ = self.command_tx.send(AudioCommand::SetLoopRegion(
                visible_start as f64,
                visible_end as f64,
            ));
            let _ = self.command_tx.send(AudioCommand::SetLoopEnabled(true));
        } else {
            // Use selected clips range
            let state = self.state.lock().unwrap();
            let mut min_beat: f64 = f64::MAX;
            let mut max_beat: f64 = 0.0;

            for &clip_id in &self.selected_clips {
                if let Some((track, loc)) = state.find_clip(clip_id) {
                    match loc {
                        crate::project::ClipLocation::Audio(idx) => {
                            if let Some(clip) = track.audio_clips.get(idx) {
                                min_beat = min_beat.min(clip.start_beat);
                                max_beat = max_beat.max(clip.start_beat + clip.length_beats);
                            }
                        }
                        crate::project::ClipLocation::Midi(idx) => {
                            if let Some(clip) = track.midi_clips.get(idx) {
                                min_beat = min_beat.min(clip.start_beat);
                                max_beat = max_beat.max(clip.start_beat + clip.length_beats);
                            }
                        }
                    }
                }
            }

            if min_beat < f64::MAX && max_beat > 0.0 {
                self.audio_state.loop_start.store(min_beat);
                self.audio_state.loop_end.store(max_beat);
                self.audio_state.loop_enabled.store(true, Ordering::Relaxed);

                let _ = self
                    .command_tx
                    .send(AudioCommand::SetLoopRegion(min_beat, max_beat));
                let _ = self.command_tx.send(AudioCommand::SetLoopEnabled(true));
            }
        }
    }

    // MIDI operations
    pub fn quantize_selected_notes(&mut self, strength: f32) {
        let grid = self.piano_roll_view.piano_roll.grid_snap;
        self.quantize_selected_notes_with_params(strength, grid, 0.0);
    }

    pub fn transpose_selected_notes(&mut self, semitones: i32) {
        self.push_undo();
        let Some(clip_id) = self.piano_roll_view.selected_clip else {
            return;
        };
        let note_ids = self.piano_roll_view.piano_roll.selected_note_ids.clone();
        if note_ids.is_empty() {
            return;
        }

        let _ = self.command_tx.send(AudioCommand::TransposeSelectedNotes {
            clip_id,
            note_ids,
            semitones,
        });
    }

    fn nudge_notes(&mut self, direction: f32, fine: bool, coarse: bool) {
        self.push_undo();
        let Some(clip_id) = self.piano_roll_view.selected_clip else {
            return;
        };
        let note_ids = self.piano_roll_view.piano_roll.selected_note_ids.clone();
        if note_ids.is_empty() {
            return;
        }

        let grid = self.piano_roll_view.piano_roll.grid_snap as f64;
        let delta_beats = if fine {
            (grid / 4.0).max(1e-6) * direction as f64
        } else if coarse {
            1.0 * direction as f64
        } else {
            grid * direction as f64
        };

        let _ = self.command_tx.send(AudioCommand::NudgeSelectedNotes {
            clip_id,
            note_ids,
            delta_beats,
        });
    }

    pub fn quantize_selected_notes_with_params(&mut self, strength: f32, grid: f32, _swing: f32) {
        self.push_undo();
        let Some(clip_id) = self.piano_roll_view.selected_clip else {
            return;
        };
        let note_ids = self.piano_roll_view.piano_roll.selected_note_ids.clone();
        if note_ids.is_empty() {
            return;
        }

        let _ = self.command_tx.send(AudioCommand::QuantizeSelectedNotes {
            clip_id,
            note_ids,
            strength,
            grid,
        });
    }

    pub fn humanize_selected_notes(&mut self, amount: f32) {
        self.push_undo();
        let Some(clip_id) = self.piano_roll_view.selected_clip else {
            return;
        };
        let note_ids = self.piano_roll_view.piano_roll.selected_note_ids.clone();
        if note_ids.is_empty() {
            return;
        }

        let _ = self.command_tx.send(AudioCommand::HumanizeSelectedNotes {
            clip_id,
            note_ids,
            amount,
        });
    }

    pub fn add_automation_lane(&mut self, track_id: u64, target: AutomationTarget) {
        self.push_undo();
        let _ = self
            .command_tx
            .send(AudioCommand::AddAutomationPoint(track_id, target, 0.0, 0.5));
    }

    pub fn zoom_to_fit(&mut self) {
        // Calculate the extent of all content
        let state = self.state.lock().unwrap();
        let mut max_beat: f64 = DEFAULT_MIN_PROJECT_BEATS; // Minimum 4 beats

        for track in state.tracks.values() {
            for clip in &track.audio_clips {
                max_beat = max_beat.max(clip.start_beat + clip.length_beats);
            }
        }

        // Calculate zoom level to fit content
        let available_width: f32 = 800.0;
        self.timeline_ui.zoom_x = (available_width / max_beat as f32).min(200.0).max(10.0);
        self.timeline_ui.scroll_x = 0.0;
    }

    pub fn reset_layout(&mut self) {
        // Reset all UI components to default positions
        self.timeline_ui = super::timeline::TimelineView::new();
        self.mixer_ui = super::mixer::MixerWindow::new();
        self.tracks_ui = super::tracks::TracksPanel::new();
    }

    pub fn tap_tempo(&mut self) {
        self.push_undo();
        let now = Instant::now();
        let taps = &mut self.touch_state.tap_times;

        // Keep only taps within the last 2 seconds
        taps.retain(|t| now.duration_since(*t).as_secs_f64() < 2.0);
        taps.push(now);

        if taps.len() >= 2 {
            let total: f64 = taps.windows(2).map(|w| (w[1] - w[0]).as_secs_f64()).sum();
            let avg = total / (taps.len() - 1) as f64;
            let bpm = (60.0 / avg) as f32;
            if (20.0..=999.0).contains(&bpm) {
                if let Some(transport) = &mut self.transport_ui.transport {
                    transport.set_bpm(bpm);
                }
                self.audio_state.bpm.store(bpm);
            }
        }

        // cap stored taps so the Vec doesn't grow unbounded over time
        const MAX_TAPS: usize = 8;
        if taps.len() > MAX_TAPS {
            let drain = taps.len() - MAX_TAPS;
            taps.drain(0..drain);
        }
    }

    pub fn import_audio_dialog(&mut self) {
        self.dialogs.open_import_audio();
    }

    pub fn export_audio_dialog(&mut self) {
        self.dialogs.show_export_dialog();
    }

    pub fn is_selected_track_midi(&self) -> bool {
        let state = self.state.lock().unwrap();
        state
            .tracks
            .get(&self.selected_track)
            .map(|t| matches!(t.track_type, TrackType::Midi))
            .unwrap_or(false)
    }

    fn show_main_panels(&mut self, ctx: &egui::Context) {
        let show_midi = self.is_selected_track_midi();

        // Left panel - Tracks
        egui::SidePanel::left("tracks_panel")
            .default_width(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                let mut tracks_ui = std::mem::take(&mut self.tracks_ui);
                tracks_ui.show(ui, self);
                self.tracks_ui = tracks_ui;
            });

        // Central panel - Timeline or Piano Roll
        egui::CentralPanel::default().show(ctx, |ui| {
            if show_midi {
                // ui.heading("Piano Roll View");
                let mut piano_roll = std::mem::take(&mut self.piano_roll_view);
                piano_roll.show(ui, self);
                self.piano_roll_view = piano_roll;
            } else {
                let mut timeline = std::mem::take(&mut self.timeline_ui);
                timeline.show(ui, self);
                self.timeline_ui = timeline;
            }
        });
    }

    fn show_floating_windows(&mut self, ctx: &egui::Context) {
        // Mixer window
        if self.mixer_ui.is_visible() {
            let mut mixer = std::mem::take(&mut self.mixer_ui);
            mixer.show(ctx, self);
            self.mixer_ui = mixer;
        }

        // Dialogs
        let mut dialogs = std::mem::take(&mut self.dialogs);
        dialogs.show_all(ctx, self);
        self.dialogs = dialogs;

        // Performance monitor
        if self.show_performance {
            self.show_performance_window(ctx);
        }
    }

    fn process_ui_update(&mut self, update: UIUpdate, ctx: &egui::Context) {
        match update {
            UIUpdate::Position(pos) => {
                if let Ok(state) = self.state.lock() {
                    let current_beat = state.position_to_beats(pos);
                    ctx.memory_mut(|mem| {
                        mem.data
                            .insert_temp(egui::Id::new("current_beat"), current_beat);
                    });
                }
            }
            UIUpdate::TrackLevels(levels) => {
                self.tracks_ui.update_levels(levels);
            }
            UIUpdate::RecordingFinished(track_id, mut clip) => {
                self.push_undo();
                let mut state = self.state.lock().unwrap();
                clip.id = state.fresh_id();
                let clip_id = clip.id;

                let added = if let Some(track) = state.tracks.get_mut(&track_id) {
                    if !matches!(track.track_type, crate::model::track::TrackType::Midi) {
                        track.audio_clips.push(clip);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                if added {
                    state.clips_by_id.insert(
                        clip_id,
                        crate::project::ClipRef {
                            track_id,
                            is_midi: false,
                        },
                    );
                }

                drop(state);
                let _ = self
                    .command_tx
                    .send(crate::messages::AudioCommand::UpdateTracks);
                self.project_manager.mark_dirty();
            }
            UIUpdate::RecordingStateChanged(on) => {
                self.is_recording_ui = on;
            }
            UIUpdate::RecordingLevel(_) => {}
            UIUpdate::MasterLevel(_, _) => {}
            UIUpdate::PushUndo(snapshot) => {
                self.undo_stack.push_back(snapshot);
                self.redo_stack.clear();
                if self.undo_stack.len() > 100 {
                    self.undo_stack.remove(0);
                }
            }
            UIUpdate::PerformanceMetric {
                cpu_usage,
                buffer_fill,
                xruns,
                plugin_time_ms,
                latency_ms,
            } => {
                let metrics = crate::performance::PerformanceMetrics {
                    cpu_usage,
                    memory_usage: 0,
                    disk_streaming_rate: 0.0,
                    audio_buffer_health: buffer_fill,
                    plugin_processing_time: std::time::Duration::from_secs_f32(
                        plugin_time_ms / 1000.0,
                    ),
                    xruns: xruns as usize,
                    latency_ms,
                };
                self.performance_monitor.update_metrics(metrics);
                self.last_real_metrics_at = Some(std::time::Instant::now());
            }
            UIUpdate::NotesCutToClipboard(notes) => {
                self.note_clipboard = Some(notes);
            }
            UIUpdate::ExportStateUpdate(state) => {
                if let Some(dialog) = &mut self.dialogs.export_dialog {
                    dialog.set_state(state);
                }
            }
            UIUpdate::PluginParamsDiscovered {
                track_id,
                plugin_idx,
                params,
            } => {
                // Store meta info (name, min, max, default) for UI sliders
                let meta: Vec<PluginParamInfo> = params.iter().map(|p| p.clone()).collect();
                self.clap_param_meta.insert((track_id, plugin_idx), meta);

                {
                    let mut state = self.state.lock().unwrap();
                    if let Some(track) = state.tracks.get_mut(&track_id) {
                        if let Some(plugin) = track.plugin_chain.get_mut(plugin_idx) {
                            for (param_info) in &params {
                                plugin
                                    .params
                                    .insert(param_info.name.clone(), param_info.current);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn open_midi_clip_in_piano_roll(&mut self, clip_id: u64) {
        let track_id = {
            let state = self.state.lock().unwrap();
            state.clips_by_id.get(&clip_id).map(|r| r.track_id)
        };

        if let Some(tid) = track_id {
            self.select_track(tid);
            self.last_active_clip_per_track.insert(tid, clip_id);
        }

        self.piano_roll_view.set_editing_clip(clip_id);
    }

    fn show_performance_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Performance Monitor (TODO/WIP)")
            .open(&mut self.show_performance)
            .show(ctx, |ui| {
                if let Some(metrics) = self.performance_monitor.get_current_metrics() {
                    ui.label(format!("CPU Usage: {:.1}%", metrics.cpu_usage * 100.0));
                    ui.label(format!(
                        "Memory: {} MB",
                        metrics.memory_usage / (1024 * 1024)
                    ));
                    ui.label(format!("Latency: {:.1} ms", metrics.latency_ms));
                    ui.label(format!("XRuns: {}", metrics.xruns));

                    ui.separator();
                    ui.label("Optimization Hints:");

                    for hint in self.performance_monitor.get_optimization_hints() {
                        ui.horizontal(|ui| {
                            let color = match hint.severity {
                                crate::performance::Severity::Info => egui::Color32::LIGHT_BLUE,
                                crate::performance::Severity::Warning => egui::Color32::YELLOW,
                                crate::performance::Severity::Critical => egui::Color32::RED,
                            };
                            ui.colored_label(color, &hint.message);
                        });
                        ui.label(&hint.suggestion);
                    }
                }
            });
    }

    pub fn handle_action(&mut self, action: AppAction) {
        use AppAction::*;

        match action {
            PlayPause => {
                self.transport_ui.toggle_playback(&self.command_tx);
            }

            Stop => {
                if let Some(transport) = &self.transport_ui.transport {
                    transport.stop();
                }
            }

            Record => {
                if self.audio_state.recording.load(Ordering::Relaxed) {
                    let _ = self.command_tx.send(AudioCommand::StopRecording);
                } else {
                    let _ = self.command_tx.send(AudioCommand::StartRecording);
                }
            }

            GoToStart => {
                if let Some(transport) = &self.transport_ui.transport {
                    transport.set_position(0.0);
                }
            }

            Rewind => {
                if let Some(transport) = &self.transport_ui.transport {
                    transport.rewind_beats(4.0);
                }
            }

            FastForward => {
                if let Some(transport) = &self.transport_ui.transport {
                    transport.fast_forward(4.0);
                }
            }

            Undo => self.undo(),
            Redo => self.redo(),

            Copy => {
                if self.is_selected_track_midi() {
                    // Piano roll copy
                    let clipboard = self
                        .piano_roll_view
                        .copy_selected_notes(&self.state, self.selected_track);
                    if let Some(notes) = clipboard {
                        self.note_clipboard = Some(notes);
                    }
                } else {
                    // Timeline copy
                    self.copy_selected();
                }
            }

            Cut => {
                if self.is_selected_track_midi() {
                    self.push_undo();
                    self.piano_roll_view.cut_selected_notes(&self.command_tx);
                } else {
                    self.cut_selected();
                }
            }
            Paste => {
                if self.is_selected_track_midi() {
                    if let Some(ref clipboard) = self.note_clipboard.clone() {
                        self.push_undo();
                        self.piano_roll_view.paste_notes(
                            &self.audio_state,
                            &self.command_tx,
                            clipboard,
                        );
                    }
                } else {
                    self.paste_at_playhead();
                }
            }
            Delete => {
                if self.is_selected_track_midi() {
                    self.push_undo();
                    self.piano_roll_view.delete_selected_notes(&self.command_tx);
                } else {
                    self.delete_selected();
                }
            }

            SelectAll => {
                if self.is_selected_track_midi() {
                    self.piano_roll_view
                        .select_all_notes(&self.state, self.selected_track);
                } else {
                    self.select_all();
                }
            }

            DeselectAll => {
                if self.is_selected_track_midi() {
                    self.piano_roll_view.piano_roll.selected_note_ids.clear();
                    self.piano_roll_view
                        .piano_roll
                        .temp_selected_indices
                        .clear();
                } else {
                    self.deselect_all();
                }
            }

            Duplicate => {
                if self.is_selected_track_midi() {
                    // Duplicate notes (handled via copy+paste)
                    if let Some(clipboard) = self
                        .piano_roll_view
                        .copy_selected_notes(&self.state, self.selected_track)
                    {
                        self.piano_roll_view.paste_notes(
                            &self.audio_state,
                            &self.command_tx,
                            &clipboard,
                        );
                        self.push_undo();
                    }
                } else {
                    // Duplicate track
                    self.duplicate_selected_track();
                }
            }

            NewProject => self.new_project(),
            OpenProject => self.dialogs.show_open_dialog(),
            SaveProject => self.save_project(),
            SaveProjectAs => self.dialogs.show_save_dialog(),
            ImportAudio => self.import_audio_dialog(),
            ExportAudio => self.export_audio_dialog(),

            ZoomIn => {
                if self.is_selected_track_midi() {
                    self.piano_roll_view.piano_roll.zoom_x =
                        (self.piano_roll_view.piano_roll.zoom_x * 1.25).min(500.0);
                } else {
                    self.timeline_ui.zoom_x = (self.timeline_ui.zoom_x * 1.25).min(500.0);
                }
            }

            ZoomOut => {
                if self.is_selected_track_midi() {
                    self.piano_roll_view.piano_roll.zoom_x =
                        (self.piano_roll_view.piano_roll.zoom_x * 0.8).max(10.0);
                } else {
                    self.timeline_ui.zoom_x = (self.timeline_ui.zoom_x * 0.8).max(10.0);
                }
            }

            ZoomToFit => self.zoom_to_fit(),
            ToggleMixer => self.mixer_ui.toggle_visibility(),
            TogglePianoRoll => self.switch_to_piano_roll(),
            ToggleTimeline => self.switch_to_timeline(),

            ToggleLoop => {
                self.push_undo();

                let enabled = !self.audio_state.loop_enabled.load(Ordering::Relaxed);
                self.audio_state
                    .loop_enabled
                    .store(enabled, Ordering::Relaxed);
                let _ = self.command_tx.send(AudioCommand::SetLoopEnabled(enabled));
            }

            SetLoopToSelection => self.set_loop_to_selection(),

            ClearLoop => {
                self.push_undo();

                self.audio_state
                    .loop_enabled
                    .store(false, Ordering::Relaxed);
                let _ = self.command_tx.send(AudioCommand::SetLoopEnabled(false));
            }

            NudgeLeft => self.nudge_notes(-1.0, false, false),
            NudgeRight => self.nudge_notes(1.0, false, false),
            NudgeLeftFine => self.nudge_notes(-1.0, true, false),
            NudgeRightFine => self.nudge_notes(1.0, true, false),
            NudgeLeftCoarse => self.nudge_notes(-1.0, false, true),
            NudgeRightCoarse => self.nudge_notes(1.0, false, true),

            TransposeUp => self.transpose_selected_notes(1),
            TransposeDown => self.transpose_selected_notes(-1),
            TransposeOctaveUp => self.transpose_selected_notes(12),
            TransposeOctaveDown => self.transpose_selected_notes(-12),

            VelocityUp => self.adjust_velocity(10),
            VelocityDown => self.adjust_velocity(-10),

            SplitAtPlayhead => self.split_selected_at_playhead(),
            Normalize => self.normalize_selected(),
            Reverse => self.reverse_selected(),
            FadeIn => self.apply_fade_in(),
            FadeOut => self.apply_fade_out(),

            QuantizeDialog => self.dialogs.show_quantize_dialog(),
            TransposeDialog => self.dialogs.show_transpose_dialog(),
            HumanizeDialog => self.dialogs.show_humanize_dialog(),

            Escape => {
                // Close dialogs or deselect
                self.deselect_all();
                self.timeline_ui.show_clip_menu = false;
            }
        }
    }

    fn adjust_velocity(&mut self, delta: i8) {
        self.push_undo();

        let clip_id = match self.piano_roll_view.selected_clip {
            Some(id) => id,
            None => return,
        };

        let selected_ids = self.piano_roll_view.piano_roll.selected_note_ids.clone();
        if selected_ids.is_empty() {
            return;
        }

        let pattern_notes: Vec<MidiNote> = {
            let state = self.state.lock().unwrap();
            match state.find_clip(clip_id) {
                Some((track, ClipLocation::Midi(idx))) => {
                    let clip = &track.midi_clips[idx];
                    if let Some(pid) = clip.pattern_id {
                        state
                            .patterns
                            .get(&pid)
                            .map(|p| p.notes.clone())
                            .unwrap_or_else(|| clip.notes.clone())
                    } else {
                        clip.notes.clone()
                    }
                }
                _ => Vec::new(),
            }
        };

        if pattern_notes.is_empty() {
            return;
        }

        let mut updated: Vec<MidiNote> = Vec::new();
        for n in &pattern_notes {
            if selected_ids.contains(&n.id) {
                let mut nn = *n;
                let new_vel = (nn.velocity as i16 + delta as i16).clamp(1, 127) as u8;
                nn.velocity = new_vel;
                updated.push(nn);
            }
        }

        if updated.is_empty() {
            return;
        }

        let _ = self
            .command_tx
            .send(crate::messages::AudioCommand::UpdateNotesById {
                clip_id,
                notes: updated,
            });
    }

    pub fn switch_to_piano_roll(&mut self) {
        let midi_id = {
            let state = self.state.lock().unwrap();
            state
                .tracks
                .iter()
                .find(|(_id, t)| matches!(t.track_type, TrackType::Midi))
                .map(|(id, _track)| *id)
        };
        if let Some(id) = midi_id {
            self.select_track(id);
        } else {
            self.dialogs
                .show_message("No MIDI track found. Add a MIDI track first.");
        }
    }

    pub fn switch_to_timeline(&mut self) {
        let audio_id = {
            let state = self.state.lock().unwrap();
            state
                .track_order
                .iter()
                .find(|&&id| {
                    state
                        .tracks
                        .get(&id)
                        .map_or(false, |t| !matches!(t.track_type, TrackType::Midi))
                })
                .copied()
        };
        if let Some(id) = audio_id {
            self.select_track(id);
        } else {
            self.dialogs.show_message("No audio track found.");
        }
    }

    pub fn sync_views_after_model_change(&mut self) {
        let state = self.state.lock().unwrap();
        let tracks_len = state.tracks.len();
        if tracks_len == 0 {
            self.selected_track = 0;
            self.piano_roll_view.selected_clip = None;
            return;
        }

        // Refresh piano roll notes if a clip is selected
        if let Some(clip_id) = self.piano_roll_view.selected_clip {
            let _notes_opt = state
                .tracks
                .get(&self.selected_track)
                .and_then(|t| t.midi_clips.iter().find(|c| c.id == clip_id))
                .map(|c| c.notes.clone());
        }
    }

    pub fn select_track(&mut self, track_id: u64) {
        self.selected_track = track_id;

        let is_midi = {
            let state = self.state.lock().unwrap();
            state
                .tracks
                .get(&track_id)
                .map_or(false, |t| matches!(t.track_type, TrackType::Midi))
        };

        if is_midi {
            if let Some(&last_clip_id) = self.last_active_clip_per_track.get(&track_id) {
                let state = self.state.lock().unwrap();
                if state.clips_by_id.contains_key(&last_clip_id) {
                    self.piano_roll_view.set_editing_clip(last_clip_id);
                    return;
                }
            }

            let current_pos = self.audio_state.get_position();
            let current_beat = {
                let state = self.state.lock().unwrap();
                state.position_to_beats(current_pos)
            };

            let state = self.state.lock().unwrap();
            let track = state.tracks.get(&track_id);

            let clip_under_playhead = track.and_then(|t| {
                t.midi_clips.iter().find(|c| {
                    current_beat >= c.start_beat && current_beat < (c.start_beat + c.length_beats)
                })
            });

            if let Some(clip) = clip_under_playhead {
                let id = clip.id;
                drop(state);
                self.piano_roll_view.set_editing_clip(id);
                self.last_active_clip_per_track.insert(track_id, id);
                return;
            }

            let first_clip = track.and_then(|t| t.midi_clips.first().map(|c| c.id));
            if let Some(id) = first_clip {
                drop(state);
                self.piano_roll_view.set_editing_clip(id);
                self.last_active_clip_per_track.insert(track_id, id);
            } else {
                drop(state);
                self.piano_roll_view.selected_clip = None;
            }
        }
    }

    pub fn get_selected_track(&self) -> Option<Track> {
        let state = self.state.lock().unwrap();
        state.tracks.get(&self.selected_track).cloned()
    }

    pub fn selected_track_display_index(&self) -> Option<usize> {
        let state = self.state.lock().unwrap();
        state
            .track_order
            .iter()
            .position(|&id| id == self.selected_track)
    }

    pub fn track_index_to_id(&self, idx: usize) -> Option<u64> {
        let state = self.state.lock().unwrap();
        state.track_order.get(idx).copied()
    }

    pub fn track_id_to_index(&self, track_id: u64) -> Option<usize> {
        let state = self.state.lock().unwrap();
        state.track_order.iter().position(|&id| id == track_id)
    }

    /// Get track by ID (old)
    pub fn get_track(&self, track_id: u64) -> Option<Track> {
        let state = self.state.lock().unwrap();
        state.tracks.get(&track_id).cloned()
    }

    pub fn invalidate_clap_params_for_track(&mut self, track_id: u64) {
        self.clap_param_meta.retain(|(tid, _), _| *tid != track_id);
    }
}

impl eframe::App for YadawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.project_manager.is_dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.show_close_confirmation = true;
            }
        }

        if self.show_close_confirmation {
            egui::Modal::new(egui::Id::new("close_confirm_modal")).show(ctx, |ui| {
                ui.set_width(300.0);
                ui.heading("Unsaved Changes");
                ui.label("You have unsaved changes. Do you want to save before closing?");

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save_project();
                        if !self.project_manager.is_dirty() {
                            self.show_close_confirmation = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }

                    if ui.button("Don't Save").clicked() {
                        self.show_close_confirmation = false;
                        self.project_manager.mark_clean();

                        std::process::exit(0);
                    }

                    if ui.button("Cancel").clicked() {
                        self.show_close_confirmation = false;
                    }
                });
            });
        }

        self.theme_manager.apply_theme(ctx);

        // Process UI updates from audio thread
        while let Ok(update) = self.ui_rx.try_recv() {
            self.process_ui_update(update, ctx);
        }

        if self.is_selected_track_midi() {
            self.input_manager.set_context(ActionContext::PianoRoll);
        } else {
            self.input_manager.set_context(ActionContext::Timeline);
        }

        // Poll actions
        let actions = self.input_manager.poll_actions(ctx);

        // Dispatch actions
        for action in actions {
            self.handle_action(action);
        }

        // Draw menu bar
        let mut menu_bar = std::mem::take(&mut self.menu_bar);
        menu_bar.show(ctx, self);
        self.menu_bar = menu_bar;

        // Draw transport
        let mut transport_ui = std::mem::take(&mut self.transport_ui);
        transport_ui.show(ctx, self);
        self.transport_ui = transport_ui;

        // Draw main panels
        self.show_main_panels(ctx);

        // Draw floating windows
        self.show_floating_windows(ctx);

        // Request repaint if playing
        if self.audio_state.playing.load(Ordering::Relaxed) {
            ctx.request_repaint();
        }

        if self.project_manager.get_current_project().is_some()
            && self.last_autosave.elapsed() > self.autosave_interval
        {
            let state_guard = self.state.lock().unwrap();
            if let Err(e) = self.project_manager.auto_save(&state_guard) {
                log::error!("Auto-save failed: {}", e);
                // Non-intrusive feedback, could be a small status bar icon
            }
            self.last_autosave = Instant::now();
        }
    }
}

impl Drop for YadawApp {
    fn drop(&mut self) {
        let _ = self.input_manager.save_shortcuts(&shortcuts_path());
        let _ = self.theme_manager.save_custom_themes(&custom_themes_path());
        let _ = self.theme_manager.save_current_theme(&current_theme_path());
    }
}
