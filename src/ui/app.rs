use super::*;
use crate::audio_state::AudioState;
use crate::config::Config;
use crate::level_meter::LevelMeter;
use crate::lv2_plugin_host::PluginInfo;
use crate::performance::PerformanceMonitor;
use crate::piano_roll::PianoRoll;
use crate::project_manager::ProjectManager;
use crate::state::{AppState, AppStateSnapshot, AudioClip};
use crate::track_manager::TrackManager;
use crate::transport::{Transport, TransportState};

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
    pub(super) available_plugins: Vec<PluginInfo>,

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
    pub(super) show_performance: bool,
    pub(super) performance_monitor: PerformanceMonitor,
}

impl YadawApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        audio_state: Arc<AudioState>,
        command_tx: Sender<AudioCommand>,
        ui_rx: Receiver<UIUpdate>,
        available_plugins: Vec<PluginInfo>,
        config: Config,
    ) -> Self {
        let transport = Transport::new(audio_state.clone(), command_tx.clone());

        Self {
            // Initialize transport UI with the transport
            transport_ui: super::transport::TransportUI::new(transport),
            tracks_ui: super::tracks::TracksPanel::new(),
            timeline_ui: super::timeline::TimelineView::new(),
            mixer_ui: super::mixer::MixerWindow::new(),
            menu_bar: super::menu_bar::MenuBar::new(),
            piano_roll_view: super::piano_roll_view::PianoRollView::new(),
            dialogs: super::dialogs::DialogManager::new(),
            theme_manager: super::theme::ThemeManager::new(config.ui.theme.clone()),

            state,
            audio_state,
            command_tx,
            ui_rx,
            config,
            available_plugins,

            selected_track: 0,
            selected_pattern: 0,
            selected_clips: Vec::new(),

            undo_stack: Vec::new(),
            redo_stack: Vec::new(),

            project_path: None,
            clipboard: None,
            show_performance: false,
            performance_monitor: PerformanceMonitor::new(),
        }
    }

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
        }
    }

    pub(super) fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop() {
            let mut state = self.state.lock().unwrap();
            let current = state.snapshot();
            self.undo_stack.push(current);
            state.restore(snapshot);
        }
    }
}

impl eframe::App for YadawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme
        self.theme_manager.apply_theme(ctx);

        // Process UI updates from audio thread
        while let Ok(update) = self.ui_rx.try_recv() {
            self.process_ui_update(update);
        }

        // Draw menu bar
        self.menu_bar.show(ctx, self);

        // Draw transport
        self.transport_ui.show(ctx, self);

        // Draw main panels
        self.show_main_panels(ctx);

        // Draw floating windows
        self.show_floating_windows(ctx);

        // Handle global shortcuts
        self.handle_global_shortcuts(ctx);

        // Request repaint if playing
        if self
            .audio_state
            .playing
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            ctx.request_repaint();
        }
    }
}

impl YadawApp {
    fn process_ui_update(&mut self, update: UIUpdate) {
        // Handle UI updates from audio thread
        match update {
            UIUpdate::Position(pos) => {
                // Update position displays
            }
            UIUpdate::TrackLevels(levels) => {
                self.tracks_ui.update_levels(levels);
            }
            // ... handle other updates
            _ => {}
        }
    }

    fn show_main_panels(&mut self, ctx: &egui::Context) {
        // Left panel - Tracks
        egui::SidePanel::left("tracks_panel")
            .default_width(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.tracks_ui.show(ui, self);
            });

        // Central panel - Timeline or Piano Roll
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_selected_track_midi() {
                self.piano_roll_view.show(ui, self);
            } else {
                self.timeline_ui.show(ui, self);
            }
        });
    }

    fn show_floating_windows(&mut self, ctx: &egui::Context) {
        // Mixer window
        if self.mixer_ui.is_visible() {
            self.mixer_ui.show(ctx, self);
        }

        // Dialogs
        self.dialogs.show_all(ctx, self);

        // Performance monitor
        if self.show_performance {
            self.show_performance_window(ctx);
        }
    }

    fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if i.modifiers.ctrl {
                if i.key_pressed(egui::Key::Z) {
                    if i.modifiers.shift {
                        self.redo();
                    } else {
                        self.undo();
                    }
                }

                if i.key_pressed(egui::Key::S) {
                    self.save_project();
                }
            }

            if i.key_pressed(egui::Key::Space) {
                self.transport_ui.toggle_playback(&self.command_tx);
            }
        });
    }

    fn is_selected_track_midi(&self) -> bool {
        let state = self.state.lock().unwrap();
        state
            .tracks
            .get(self.selected_track)
            .map(|t| t.is_midi)
            .unwrap_or(false)
    }

    fn show_performance_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Performance Monitor")
            .open(&mut self.show_performance)
            .show(ctx, |ui| {
                // Performance monitor UI
            });
    }

    pub fn add_audio_track(&mut self) {
        let mut state = self.state.lock().unwrap();
        let track = self
            .track_manager
            .create_track(crate::track_manager::TrackType::Audio, None);
        state.tracks.push(track);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_midi_track(&mut self) {
        let mut state = self.state.lock().unwrap();
        let track = self
            .track_manager
            .create_track(crate::track_manager::TrackType::Midi, None);
        state.tracks.push(track);
        let _ = self.command_tx.send(AudioCommand::UpdateTracks);
    }

    pub fn add_bus_track(&mut self) {
        // Implementation for bus tracks
    }

    pub fn duplicate_selected_track(&mut self) {
        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get(self.selected_track) {
            let new_track = self.track_manager.duplicate_track(track);
            state.tracks.insert(self.selected_track + 1, new_track);
            let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        }
    }

    pub fn delete_selected_track(&mut self) {
        let mut state = self.state.lock().unwrap();
        if state.tracks.len() > 1 {
            state.tracks.remove(self.selected_track);
            if self.selected_track >= state.tracks.len() {
                self.selected_track = state.tracks.len() - 1;
            }
            let _ = self.command_tx.send(AudioCommand::UpdateTracks);
        }
    }

    // Clipboard operations
    pub fn cut_selected(&mut self) {
        self.copy_selected();
        self.delete_selected();
    }

    pub fn copy_selected(&mut self) {
        let state = self.state.lock().unwrap();
        let mut clips = Vec::new();

        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get(*track_id) {
                if let Some(clip) = track.audio_clips.get(*clip_id) {
                    clips.push(clip.clone());
                }
            }
        }

        self.clipboard = Some(clips);
    }

    pub fn paste_at_playhead(&mut self) {
        // Implementation from original
    }

    pub fn delete_selected(&mut self) {
        // Implementation from original
    }

    // Selection
    pub fn select_all(&mut self) {
        let state = self.state.lock().unwrap();
        self.selected_clips.clear();

        for (track_idx, track) in state.tracks.iter().enumerate() {
            for clip_idx in 0..track.audio_clips.len() {
                self.selected_clips.push((track_idx, clip_idx));
            }
        }
    }

    pub fn deselect_all(&mut self) {
        self.selected_clips.clear();
    }

    // Project management
    pub fn new_project(&mut self) {
        let mut state = self.state.lock().unwrap();
        *state = crate::state::AppState::new();
        self.project_path = None;
        self.selected_track = 0;
        self.selected_clips.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub fn save_project(&mut self) {
        if let Some(path) = &self.project_path {
            self.save_project_to_path(&std::path::Path::new(path));
        } else {
            self.dialogs.show_save_dialog();
        }
    }

    pub fn save_project_to_path(&mut self, path: &std::path::Path) {
        let state = self.state.lock().unwrap();
        if let Err(e) = self.project_manager.save_project(&*state, path) {
            self.dialogs
                .show_message(&format!("Failed to save project: {}", e));
        } else {
            self.project_path = Some(path.to_string_lossy().to_string());
            self.dialogs.show_message("Project saved successfully");
        }
    }

    pub fn load_project_from_path(&mut self, path: &std::path::Path) {
        match self.project_manager.load_project(path) {
            Ok(project) => {
                let mut state = self.state.lock().unwrap();
                state.load_project(project);
                self.project_path = Some(path.to_string_lossy().to_string());
                let _ = self.command_tx.send(AudioCommand::UpdateTracks);
            }
            Err(e) => {
                self.dialogs
                    .show_message(&format!("Failed to load project: {}", e));
            }
        }
    }

    // Audio operations
    fn normalize_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();

        let mut state = self.state.lock().unwrap();

        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                    // Find peak
                    let peak = clip
                        .samples
                        .iter()
                        .map(|s| s.abs())
                        .fold(0.0f32, |a, b| a.max(b));

                    if peak > 0.0 {
                        // Normalize to -0.1 dB (0.989)
                        let gain = 0.989 / peak;
                        for sample in &mut clip.samples {
                            *sample *= gain;
                        }
                    }
                }
            }
        }
    }

    fn reverse_selected(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }

        self.push_undo();

        let mut state = self.state.lock().unwrap();

        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                    clip.samples.reverse();
                }
            }
        }
    }

    fn apply_fade_in(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                    EditProcessor::apply_fade_in(clip, 0.25); // Quarter beat fade
                }
            }
        }
    }

    fn apply_fade_out(&mut self) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        for (track_id, clip_id) in &self.selected_clips {
            if let Some(track) = state.tracks.get_mut(*track_id) {
                if let Some(clip) = track.audio_clips.get_mut(*clip_id) {
                    EditProcessor::apply_fade_out(clip, 0.25); // Quarter beat fade
                }
            }
        }
    }

    fn split_selected_at_playhead(&mut self) {
        if self.selected_clips.is_empty() {
            return;
        }
        let selected_clips = self.selected_clips.clone();

        self.push_undo();

        let current_beat = {
            let position = self.audio_state.get_position();
            let sample_rate = self.audio_state.sample_rate.load();
            let bpm = self.audio_state.bpm.load();
            (position / sample_rate as f64) * (bpm as f64 / 60.0)
        };

        let mut state = self.state.lock().unwrap();
        let bpm = state.bpm;

        for (track_id, clip_id) in selected_clips {
            if let Some(track) = state.tracks.get_mut(track_id) {
                if let Some(clip) = track.audio_clips.get_mut(clip_id) {
                    // Check if playhead is within clip
                    if current_beat > clip.start_beat
                        && current_beat < clip.start_beat + clip.length_beats
                    {
                        // Calculate split point
                        let split_offset = current_beat - clip.start_beat;
                        let split_sample =
                            (split_offset * 60.0 / bpm as f64 * clip.sample_rate as f64) as usize;

                        if split_sample < clip.samples.len() {
                            // Create new clip from split point
                            let new_clip = AudioClip {
                                name: format!("{} (2)", clip.name),
                                start_beat: current_beat,
                                length_beats: clip.length_beats - split_offset,
                                samples: clip.samples[split_sample..].to_vec(),
                                sample_rate: clip.sample_rate,
                            };

                            clip.length_beats = split_offset;
                            clip.samples.truncate(split_sample);

                            track.audio_clips.push(new_clip);
                        }
                    }
                }
            }
        }
    }

    // MIDI operations
    fn quantize_selected_notes(&mut self, strength: f32) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track) {
            if let Some(pattern) = track.patterns.get_mut(self.selected_pattern) {
                EditProcessor::quantize_notes(
                    &mut pattern.notes,
                    DEFAULT_GRID_SNAP as f64,
                    strength,
                );
            }
        }
    }

    pub fn quantize_selected_notes_with_params(&mut self, strength: f32, grid: f32, swing: f32) {
        // Extended quantize implementation
    }

    fn transpose_selected_notes(&mut self, semitones: i32) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track) {
            if let Some(pattern) = track.patterns.get_mut(self.selected_pattern) {
                EditProcessor::transpose_notes(&mut pattern.notes, semitones);
            }
        }
    }

    fn humanize_selected_notes(&mut self, amount: f32) {
        self.push_undo();
        let mut state = self.state.lock().unwrap();
        if let Some(track) = state.tracks.get_mut(self.selected_track) {
            if let Some(pattern) = track.patterns.get_mut(self.selected_pattern) {
                EditProcessor::humanize_notes(&mut pattern.notes, amount);
            }
        }
    }

    // UI operations
    pub fn show_plugin_browser_for_track(&mut self, track_id: usize) {
        self.selected_track_for_plugin = Some(track_id);
        self.dialogs.show_plugin_browser();
    }

    pub fn add_automation_lane(&mut self, track_id: usize, target: crate::state::AutomationTarget) {
        let _ = self
            .command_tx
            .send(AudioCommand::AddAutomationPoint(track_id, target, 0.0, 0.5));
    }

    pub fn zoom_to_fit(&mut self) {
        // Calculate zoom to fit all content
    }

    pub fn reset_layout(&mut self) {
        // Reset UI layout to defaults
    }

    pub fn tap_tempo(&mut self) {
        // Implement tap tempo functionality
    }

    pub fn import_audio_dialog(&mut self) {
        // Implementation from original
    }

    pub fn export_audio_dialog(&mut self) {
        // Show export dialog
    }
}
