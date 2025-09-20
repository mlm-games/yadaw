use std::path::PathBuf;

use egui_file_dialog::FileDialog;

use super::*;
use crate::edit_actions::EditProcessor;
use crate::error::{ResultExt, UserNotification, common};
use crate::messages::AudioCommand;
use crate::model::plugin_api::BackendKind;
use crate::plugin::{categorize_plugin, categorize_unified_plugin};
use crate::ui::app::FileDialogPurpose;
use crate::ui::theme;

macro_rules! simple_dialog {
    ($name:ident, $title:expr, $content:expr) => {
        pub struct $name {
            closed: bool,
        }

        impl $name {
            pub fn new() -> Self {
                Self { closed: false }
            }

            pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
                let mut open = true;
                egui::Window::new($title)
                    .open(&mut open)
                    .resizable(false)
                    .show(ctx, |ui| {
                        $content(ui, app, &mut self.closed);
                    });
                if !open {
                    self.closed = true;
                }
            }

            pub fn is_closed(&self) -> bool {
                self.closed
            }
        }
    };
}

impl UserNotification for DialogManager {
    fn show_error(&mut self, message: &str) {
        self.message_box = Some(DialogWrapper::new(MessageContent::new(message.to_string())));
    }

    fn show_success(&mut self, message: &str) {
        self.show_message(message);
    }

    fn show_warning(&mut self, message: &str) {
        self.show_message(message);
    }

    fn show_info(&mut self, message: &str) {
        self.show_message(message);
    }
}

/// Base trait for all dialog implementations
pub trait Dialog {
    /// Draw the dialog content (returns true if dialog should close)
    fn draw_content(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) -> bool;

    /// Get the dialog title
    fn title(&self) -> &str;

    /// Check if dialog is closed
    fn is_closed(&self) -> bool;

    /// Optional: Configure window properties
    fn configure_window<'a>(&self, window: egui::Window<'a>) -> egui::Window<'a> {
        window.resizable(false).collapsible(false)
    }
}

/// Generic dialog wrapper that handles common window logic
pub struct DialogWrapper<T: Dialog> {
    inner: T,
    closed: bool,
}

impl<T: Dialog> DialogWrapper<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            closed: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = !self.closed;

        let window = egui::Window::new(self.inner.title()).open(&mut open);

        let window = self.inner.configure_window(window);

        window.show(ctx, |ui| {
            if self.inner.draw_content(ui, app) {
                self.closed = true;
            }
        });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed || self.inner.is_closed()
    }
}

/// Simple message dialog
pub struct MessageContent {
    message: String,
}

impl MessageContent {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl Dialog for MessageContent {
    fn title(&self) -> &str {
        "Message"
    }

    fn draw_content(&mut self, ui: &mut egui::Ui, _app: &mut super::app::YadawApp) -> bool {
        ui.label(&self.message);
        ui.separator();
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                close = true;
            }
        });
        close
    }

    fn is_closed(&self) -> bool {
        false // Wrapper handles this
    }
}

pub type MessageBox = DialogWrapper<MessageContent>;

/// Quantize dialog using the new pattern
pub struct QuantizeContent {
    strength: f32,
    grid_size: f32,
    swing: f32,
}

impl QuantizeContent {
    pub fn new() -> Self {
        Self {
            strength: 1.0,
            grid_size: 0.25,
            swing: 0.0,
        }
    }
}

impl Dialog for QuantizeContent {
    fn title(&self) -> &str {
        "Quantize"
    }

    fn draw_content(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) -> bool {
        ui.horizontal(|ui| {
            ui.label("Strength:");
            ui.add(
                egui::Slider::new(&mut self.strength, 0.0..=1.0)
                    .suffix("%")
                    .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Grid:");
            egui::ComboBox::from_id_salt("quantize_grid")
                .selected_text(format!("1/{}", (1.0 / self.grid_size) as i32))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.grid_size, 1.0, "1/1");
                    ui.selectable_value(&mut self.grid_size, 0.5, "1/2");
                    ui.selectable_value(&mut self.grid_size, 0.25, "1/4");
                    ui.selectable_value(&mut self.grid_size, 0.125, "1/8");
                    ui.selectable_value(&mut self.grid_size, 0.0625, "1/16");
                    ui.selectable_value(&mut self.grid_size, 0.03125, "1/32");
                });
        });
        ui.horizontal(|ui| {
            ui.label("Swing:");
            ui.add(egui::Slider::new(&mut self.swing, -50.0..=50.0).suffix("%"));
        });
        ui.separator();
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                app.quantize_selected_notes_with_params(self.strength, self.grid_size, self.swing);
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        close
    }

    fn is_closed(&self) -> bool {
        false
    }
}

pub type QuantizeDialog = DialogWrapper<QuantizeContent>;

pub struct DialogManager {
    pub message_box: Option<MessageBox>,
    pub quantize_dialog: Option<QuantizeDialog>,

    pub open_dialog: Option<OpenDialog>,
    pub save_dialog: Option<SaveDialog>,

    pub audio_setup: Option<AudioSetupDialog>,
    pub plugin_browser: Option<PluginBrowserDialog>,
    pub plugin_manager: Option<PluginManagerDialog>,

    pub transpose_dialog: Option<TransposeDialog>,
    pub humanize_dialog: Option<HumanizeDialog>,
    pub time_stretch_dialog: Option<TimeStretchDialog>,

    pub project_settings: Option<ProjectSettingsDialog>,
    pub export_dialog: Option<ExportDialog>,

    pub theme_editor: Option<ThemeEditorDialog>,
    pub layout_manager: Option<LayoutManagerDialog>,

    // Utility
    pub progress_bar: Option<ProgressBar>,
    pub track_grouping: Option<TrackGroupingDialog>,
    pub track_rename: Option<TrackRenameDialog>,
    pub import_audio: Option<ImportAudioDialog>,
}

impl DialogManager {
    pub fn new() -> Self {
        Self {
            open_dialog: None,
            save_dialog: None,
            audio_setup: None,
            plugin_browser: None,
            plugin_manager: None,
            quantize_dialog: None,
            transpose_dialog: None,
            humanize_dialog: None,
            time_stretch_dialog: None,
            project_settings: None,
            export_dialog: None,
            theme_editor: None,
            layout_manager: None,
            message_box: None,
            progress_bar: None,
            track_grouping: None,
            track_rename: None,
            import_audio: None,
        }
    }

    pub fn open_import_audio(&mut self) {
        let mut dlg = ImportAudioDialog::new();
        dlg.open();
        self.import_audio = Some(dlg);
    }

    pub fn show_all(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        // File dialogs
        if let Some(mut d) = self.open_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.open_dialog = Some(d);
            }
        }
        if let Some(mut d) = self.save_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.save_dialog = Some(d);
            }
        }

        // Tools / audio dialogs
        if let Some(mut d) = self.audio_setup.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.audio_setup = Some(d);
            }
        }
        if let Some(mut d) = self.plugin_browser.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.plugin_browser = Some(d);
            }
        }
        if let Some(mut d) = self.plugin_manager.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.plugin_manager = Some(d);
            }
        }
        if let Some(mut d) = self.track_grouping.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.track_grouping = Some(d);
            }
        }

        // Edit dialogs
        if let Some(mut d) = self.quantize_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.quantize_dialog = Some(d);
            }
        }
        if let Some(mut d) = self.transpose_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.transpose_dialog = Some(d);
            }
        }
        if let Some(mut d) = self.humanize_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.humanize_dialog = Some(d);
            }
        }
        if let Some(mut d) = self.time_stretch_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.time_stretch_dialog = Some(d);
            }
        }

        // Project dialogs
        if let Some(mut d) = self.project_settings.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.project_settings = Some(d);
            }
        }
        if let Some(mut d) = self.export_dialog.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.export_dialog = Some(d);
            }
        }

        // UI dialogs
        if let Some(mut d) = self.theme_editor.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.theme_editor = Some(d);
            }
        }
        if let Some(mut d) = self.layout_manager.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.layout_manager = Some(d);
            }
        }

        // Utility
        if let Some(mut d) = self.message_box.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.message_box = Some(d);
            }
        }
        if let Some(mut d) = self.progress_bar.take() {
            d.show(ctx);
            if !d.is_closed() {
                self.progress_bar = Some(d);
            }
        }
        if let Some(mut d) = self.track_rename.take() {
            d.show(ctx, app);
            if !d.is_closed() {
                self.track_rename = Some(d);
            }
        }
        if let Some(d) = self.import_audio.as_mut() {
            d.show(ctx, app);
        }
        if let Some(d) = &self.import_audio {
            if !d.is_open() {
                self.import_audio = None;
            }
        }
    }

    pub fn show_project_settings(&mut self) {
        self.project_settings = Some(ProjectSettingsDialog::new());
    }
    pub fn show_track_grouping(&mut self) {
        self.track_grouping = Some(TrackGroupingDialog::new());
    }
    pub fn show_plugin_manager(&mut self) {
        self.plugin_manager = Some(PluginManagerDialog::new());
    }
    pub fn show_transpose_dialog(&mut self) {
        self.transpose_dialog = Some(TransposeDialog::new(TransposeContent::new()));
    }
    pub fn show_humanize_dialog(&mut self) {
        self.humanize_dialog = Some(HumanizeDialog::new());
    }
    pub fn show_time_stretch_dialog(&mut self) {
        self.time_stretch_dialog = Some(TimeStretchDialog::new());
    }
    pub fn show_save_layout_dialog(&mut self) {
        self.layout_manager = Some(LayoutManagerDialog::new());
    }
    pub fn show_load_layout_dialog(&mut self) {
        self.layout_manager = Some(LayoutManagerDialog::new());
    }

    pub fn show_message(&mut self, message: &str) {
        self.message_box = Some(DialogWrapper::new(MessageContent::new(message.to_string())));
    }

    pub fn show_quantize_dialog(&mut self) {
        self.quantize_dialog = Some(DialogWrapper::new(QuantizeContent::new()));
    }

    pub fn show_open_dialog(&mut self) {
        self.open_dialog = Some(OpenDialog::new());
    }

    pub fn show_save_dialog(&mut self) {
        self.save_dialog = Some(SaveDialog::new());
    }

    pub fn show_plugin_browser(&mut self) {
        self.plugin_browser = Some(PluginBrowserDialog::new());
    }

    pub fn show_audio_setup(&mut self) {
        self.audio_setup = Some(AudioSetupDialog::new());
    }

    pub fn show_theme_editor(&mut self) {
        self.theme_editor = Some(ThemeEditorDialog::new());
    }

    pub fn show_rename_track(&mut self, idx: usize, current: String) {
        self.track_rename = Some(TrackRenameDialog::new(idx, current));
    }
}

// Individual dialog implementations
pub struct OpenDialog {
    closed: bool,
    fd: FileDialog,
    opened: bool,
}

impl OpenDialog {
    pub fn new() -> Self {
        let fd = FileDialog::new()
            .title("Open Project")
            .add_file_filter_extensions("YADAW Project", (&["yadaw", "ydw"]).to_vec())
            .add_file_filter_extensions("All Files", (&["*"]).to_vec());
        Self {
            closed: false,
            fd,
            opened: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        if !self.opened {
            self.fd.pick_file();
            self.opened = true;
        }
        self.fd.update(ctx);
        if let Some(path) = self.fd.take_picked() {
            app.load_project_from_path(&path);
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct SaveDialog {
    closed: bool,
    fd: FileDialog,
    opened: bool,
}

impl SaveDialog {
    pub fn new() -> Self {
        let fd = FileDialog::new()
            .title("Save Project")
            .default_file_name("untitled.yadaw")
            .add_file_filter_extensions("YADAW Project", (&["yadaw"]).to_vec())
            .allow_file_overwrite(true);
        Self {
            closed: false,
            fd,
            opened: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        if !self.opened {
            self.fd.save_file();
            self.opened = true;
        }
        self.fd.update(ctx);
        if let Some(path) = self.fd.take_picked() {
            app.save_project_to_path(&path);
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct PluginBrowserDialog {
    closed: bool,
    search_text: String,
    selected_category: String,
    selected_plugin: Option<usize>,
    available_categories: Vec<String>,
}

impl PluginBrowserDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            search_text: String::new(),
            selected_category: "All".to_string(),
            selected_plugin: None,
            available_categories: vec![
                "All".to_string(),
                "Instruments".to_string(),
                "Effects".to_string(),
                "Dynamics".to_string(),
                "EQ".to_string(),
                "Reverb".to_string(),
                "Delay".to_string(),
                "Modulation".to_string(),
                "Distortion".to_string(),
                "Utility".to_string(),
            ],
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Plugin Browser")
            .open(&mut open)
            .resizable(true)
            .default_size(egui::vec2(420.0, 220.0))
            .show(ctx, |ui| {
                // Header controls
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.text_edit_singleline(&mut self.search_text);

                    ui.separator();

                    ui.label("Category:");
                    egui::ComboBox::from_id_salt("plugin_category")
                        .selected_text(&self.selected_category)
                        .show_ui(ui, |ui| {
                            for category in &self.available_categories {
                                ui.selectable_value(
                                    &mut self.selected_category,
                                    category.clone(),
                                    category,
                                );
                            }
                        });
                });

                ui.separator();

                // Plugin list
                egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(220.0)
                .show(ui, |ui| {
                    for (idx, plugin) in app.available_plugins.iter().enumerate() {
                        // search filter
                        if !self.search_text.is_empty() {
                            let q = self.search_text.to_lowercase();
                            if !plugin.name.to_lowercase().contains(&q)
                                && !plugin.uri.to_lowercase().contains(&q)
                            {
                                continue;
                            }
                        }
                        // category filter
                        if self.selected_category != "All" {
                            let cats = categorize_unified_plugin(plugin);
                            if !cats.contains(&self.selected_category) {
                                continue;
                            }
                        }

                        let selected = self.selected_plugin == Some(idx);

                        // Show backend badge
                        let backend_badge = if plugin.uri.starts_with("file://") { "[CLAP]" } else { "[LV2]" };

                        // Show category hint in “All”
                        let display_name = if self.selected_category == "All" {
                            let cats = categorize_unified_plugin(plugin);
                            let main_cat = cats.iter().find(|c| *c != "All").map(|c| c.as_str()).unwrap_or("Unknown");
                            format!("{} {} [{}]", backend_badge, plugin.name, main_cat)
                        } else {
                            format!("{} {}", backend_badge, plugin.name)
                        };

                        let resp = ui.selectable_label(selected, display_name);
                        if resp.double_clicked() {
                            let backend = if plugin.uri.starts_with("file://") {
                                BackendKind::Clap
                            } else {
                                BackendKind::Lv2
                            };
                            let track_id = app.selected_track_for_plugin.take().unwrap_or(app.selected_track);
                            let _ = app.command_tx.send(AudioCommand::AddPluginUnified {
                                track_id,
                                backend,
                                uri: plugin.uri.clone(),
                                display_name: plugin.name.clone(),
                            });
                            self.closed = true;
                        } else if resp.clicked() {
                            self.selected_plugin = Some(idx);
                        }
                    }
                });

                ui.separator();

                // Plugin info
                if let Some(idx) = self.selected_plugin {
                    if let Some(plugin) = app.available_plugins.get(idx) {
                        ui.heading(&plugin.name);
                        ui.separator();
                        ui.label(format!("Backend: {}", if plugin.uri.starts_with("file://") { "CLAP" } else { "LV2" }));
                        ui.label(format!("Type: {}", if plugin.is_instrument { "Instrument" } else { "Effect" }));
                        ui.label(format!("Audio I/O: {} inputs / {} outputs", plugin.audio_inputs, plugin.audio_outputs));
                        ui.label(format!("MIDI: {}", if plugin.has_midi { "Yes" } else { "No" }));
                        ui.separator();
                        ui.label("Parameters: shown after loading the plugin.");
                    }
                } else {
                    ui.label("Select a plugin to see details.");
                }

                ui.separator();

                // Footer
                ui.horizontal(|ui| {
                    let can_add = self.selected_plugin.is_some();
                    if ui.add_enabled(can_add, egui::Button::new("Add to Track")).clicked() {
                        if let Some(idx) = self.selected_plugin {
                            if let Some(plugin) = app.available_plugins.get(idx) {
                                // Check track type vs plugin kind
                                let kind = crate::plugin::classify_plugin_uri(&plugin.uri).unwrap_or(crate::plugin::PluginKind::Unknown);
                                let track_id = app.selected_track_for_plugin.take().unwrap_or(app.selected_track);

                                // Peek current track type
                                let is_midi = {
                                    let state = app.state.lock().unwrap();
                                    state.tracks.get(track_id).map(|t| t.is_midi).unwrap_or(false)
                                };

                                if is_midi && matches!(kind, crate::plugin::PluginKind::Effect) {
                                    app.dialogs.show_message("You are adding an effect plugin to a MIDI track. It will not output audio unless the track is fed with audio. Consider adding it to an audio track or a bus.");
                                }

                                // Proceed with adding anyway (can choose)
                                let backend = if plugin.uri.starts_with("file://") {
                                BackendKind::Clap
                            } else {
                                BackendKind::Lv2
                            };
                            let track_id = app.selected_track_for_plugin.take().unwrap_or(app.selected_track);
                            let _ = app.command_tx.send(AudioCommand::AddPluginUnified {
                                track_id,
                                backend,
                                uri: plugin.uri.clone(),
                                display_name: plugin.name.clone(),
                            });
                            self.closed = true;
                            }
                        }
                    }

                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

simple_dialog!(
    AudioSetupDialog,
    "Audio Setup",
    |ui: &mut egui::Ui, _app: &mut super::app::YadawApp, closed: &mut bool| {
        ui.label("Audio configuration would be shown here");
        ui.label("(Not implemented yet)");
        ui.separator();
        if ui.button("Close").clicked() {
            *closed = true;
        }
    }
);

pub struct TransposeContent {
    semitones: i32,
}

impl TransposeContent {
    pub fn new() -> Self {
        Self { semitones: 0 }
    }
}

impl Dialog for TransposeContent {
    fn title(&self) -> &str {
        "Transpose"
    }

    fn draw_content(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) -> bool {
        ui.horizontal(|ui| {
            ui.label("Semitones:");
            ui.add(
                egui::DragValue::new(&mut self.semitones)
                    .speed(1)
                    .range(-24..=24),
            );
        });

        ui.separator();

        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                app.transpose_selected_notes(self.semitones);
                return true;
            }

            if ui.button("Cancel").clicked() {
                return true;
            } else {
                false
            }
        });

        false
    }

    fn is_closed(&self) -> bool {
        false
    }
}

pub type TransposeDialog = DialogWrapper<TransposeContent>;

pub struct HumanizeDialog {
    closed: bool,
    amount: f32,
    timing_variation: f32,
    velocity_variation: f32,
}

impl HumanizeDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            amount: 0.1,
            timing_variation: 0.05,
            velocity_variation: 0.1,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Humanize")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Overall Amount:");
                    ui.add(
                        egui::Slider::new(&mut self.amount, 0.0..=1.0)
                            .suffix("%")
                            .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Timing Variation:");
                    ui.add(
                        egui::Slider::new(&mut self.timing_variation, 0.0..=0.2).suffix(" beats"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Velocity Variation:");
                    ui.add(
                        egui::Slider::new(&mut self.velocity_variation, 0.0..=0.5)
                            .suffix("%")
                            .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
                    );
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        app.humanize_selected_notes(self.amount);
                        self.closed = true;
                    }

                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct TimeStretchDialog {
    closed: bool,
    stretch_factor: f32,
    preserve_pitch: bool,
}

impl TimeStretchDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            stretch_factor: 1.0,
            preserve_pitch: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Time Stretch")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Stretch Factor:");
                    ui.add(
                        egui::Slider::new(&mut self.stretch_factor, 0.5..=2.0)
                            .suffix("x")
                            .logarithmic(true),
                    );
                });

                ui.checkbox(&mut self.preserve_pitch, "Preserve Pitch");

                ui.separator();

                ui.label(format!("New length: {:.1}%", self.stretch_factor * 100.0));

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        // Apply time stretch
                        self.closed = true;
                    }

                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct ProjectSettingsDialog {
    closed: bool,
    bpm: f32,
    time_signature: (u32, u32),
    sample_rate: f32,
    initialized: bool,
}

impl ProjectSettingsDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            bpm: 120.0,
            time_signature: (4, 4),
            sample_rate: 44100.0,
            initialized: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        // Load current settings
        if !self.initialized {
            self.bpm = app.audio_state.bpm.load();
            self.sample_rate = app.audio_state.sample_rate.load();
            self.initialized = true;
        }

        egui::Window::new("Project Settings")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Project Settings");

                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("BPM:");
                    ui.add(
                        egui::DragValue::new(&mut self.bpm)
                            .speed(0.5)
                            .range(20.0..=999.0),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Time Signature:");
                    ui.add(
                        egui::DragValue::new(&mut self.time_signature.0)
                            .speed(1)
                            .range(1..=32),
                    );
                    ui.label("/");
                    egui::ComboBox::from_id_salt("time_sig_denom")
                        .selected_text(format!("{}", self.time_signature.1))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.time_signature.1, 2, "2");
                            ui.selectable_value(&mut self.time_signature.1, 4, "4");
                            ui.selectable_value(&mut self.time_signature.1, 8, "8");
                            ui.selectable_value(&mut self.time_signature.1, 16, "16");
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("Sample Rate:");
                    ui.label(format!("{} Hz", self.sample_rate));
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        let _ = app.command_tx.send(AudioCommand::SetBPM(self.bpm));
                        self.closed = true;
                    }

                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct ThemeEditorDialog {
    closed: bool,
    custom_theme: theme::CustomTheme,
    preview_enabled: bool,
}

impl ThemeEditorDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            custom_theme: theme::ThemeManager::create_dark_blue_theme(),
            preview_enabled: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Theme Editor")
            .open(&mut open)
            .default_size(egui::vec2(400.0, 500.0))
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Theme Name:");
                    ui.text_edit_singleline(&mut self.custom_theme.name);
                });

                ui.separator();

                fn show_color_picker(
                    ui: &mut egui::Ui,
                    label: &str,
                    color: &mut theme::Color32Ser,
                ) {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", label));

                        let mut rgba = [color.r, color.g, color.b, color.a];
                        if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                            color.r = rgba[0];
                            color.g = rgba[1];
                            color.b = rgba[2];
                            color.a = rgba[3];
                        }
                    });
                }

                // Use the helper function for each color
                show_color_picker(ui, "Background", &mut self.custom_theme.background);
                show_color_picker(ui, "Foreground", &mut self.custom_theme.foreground);
                show_color_picker(ui, "Primary", &mut self.custom_theme.primary);
                show_color_picker(ui, "Secondary", &mut self.custom_theme.secondary);
                show_color_picker(ui, "Success", &mut self.custom_theme.success);
                show_color_picker(ui, "Warning", &mut self.custom_theme.warning);
                show_color_picker(ui, "Error", &mut self.custom_theme.error);

                ui.separator();

                ui.checkbox(&mut self.preview_enabled, "Preview Theme");

                if self.preview_enabled {
                    app.theme_manager
                        .set_theme(theme::Theme::Custom(self.custom_theme.clone()));
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Save Theme").clicked() {
                        app.theme_manager
                            .add_custom_theme(self.custom_theme.clone());
                        self.closed = true;
                    }

                    if ui.button("Cancel").clicked() {
                        if self.preview_enabled {
                            app.theme_manager.set_theme(theme::Theme::Dark);
                        }
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    fn color_picker(&mut self, ui: &mut egui::Ui, label: &str, color: &mut theme::Color32Ser) {
        ui.horizontal(|ui| {
            ui.label(format!("{}:", label));

            let mut rgba = [color.r, color.g, color.b, color.a];
            if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                color.r = rgba[0];
                color.g = rgba[1];
                color.b = rgba[2];
                color.a = rgba[3];
            }
        });
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct ExportDialog {
    closed: bool,
    format: ExportFormat,
    quality: ExportQuality,
    normalize: bool,
    dither: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum ExportFormat {
    Wav,
    Mp3,
    Flac,
    Ogg,
}

#[derive(Clone, Copy, PartialEq)]
enum ExportQuality {
    Low,
    Medium,
    High,
    Lossless,
}

impl ExportDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            format: ExportFormat::Wav,
            quality: ExportQuality::High,
            normalize: true,
            dither: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Export Audio")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Format:");
                    ui.radio_value(&mut self.format, ExportFormat::Wav, "WAV");
                    ui.radio_value(&mut self.format, ExportFormat::Mp3, "MP3");
                    ui.radio_value(&mut self.format, ExportFormat::Flac, "FLAC");
                    ui.radio_value(&mut self.format, ExportFormat::Ogg, "OGG");
                });

                ui.horizontal(|ui| {
                    ui.label("Quality:");
                    ui.radio_value(&mut self.quality, ExportQuality::Low, "Low");
                    ui.radio_value(&mut self.quality, ExportQuality::Medium, "Medium");
                    ui.radio_value(&mut self.quality, ExportQuality::High, "High");
                    ui.radio_value(&mut self.quality, ExportQuality::Lossless, "Lossless");
                });

                ui.separator();

                ui.checkbox(&mut self.normalize, "Normalize");
                ui.checkbox(&mut self.dither, "Apply Dither");

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Export").clicked() {
                        // Perform export
                        self.closed = true;
                    }

                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct PluginManagerDialog {
    closed: bool,
    scan_paths: Vec<String>,
    new_path: String,
    browse_fd: FileDialog,
    browse_opened: bool,
}
impl PluginManagerDialog {
    pub fn new() -> Self {
        let browse_fd = FileDialog::new().title("Select Plugin Directory");
        Self {
            closed: false,
            scan_paths: vec![
                "~/.lv2".to_string(),
                "/usr/lib/lv2".to_string(),
                "/usr/local/lib/lv2".to_string(),
                "~/.clap".to_string(),
                "/usr/lib/clap".to_string(),
                "/usr/local/lib/clap".to_string(),
            ],
            new_path: String::new(),
            browse_fd,
            browse_opened: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut YadawApp) {
        let mut open = true;

        egui::Window::new("Plugin Manager")
            .open(&mut open)
            .default_size(egui::vec2(600.0, 500.0))
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Plugin Scan Paths");

                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        let mut to_remove = None;

                        for (idx, path) in self.scan_paths.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(path);
                                if ui.small_button("Remove").clicked() {
                                    to_remove = Some(idx);
                                }
                            });
                        }

                        if let Some(idx) = to_remove {
                            self.scan_paths.remove(idx);
                        }
                    });

                ui.separator();

                // Add path section
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_path);
                    if ui.button("Add Path").clicked() && !self.new_path.is_empty() {
                        self.scan_paths.push(self.new_path.clone());
                        self.new_path.clear();
                    }
                    if ui.button("Browse...").clicked() {
                        self.browse_fd.pick_directory();
                        self.browse_opened = true;
                    }
                });

                ui.separator();

                if ui.button("Scan for Plugins").clicked() {
                    // Trigger plugin scan
                }

                ui.separator();

                if ui.button("Close").clicked() {
                    self.closed = true;
                }
            });

        if self.browse_opened {
            self.browse_fd.update(ctx);
            if let Some(path) = self.browse_fd.take_picked() {
                self.scan_paths.push(path.to_string_lossy().to_string());
                self.browse_opened = false;
            }
        }

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct ImportAudioDialog {
    fd: FileDialog,
    opened: bool,
}

impl ImportAudioDialog {
    pub fn new() -> Self {
        let fd = FileDialog::new()
            .title("Import Audio")
            .add_file_filter_extensions(
                "Audio Files",
                (&["wav", "mp3", "flac", "ogg", "m4a", "aac"]).to_vec(),
            )
            .add_file_filter_extensions("All Files", (&["*"]).to_vec());
        Self { fd, opened: false }
    }

    pub fn open(&mut self) {
        self.fd.pick_multiple(); // precise 0.11 API for multi-select
        self.opened = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        if !self.opened {
            return;
        }
        self.fd.update(ctx);
        if let Some(paths) = self.fd.take_picked_multiple() {
            app.push_undo();
            let bpm = app.audio_state.bpm.load();
            for path in paths {
                crate::audio_import::import_audio_file(&path, bpm)
                    .map_err(|e| common::audio_import_failed(&path, e))
                    .map(|clip| {
                        let mut state = app.state.lock().unwrap();
                        if let Some(track) = state.tracks.get_mut(app.selected_track) {
                            if !track.is_midi {
                                track.audio_clips.push(clip);
                                state.ensure_ids();
                            } else {
                                app.dialogs
                                    .show_warning("Cannot import audio to MIDI track");
                            }
                        }
                    })
                    .notify_user(&mut app.dialogs);
            }
            self.opened = false;
        }
    }

    pub fn is_open(&self) -> bool {
        self.opened
    }
}

pub struct LayoutManagerDialog {
    closed: bool,
    layouts: Vec<String>,
    selected_layout: Option<usize>,
}

impl LayoutManagerDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            layouts: vec![
                "Default".to_string(),
                "Mixing".to_string(),
                "Recording".to_string(),
                "Editing".to_string(),
            ],
            selected_layout: Some(0),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Layout Manager")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Window Layouts");

                for (idx, layout) in self.layouts.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected_layout == Some(idx), layout)
                        .clicked()
                    {
                        self.selected_layout = Some(idx);
                    }
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Load").clicked() {
                        if let Some(idx) = self.selected_layout {
                            // Load the selected layout
                            match idx {
                                0 => app.reset_layout(), // Default
                                1 => {
                                    // Mixing layout - show mixer
                                    app.mixer_ui.toggle_visibility();
                                }
                                2 => {
                                    // Recording layout
                                }
                                3 => {
                                    // Editing layout
                                }
                                _ => {}
                            }
                        }
                        self.closed = true;
                    }

                    if ui.button("Save Current...").clicked() {
                        // Save current layout
                    }

                    if ui.button("Delete").clicked() {
                        if let Some(idx) = self.selected_layout {
                            if idx >= 4 {
                                // Don't delete built-in layouts
                                self.layouts.remove(idx);
                                self.selected_layout = None;
                            }
                        }
                    }

                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct ProgressBar {
    message: String,
    progress: f32,
    closed: bool,
}

impl ProgressBar {
    pub fn new(message: String) -> Self {
        Self {
            message,
            progress: 0.0,
            closed: false,
        }
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
        if self.progress >= 1.0 {
            self.closed = true;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        egui::Window::new("Progress")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(&self.message);
                ui.add(egui::ProgressBar::new(self.progress));

                if self.progress >= 1.0 {
                    if ui.button("OK").clicked() {
                        self.closed = true;
                    }
                }
            });
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Default for DialogManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TrackGroupingDialog {
    closed: bool,
    new_group_name: String,
    selected_tracks: Vec<usize>,
    selected_group: Option<usize>,
}

impl TrackGroupingDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            new_group_name: String::from("New Group"),
            selected_tracks: Vec::new(),
            selected_group: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Track Grouping (not yet implemented fully)")
            .open(&mut open)
            .resizable(true)
            .default_size(egui::vec2(400.0, 500.0))
            .show(ctx, |ui| {
                ui.heading("Track Groups");

                // List existing groups
                ui.group(|ui| {
                    ui.label("Existing Groups:");

                    let groups = app.track_manager.get_groups().to_vec();
                    for (idx, group) in groups.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.selected_group == Some(idx), &group.name)
                                .clicked()
                            {
                                self.selected_group = Some(idx);
                            }

                            ui.label(format!("({} tracks)", group.track_ids.len()));

                            if ui.small_button("Delete").clicked() {
                                // TODO: Remove group
                            }
                        });
                    }
                });

                ui.separator();

                // Create new group
                ui.group(|ui| {
                    ui.label("Create New Group:");

                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.new_group_name);
                    });

                    ui.label("Select tracks to group:");

                    let state = app.state.lock().unwrap();
                    for (idx, track) in state.tracks.iter().enumerate() {
                        let mut is_selected = self.selected_tracks.contains(&idx);
                        if ui.checkbox(&mut is_selected, &track.name).changed() {
                            if is_selected {
                                self.selected_tracks.push(idx);
                            } else {
                                self.selected_tracks.retain(|&i| i != idx);
                            }
                        }
                    }
                    drop(state);

                    if ui.button("Create Group").clicked() {
                        if !self.selected_tracks.is_empty() {
                            app.track_manager.create_group(
                                self.new_group_name.clone(),
                                self.selected_tracks.clone(),
                            );
                            self.selected_tracks.clear();
                            self.new_group_name = String::from("New Group");
                        }
                    }
                });

                ui.separator();

                if ui.button("Close").clicked() {
                    self.closed = true;
                }
            });

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct TrackRenameDialog {
    closed: bool,
    track_idx: usize,
    name: String,
}

impl TrackRenameDialog {
    pub fn new(track_idx: usize, current: String) -> Self {
        Self {
            closed: false,
            track_idx,
            name: current,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;
        egui::Window::new("Rename Track")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.name);
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        // Apply
                        {
                            let mut state = app.state.lock().unwrap();
                            if let Some(t) = state.tracks.get_mut(self.track_idx) {
                                t.name = self.name.trim().to_string();
                            }
                        }
                        let _ = app.command_tx.send(AudioCommand::UpdateTracks);
                        self.closed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.closed = true;
                    }
                });
            });
        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}
