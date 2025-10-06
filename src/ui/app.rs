use super::*;
use crate::audio_state::AudioState;
use crate::config::Config;
use crate::constants::DEFAULT_MIN_PROJECT_BEATS;
use crate::edit_actions::EditProcessor;
use crate::error::{ResultExt, UserNotification, common};
use crate::model::automation::AutomationTarget;
use crate::model::plugin_api::UnifiedPluginInfo;
use crate::model::{AudioClip, MidiNote};
use crate::performance::{PerformanceMetrics, PerformanceMonitor};
use crate::project::{AppState, AppStateSnapshot};
use crate::project_manager::ProjectManager;

use crate::track_manager::{TrackManager, TrackType};
use crate::transport::Transport;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
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
    pub(super) available_plugins: Vec<UnifiedPluginInfo>,
    pub(super) selected_track_for_plugin: Option<usize>,

    // Selection state
    pub(super) selected_track: usize,
    pub(super) selected_pattern: usize,
    pub(super) selected_clips: Vec<(usize, usize)>,

    // Undo/Redo
    pub(super) undo_stack: Vec<AppStateSnapshot>,
    pub(super) redo_stack: Vec<AppStateSnapshot>,

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

    pub(super) note_clipboard: Option<Vec<MidiNote>>,
    pub(super) active_edit_target: ActiveEditTarget,
    pub(crate) last_real_metrics_at: Option<Instant>,
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
    ) -> Self {
        let transport = Transport::new(audio_state.clone(), command_tx.clone());
        let theme = match config.ui.theme {
            crate::config::Theme::Dark => super::theme::Theme::Dark,
            crate::config::Theme::Light => super::theme::Theme::Light,
        };

        Self {
            transport_ui: super::transport::TransportUI::new(transport),
            tracks_ui: super::tracks::TracksPanel::new(),
            timeline_ui: super::timeline::TimelineView::new(),
            mixer_ui: super::mixer::MixerWindow::new(),
            menu_bar: super::menu_bar::MenuBar::new(),
            piano_roll_view: super::piano_roll_view::PianoRollView::new(),
            dialogs: super::dialogs::DialogManager::new(),
            theme_manager: super::theme::ThemeManager::new(theme),

            state,
            audio_state,
            command_tx,
            ui_rx,
            config,
            available_plugins,

            selected_track: 0,
            selected_pattern: 0,
            selected_clips: Vec::new(),
            selected_track_for_plugin: None,

            undo_stack: Vec::new(),
            redo_stack: Vec::new(),

            project_path: None,
            clipboard: None,
            midi_clipboard: None,
            note_clipboard: None,

            active_edit_target: ActiveEditTarget::Clips,

            show_performance: false,
            performance_monitor: PerformanceMonitor::new(),
            track_manager: TrackManager::new(),
            project_manager: ProjectManager::new(),

            touch_state: TouchState {
                last_touch_pos: None,
                pinch_distance: None,
                gesture_start_time: None,
                tap_times: Vec::new(),
            },
            last_real_metrics_at: None,
        }
    }

    // Core functionality methods
    pub(super) fn push_undo(&mut self) {
        let state = self.state.lock().unwrap();
        self.undo_stack.push(state.snapshot());
        self.redo_stack.clear();

        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    pub(super) fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.redo_stack.push(current);
            state.restore(snapshot);
            drop(state);

            self.sync_views_after_model_change();
            let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        }
    }

    pub(super) fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.undo_stack.push(current);
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
        let track = self.track_manager.create_track(TrackType::Audio, None);
        state.tracks.push(track);
        state.ensure_ids();
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_midi_track(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        let track = self.track_manager.create_track(TrackType::Midi, None);
        state.tracks.push(track);
        state.ensure_ids();
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_bus_track(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        let track = self.track_manager.create_track(TrackType::Bus, None);
        state.tracks.push(track);
        state.ensure_ids();
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn duplicate_selected_track(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get(self.selected_track) {
            let new_track = self.track_manager.duplicate_track(track);
            state.tracks.insert(self.selected_track + 1, new_track);
            state.ensure_ids();
            drop(state);
            let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        }
    }

    pub fn delete_selected_track(&mut self) {
        if self.state.lock().unwrap().tracks.len() <= 1 {
            self.dialogs.show_message("Cannot delete the last track");
            return;
        }

        self.push_undo();
        let mut state = self.state.lock().unwrap();
        state.tracks.remove(self.selected_track);
        if self.selected_track >= state.tracks.len() {
            self.selected_track = state.tracks.len() - 1;
        }
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    // Clipboard operations
    pub fn cut_selected(&mut self) {
        self.copy_selected();
        self.delete_selected();
    }

    pub fn copy_selected(&mut self) {
        let state = self.state.lock().unwrap();
        let mut audio = Vec::new();
        let mut midi = Vec::new();

        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get(*track_id) {
                if track.is_midi {
                    if let Some(clip) = track.midi_clips.get(*clip_id) {
                        midi.push(clip.clone());
                    }
                } else if let Some(clip) = track.audio_clips.get(*clip_id) {
                    audio.push(clip.clone());
                }
            }
        }

        self.clipboard = if audio.is_empty() { None } else { Some(audio) };
        self.midi_clipboard = if midi.is_empty() { None } else { Some(midi) };
    }

    pub fn paste_at_playhead(&mut self) {
        self.push_undo();

        let current_beat = {
            let position = self.audio_state.get_position();
            let sample_rate = self.audio_state.sample_rate.load();
            let bpm = self.audio_state.bpm.load();
            (position / sample_rate as f64) * (bpm as f64 / 60.0)
        };

        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track) {
            if track.is_midi {
                if let Some(clips) = &self.midi_clipboard {
                    for clip in clips {
                        let mut new_clip = clip.clone();
                        new_clip.start_beat = current_beat;
                        track.midi_clips.push(new_clip);
                    }
                }
            } else if let Some(clips) = &self.clipboard {
                for clip in clips {
                    let mut new_clip = clip.clone();
                    new_clip.start_beat = current_beat;
                    track.audio_clips.push(new_clip);
                }
            }
        }
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn delete_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();
        let mut state = self.state.lock().unwrap();

        // Delete from end to start within each track
        // We already have track_id, clip_id tuples but they may come from multiple tracks
        // Sort by track, then reverse-clip index to keep indices valid
        let mut clips_to_delete = self.selected_clips.clone();
        clips_to_delete.sort_by(|a, b| {
            if a.0 == b.0 {
                b.1.cmp(&a.1)
            } else {
                a.0.cmp(&b.0)
            }
        });

        for (track_id, clip_id) in clips_to_delete {
            if let Some(track) = state.tracks.get_mut(track_id) {
                if track.is_midi {
                    if clip_id < track.midi_clips.len() {
                        track.midi_clips.remove(clip_id);
                    }
                } else if clip_id < track.audio_clips.len() {
                    track.audio_clips.remove(clip_id);
                }
            }
        }

        self.selected_clips.clear();
        // Notify audio thread of structure change
        drop(state);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    // Selection
    pub fn select_all(&mut self) {
        let state = self.state.lock().unwrap();
        self.selected_clips.clear();

        for (track_idx, track) in state.tracks.iter().enumerate() {
            if track.is_midi {
                for clip_idx in 0..track.midi_clips.len() {
                    self.selected_clips.push((track_idx, clip_idx));
                }
            } else {
                for clip_idx in 0..track.audio_clips.len() {
                    self.selected_clips.push((track_idx, clip_idx));
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
        self.selected_track = 0;
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
                state.ensure_ids();
                drop(state);

                self.project_path = Some(path.to_string_lossy().to_string());
                self.selected_track = 0;
                self.selected_clips.clear();
                self.undo_stack.clear();
                self.redo_stack.clear();

                let _ = self.command_tx.send(AudioCommand::UpdateTracks);
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
        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id)
                && let Some(clip) = track.audio_clips.get_mut(*clip_id)
            {
                let peak = clip.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                if peak > 0.0 {
                    let gain = crate::constants::NORMALIZE_TARGET_LINEAR / peak;
                    for s in &mut clip.samples {
                        *s *= gain;
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
        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id)
                && let Some(clip) = track.audio_clips.get_mut(*clip_id)
            {
                clip.samples.reverse();
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
        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id)
                && let Some(clip) = track.audio_clips.get_mut(*clip_id)
            {
                EditProcessor::apply_fade_in(clip, 0.25, bpm);
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
        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id)
                && let Some(clip) = track.audio_clips.get_mut(*clip_id)
            {
                EditProcessor::apply_fade_out(clip, 0.25, bpm);
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
        let mut state = self.state.lock().unwrap();
        let bpm = state.bpm;
        let selected_clips = self.selected_clips.clone();
        for (track_id, clip_id) in selected_clips {
            if let Some(track) = state.tracks.get_mut(track_id)
                && let Some(clip) = track.audio_clips.get(clip_id)
                && let Some((first_half, second_half)) =
                    EditProcessor::split_clip(clip, current_beat, bpm)
            {
                track.audio_clips[clip_id] = first_half;
                track.audio_clips.insert(clip_id + 1, second_half);
            }
        }
        drop(state);
        self.selected_clips.clear();
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    // MIDI operations
    pub fn quantize_selected_notes(&mut self, strength: f32) {
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track)
            && let Some(clip) = track.midi_clips.get_mut(self.selected_pattern)
        {
            EditProcessor::quantize_notes(
                &mut clip.notes,
                crate::constants::DEFAULT_GRID_SNAP as f64,
                strength,
            );
        }
    }

    pub fn quantize_selected_notes_with_params(&mut self, strength: f32, grid: f32, _swing: f32) {
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track)
            && let Some(clip) = track.midi_clips.get_mut(self.selected_pattern)
        {
            EditProcessor::quantize_notes(&mut clip.notes, grid as f64, strength);
        }
    }

    pub fn transpose_selected_notes(&mut self, semitones: i32) {
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track)
            && let Some(clip) = track.midi_clips.get_mut(self.selected_pattern)
        {
            EditProcessor::transpose_notes(&mut clip.notes, semitones);
        }
    }

    pub fn humanize_selected_notes(&mut self, amount: f32) {
        self.push_undo();

        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track)
            && let Some(clip) = track.midi_clips.get_mut(self.selected_pattern)
        {
            EditProcessor::humanize_notes(&mut clip.notes, amount);
        }
    }

    // UI operations
    pub fn show_plugin_browser_for_track(&mut self, track_id: usize) {
        self.selected_track_for_plugin = Some(track_id);
        self.dialogs.show_plugin_browser();
    }

    pub fn add_automation_lane(&mut self, track_id: usize, target: AutomationTarget) {
        let _ = self
            .command_tx
            .send(AudioCommand::AddAutomationPoint(track_id, target, 0.0, 0.5));
    }

    pub fn zoom_to_fit(&mut self) {
        // Calculate the extent of all content
        let state = self.state.lock().unwrap();
        let mut max_beat: f64 = DEFAULT_MIN_PROJECT_BEATS; // Minimum 4 beats

        for track in &state.tracks {
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
        // TODO: Implement audio export
        self.dialogs
            .show_message("Audio export not yet implemented");
    }

    pub fn is_selected_track_midi(&self) -> bool {
        let state = self.state.lock().unwrap();
        state
            .tracks
            .get(self.selected_track)
            .map(|t| t.is_midi)
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
                let mut state = self.state.lock().unwrap();
                clip.id = state.fresh_id();
                if let Some(track) = state.tracks.get_mut(track_id) {
                    track.audio_clips.push(clip);
                }
            }
            UIUpdate::RecordingLevel(_level) => {}
            UIUpdate::MasterLevel(_left, _right) => {}
            UIUpdate::PushUndo(snapshot) => {
                self.undo_stack.push(snapshot);
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
                let metrics = PerformanceMetrics {
                    cpu_usage,
                    memory_usage: self.estimate_memory_usage(),
                    disk_streaming_rate: 0.0,
                    audio_buffer_health: buffer_fill,
                    plugin_processing_time: Duration::from_secs_f32(plugin_time_ms / 1000.0),
                    xruns: xruns as usize,
                    latency_ms,
                };
                self.performance_monitor.update_metrics(metrics);
                self.last_real_metrics_at = Some(Instant::now());
            }
            _ => {}
        }
    }

    pub fn set_loop_to_selection(&mut self) {
        if self.selected_clips.is_empty() {
            // If no clips selected, use visible timeline region
            let visible_start = self.timeline_ui.scroll_x / self.timeline_ui.zoom_x;
            let visible_end = visible_start + (800.0 / self.timeline_ui.zoom_x); // Approximate width

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

            for (track_id, clip_id) in &self.selected_clips {
                if let Some(track) = state.tracks.get(*track_id) {
                    if let Some(clip) = track.audio_clips.get(*clip_id) {
                        min_beat = min_beat.min(clip.start_beat);
                        max_beat = max_beat.max(clip.start_beat + clip.length_beats);
                    }
                    // Also check MIDI clips
                    if let Some(clip) = track.midi_clips.get(*clip_id) {
                        min_beat = min_beat.min(clip.start_beat);
                        max_beat = max_beat.max(clip.start_beat + clip.length_beats);
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

    pub fn open_midi_clip_in_piano_roll(&mut self, clip_idx: usize) {
        self.piano_roll_view.set_editing_clip(clip_idx);
        // The UI will automatically switch to piano roll view since track is MIDI
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

    fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        // Respect focused text inputs for the rest
        if ctx.wants_keyboard_input() {
            return;
        }

        // Space is truly global.
        let sc_space = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Space);
        if ctx.input_mut(|i| i.consume_shortcut(&sc_space)) {
            self.transport_ui.toggle_playback(&self.command_tx);
        }

        ctx.input_mut(|i| {
            let cmd = egui::Modifiers::COMMAND; // Ctrl on Linux/Windows

            // File shortcuts
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::N)) {
                self.new_project();
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::O)) {
                self.dialogs.show_open_dialog();
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::S)) {
                if i.modifiers.shift {
                    self.dialogs.show_save_dialog();
                } else {
                    self.save_project();
                }
            }

            // Edit shortcuts (clip-level)
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::Z)) {
                if i.modifiers.shift {
                    self.redo();
                } else {
                    self.undo();
                }
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::X)) {
                self.cut_selected();
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::C)) {
                self.copy_selected();
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::V)) {
                self.paste_at_playhead();
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::A)) {
                self.select_all();
            }

            // Transport / loop
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Home)
                && let Some(transport) = &mut self.transport_ui.transport
            {
                transport.set_position(0.0);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::L) && !i.modifiers.ctrl {
                // Toggle loop
                let enabled = !self.audio_state.loop_enabled.load(Ordering::Relaxed);
                self.audio_state
                    .loop_enabled
                    .store(enabled, Ordering::Relaxed);
                let _ = self.command_tx.send(AudioCommand::SetLoopEnabled(enabled));
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::L)) {
                // Set loop to selection
                self.set_loop_to_selection();
            }
            if i.consume_key(egui::Modifiers::SHIFT, egui::Key::L) {
                // Clear loop
                self.audio_state
                    .loop_enabled
                    .store(false, Ordering::Relaxed);
                let _ = self.command_tx.send(AudioCommand::SetLoopEnabled(false));
            }

            // Delete clips
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                && !self.selected_clips.is_empty()
            {
                self.delete_selected();
            }

            // View
            if i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::M)) {
                self.mixer_ui.toggle_visibility();
            }
        });
    }

    fn handle_touch_gestures(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Handle touch events
            if let Some(touch) = i.events.iter().find_map(|e| {
                if let egui::Event::Touch { .. } = e {
                    Some(e)
                } else {
                    None
                }
            }) && let egui::Event::Touch {
                device_id: _,
                id: _,
                phase,
                pos,
                force: _,
            } = touch
            {
                match phase {
                    egui::TouchPhase::Start => {
                        self.touch_state.last_touch_pos = Some(*pos);
                        self.touch_state.gesture_start_time = Some(Instant::now());
                    }
                    egui::TouchPhase::Move => {
                        if let Some(last_pos) = self.touch_state.last_touch_pos {
                            let delta = *pos - last_pos;

                            // Pan gesture
                            if delta.length() > 5.0 {
                                self.timeline_ui.scroll_x -= delta.x;
                                self.timeline_ui.scroll_y -= delta.y;
                            }

                            self.touch_state.last_touch_pos = Some(*pos);
                        }
                    }
                    egui::TouchPhase::End => {
                        // Check for tap vs long press
                        if let Some(start_time) = self.touch_state.gesture_start_time {
                            let duration = Instant::now().duration_since(start_time);

                            if duration.as_millis() < 200 {
                                // Tap - treat as click
                            } else {
                                // Long press - show context menu
                            }
                        }

                        self.touch_state.last_touch_pos = None;
                        self.touch_state.gesture_start_time = None;
                    }
                    egui::TouchPhase::Cancel => {
                        self.touch_state.last_touch_pos = None;
                        self.touch_state.gesture_start_time = None;
                    }
                }
            }

            // Handle multi-touch (pinch zoom)
            let touches: Vec<_> = i
                .events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Touch { pos, .. } = e {
                        Some(*pos)
                    } else {
                        None
                    }
                })
                .collect();

            if touches.len() == 2 {
                let distance = (touches[0] - touches[1]).length();

                if let Some(last_distance) = self.touch_state.pinch_distance {
                    let scale = distance / last_distance;

                    // Apply zoom
                    self.timeline_ui.zoom_x *= scale;
                    self.timeline_ui.zoom_x = self.timeline_ui.zoom_x.clamp(10.0, 500.0);
                }

                self.touch_state.pinch_distance = Some(distance);
            } else {
                self.touch_state.pinch_distance = None;
            }
        });
    }

    pub fn switch_to_piano_roll(&mut self) {
        let midi_idx = {
            let state = self.state.lock().unwrap();
            state.tracks.iter().position(|t| t.is_midi)
        };
        if let Some(idx) = midi_idx {
            self.selected_track = idx;
        } else {
            self.dialogs
                .show_message("No MIDI track found. Add a MIDI track first.");
        }
    }

    pub fn switch_to_timeline(&mut self) {
        let audio_idx = {
            let state = self.state.lock().unwrap();
            state.tracks.iter().position(|t| !t.is_midi)
        };
        if let Some(idx) = audio_idx {
            self.selected_track = idx;
        } else {
            self.dialogs.show_message("No audio track found.");
        }
    }

    fn estimate_memory_usage(&self) -> usize {
        let state = self.state.lock().unwrap();
        let mut total = 0;

        for track in &state.tracks {
            for clip in &track.audio_clips {
                total += clip.samples.len() * std::mem::size_of::<f32>();
            }
            for clip in &track.midi_clips {
                total += clip.notes.len() * std::mem::size_of::<MidiNote>();
            }
        }

        total
    }

    fn estimate_cpu_usage(&self) -> f32 {
        // Simple estimation based on track count and playing state
        let is_playing = self.audio_state.playing.load(Ordering::Relaxed);
        let track_count = self.state.lock().unwrap().tracks.len();

        if is_playing {
            0.1 + (track_count as f32 * 0.05) // 5% per track + 10% base
        } else {
            0.05 // 5% idle
        }
    }

    fn update_performance_metrics(&mut self) {
        let stale = self
            .last_real_metrics_at
            .map(|t| t.elapsed() >= std::time::Duration::from_millis(200))
            .unwrap_or(true);
        if !stale {
            return;
        }
        // Calculate current metrics
        let cpu_usage = self.estimate_cpu_usage();
        let memory_usage = self.estimate_memory_usage();

        let metrics = PerformanceMetrics {
            cpu_usage,
            memory_usage,
            disk_streaming_rate: 0.0, // TODO: Track actual disk I/O
            audio_buffer_health: 1.0 - cpu_usage, // Simple approximation
            plugin_processing_time: Duration::from_millis(0), // TODO: Track plugin time
            xruns: 0,                 // TODO: Get from audio thread
            latency_ms: (512.0 / self.audio_state.sample_rate.load()) * 1000.0,
        };

        self.performance_monitor.update_metrics(metrics);
    }

    fn sync_views_after_model_change(&mut self) {
        // Clamp selected_track
        let tracks_len = self.state.lock().unwrap().tracks.len();
        if tracks_len == 0 {
            self.selected_track = 0;
            self.piano_roll_view.selected_clip = None;
            return;
        }
        if self.selected_track >= tracks_len {
            self.selected_track = tracks_len - 1;
        }

        // Refresh piano roll notes if a clip is selected
        if let Some(clip_idx) = self.piano_roll_view.selected_clip {
            let notes_opt = {
                let state = self.state.lock().unwrap();
                state
                    .tracks
                    .get(self.selected_track)
                    .and_then(|t| t.midi_clips.get(clip_idx))
                    .map(|c| c.notes.clone())
            };
        }
    }

    fn ctx(&self) -> &egui::Context {
        // This would need to be passed in or stored
        // For now, return a placeholder
        todo!("Context should be passed to methods that need it")
    }
}

impl eframe::App for YadawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme
        self.theme_manager.apply_theme(ctx);

        // Process UI updates from audio thread
        while let Ok(update) = self.ui_rx.try_recv() {
            self.process_ui_update(update, ctx);
        }

        // Handle touch gestures
        self.handle_touch_gestures(ctx);

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

        self.update_performance_metrics(); //TODO: maybe reduce calls for performance (irony? or it's just overexaggerated)

        // Handle global shortcuts
        self.handle_global_shortcuts(ctx);

        // Request repaint if playing
        if self.audio_state.playing.load(Ordering::Relaxed) {
            ctx.request_repaint();
        }
    }
}
