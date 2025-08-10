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

    fn save_project(&mut self) {
        // Implementation moved from original
    }

    fn show_performance_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Performance Monitor")
            .open(&mut self.show_performance)
            .show(ctx, |ui| {
                // Performance monitor UI
            });
    }
}
