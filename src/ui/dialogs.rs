use super::*;
use crate::edit_actions::EditProcessor;
use crate::state::AudioCommand;

pub struct DialogManager {
    // File dialogs
    pub open_dialog: Option<OpenDialog>,
    pub save_dialog: Option<SaveDialog>,

    // Audio dialogs
    pub audio_setup: Option<AudioSetupDialog>,
    pub plugin_browser: Option<PluginBrowserDialog>,
    pub plugin_manager: Option<PluginManagerDialog>,

    // Edit dialogs
    pub quantize_dialog: Option<QuantizeDialog>,
    pub transpose_dialog: Option<TransposeDialog>,
    pub humanize_dialog: Option<HumanizeDialog>,
    pub time_stretch_dialog: Option<TimeStretchDialog>,

    // Project dialogs
    pub project_settings: Option<ProjectSettingsDialog>,
    pub export_dialog: Option<ExportDialog>,

    // UI dialogs
    pub theme_editor: Option<ThemeEditorDialog>,
    pub layout_manager: Option<LayoutManagerDialog>,

    // Utility
    pub message_box: Option<MessageBox>,
    pub progress_bar: Option<ProgressBar>,
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
        }
    }

    pub fn show_all(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        // Show active dialogs
        if let Some(dialog) = &mut self.open_dialog {
            dialog.show(ctx, app);
            if dialog.is_closed() {
                self.open_dialog = None;
            }
        }

        if let Some(dialog) = &mut self.save_dialog {
            dialog.show(ctx, app);
            if dialog.is_closed() {
                self.save_dialog = None;
            }
        }

        if let Some(dialog) = &mut self.audio_setup {
            dialog.show(ctx, app);
            if dialog.is_closed() {
                self.audio_setup = None;
            }
        }

        if let Some(dialog) = &mut self.plugin_browser {
            dialog.show(ctx, app);
            if dialog.is_closed() {
                self.plugin_browser = None;
            }
        }

        if let Some(dialog) = &mut self.quantize_dialog {
            dialog.show(ctx, app);
            if dialog.is_closed() {
                self.quantize_dialog = None;
            }
        }

        if let Some(dialog) = &mut self.message_box {
            dialog.show(ctx);
            if dialog.is_closed() {
                self.message_box = None;
            }
        }

        // ... show other dialogs
    }

    pub fn show_message(&mut self, message: &str) {
        self.message_box = Some(MessageBox::new(message.to_string()));
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

    pub fn show_quantize_dialog(&mut self) {
        self.quantize_dialog = Some(QuantizeDialog::new());
    }

    pub fn show_theme_editor(&mut self) {
        self.theme_editor = Some(ThemeEditorDialog::new());
    }
}

// Individual dialog implementations

pub struct MessageBox {
    message: String,
    closed: bool,
}

impl MessageBox {
    pub fn new(message: String) -> Self {
        Self {
            message,
            closed: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        egui::Window::new("Message")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(&self.message);
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        self.closed = true;
                    }
                });
            });
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct OpenDialog {
    closed: bool,
}

impl OpenDialog {
    pub fn new() -> Self {
        Self { closed: false }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        // Use native file dialog
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("YADAW Project", &["yadaw"])
            .add_filter("All Files", &["*"])
            .pick_file()
        {
            app.load_project_from_path(&path);
        }
        self.closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

pub struct SaveDialog {
    closed: bool,
}

impl SaveDialog {
    pub fn new() -> Self {
        Self { closed: false }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        // Use native file dialog
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("YADAW Project", &["yadaw"])
            .set_file_name("untitled.yadaw")
            .save_file()
        {
            app.save_project_to_path(&path);
        }
        self.closed = true;
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
}

impl PluginBrowserDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            search_text: String::new(),
            selected_category: "All".to_string(),
            selected_plugin: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Plugin Browser")
            .open(&mut open)
            .default_size(egui::vec2(600.0, 400.0))
            .resizable(true)
            .show(ctx, |ui| {
                // Search bar
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.text_edit_singleline(&mut self.search_text);

                    ui.separator();

                    ui.label("Category:");
                    egui::ComboBox::from_id_source("plugin_category")
                        .selected_text(&self.selected_category)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.selected_category,
                                "All".to_string(),
                                "All",
                            );
                            ui.selectable_value(
                                &mut self.selected_category,
                                "Instruments".to_string(),
                                "Instruments",
                            );
                            ui.selectable_value(
                                &mut self.selected_category,
                                "Effects".to_string(),
                                "Effects",
                            );
                            ui.selectable_value(
                                &mut self.selected_category,
                                "Dynamics".to_string(),
                                "Dynamics",
                            );
                            ui.selectable_value(
                                &mut self.selected_category,
                                "EQ".to_string(),
                                "EQ",
                            );
                            ui.selectable_value(
                                &mut self.selected_category,
                                "Reverb".to_string(),
                                "Reverb",
                            );
                            ui.selectable_value(
                                &mut self.selected_category,
                                "Delay".to_string(),
                                "Delay",
                            );
                        });
                });

                ui.separator();

                // Plugin list
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (idx, plugin) in app.available_plugins.iter().enumerate() {
                            // Filter by search
                            if !self.search_text.is_empty() {
                                if !plugin
                                    .name
                                    .to_lowercase()
                                    .contains(&self.search_text.to_lowercase())
                                {
                                    continue;
                                }
                            }

                            // Filter by category
                            if self.selected_category != "All" {
                                // Add category filtering logic
                            }

                            let selected = self.selected_plugin == Some(idx);

                            if ui.selectable_label(selected, &plugin.name).clicked() {
                                self.selected_plugin = Some(idx);
                            }
                        }
                    });

                ui.separator();

                // Plugin info
                if let Some(idx) = self.selected_plugin {
                    if let Some(plugin) = app.available_plugins.get(idx) {
                        ui.label(format!("Name: {}", plugin.name));
                        ui.label(format!(
                            "Type: {}",
                            if plugin.is_instrument {
                                "Instrument"
                            } else {
                                "Effect"
                            }
                        ));
                        ui.label(format!("Inputs: {}", plugin.audio_inputs));
                        ui.label(format!("Outputs: {}", plugin.audio_outputs));
                    }
                }

                ui.separator();

                // Buttons
                ui.horizontal(|ui| {
                    if ui.button("Add to Track").clicked() {
                        if let Some(idx) = self.selected_plugin {
                            if let Some(plugin) = app.available_plugins.get(idx) {
                                let _ = app.command_tx.send(AudioCommand::AddPlugin(
                                    app.selected_track,
                                    plugin.uri.clone(),
                                ));
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

pub struct QuantizeDialog {
    closed: bool,
    strength: f32,
    grid_size: f32,
    swing: f32,
}

impl QuantizeDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            strength: 1.0,
            grid_size: 0.25,
            swing: 0.0,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Quantize")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
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
                    egui::ComboBox::from_id_source("quantize_grid")
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

                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        app.quantize_selected_notes_with_params(
                            self.strength,
                            self.grid_size,
                            self.swing,
                        );
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

// Add more dialog implementations as needed...

pub struct AudioSetupDialog {
    closed: bool,
}

impl AudioSetupDialog {
    pub fn new() -> Self {
        Self { closed: false }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Audio Setup")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Audio configuration would be shown here");
                ui.label("(Not implemented in this example)");

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

pub struct TransposeDialog {
    closed: bool,
    semitones: i32,
}

impl TransposeDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            semitones: 0,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Transpose")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Semitones:");
                    ui.add(
                        egui::DragValue::new(&mut self.semitones)
                            .speed(1)
                            .clamp_range(-24..=24),
                    );
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        app.transpose_selected_notes(self.semitones);
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

// Others later...
