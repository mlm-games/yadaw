use super::*;
use crate::edit_actions::EditProcessor;
use crate::state::AudioCommand;
use crate::ui::theme;

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
            d.show(ctx);
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
    }

    pub fn show_project_settings(&mut self) {
        self.project_settings = Some(ProjectSettingsDialog::new());
    }
    pub fn show_track_grouping(&mut self) { /* TODO */
    }
    pub fn show_plugin_manager(&mut self) {
        self.plugin_manager = Some(PluginManagerDialog::new());
    }
    pub fn show_transpose_dialog(&mut self) {
        self.transpose_dialog = Some(TransposeDialog::new());
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
                                let track_id = app
                                    .selected_track_for_plugin
                                    .take()
                                    .unwrap_or(app.selected_track);
                                let _ = app
                                    .command_tx
                                    .send(AudioCommand::AddPlugin(track_id, plugin.uri.clone()));
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
                            .range(-24..=24),
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
}

impl ProjectSettingsDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            bpm: 120.0,
            time_signature: (4, 4),
            sample_rate: 44100.0,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut open = true;

        // Load current settings
        self.bpm = app.audio_state.bpm.load();
        self.sample_rate = app.audio_state.sample_rate.load();

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
                    egui::ComboBox::from_id_source("time_sig_denom")
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
                        app.audio_state.bpm.store(self.bpm);
                        if let Some(transport) = &mut app.transport_ui.transport {
                            transport.set_bpm(self.bpm);
                        }
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
}

impl PluginManagerDialog {
    pub fn new() -> Self {
        Self {
            closed: false,
            scan_paths: vec![
                "~/.lv2".to_string(),
                "/usr/lib/lv2".to_string(),
                "/usr/local/lib/lv2".to_string(),
            ],
            new_path: String::new(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, _app: &mut super::app::YadawApp) {
        let mut open = true;

        egui::Window::new("Plugin Manager")
            .open(&mut open)
            .default_size(egui::vec2(500.0, 400.0))
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

                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_path);
                    if ui.button("Add Path").clicked() && !self.new_path.is_empty() {
                        self.scan_paths.push(self.new_path.clone());
                        self.new_path.clear();
                    }
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.scan_paths.push(path.to_string_lossy().to_string());
                        }
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

        if !open {
            self.closed = true;
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
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
