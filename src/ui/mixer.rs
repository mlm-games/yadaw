use super::*;
use crate::audio_utils::{format_pan, linear_to_db};
use crate::level_meter::LevelMeter;
use crate::state::AudioCommand;

pub struct MixerWindow {
    pub visible: bool,
    size: egui::Vec2,
    position: Option<egui::Pos2>,

    // Mixer state
    channel_strips: Vec<ChannelStrip>,
    master_strip: MasterStrip,

    // View options
    show_eq: bool,
    show_sends: bool,
    show_inserts: bool,
    narrow_strips: bool,

    // Sizing
    strip_width: f32,
    min_strip_width: f32,
    max_strip_width: f32,
}

struct ChannelStrip {
    meter: LevelMeter,
    eq_enabled: bool,
    sends: Vec<SendControl>,
}

struct MasterStrip {
    meter: LevelMeter,
    limiter_enabled: bool,
}

struct SendControl {
    destination: String,
    level: f32,
    pre_fader: bool,
}

impl MixerWindow {
    pub fn new() -> Self {
        Self {
            visible: false,
            size: egui::vec2(800.0, 600.0),
            position: None,

            channel_strips: Vec::new(),
            master_strip: MasterStrip {
                meter: LevelMeter::default(),
                limiter_enabled: false,
            },

            show_eq: true,
            show_sends: true,
            show_inserts: true,
            narrow_strips: false,

            strip_width: 100.0,
            min_strip_width: 60.0,
            max_strip_width: 150.0,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut super::app::YadawApp) {
        let mut visible = self.visible;

        let mut window = egui::Window::new("Mixer")
            .open(&mut visible)
            .default_size(self.size)
            .resizable(true)
            .collapsible(true);

        if let Some(pos) = self.position {
            window = window.default_pos(pos);
        }

        window.show(ctx, |ui| {
            // Store window position for next frame
            let pos = ui.cursor().left_top();
            {
                self.position = Some(pos);
            }

            // Mixer toolbar
            self.draw_toolbar(ui);

            ui.separator();

            // Mixer channels
            egui::ScrollArea::horizontal()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_mixer_channels(ui, app);
                });
        });

        self.visible = visible;
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("View:");

            ui.toggle_value(&mut self.show_eq, "EQ")
                .on_hover_text("Show/Hide EQ Section");

            ui.toggle_value(&mut self.show_sends, "Sends")
                .on_hover_text("Show/Hide Sends");

            ui.toggle_value(&mut self.show_inserts, "Inserts")
                .on_hover_text("Show/Hide Insert Effects");

            ui.separator();

            ui.toggle_value(&mut self.narrow_strips, "Narrow")
                .on_hover_text("Use Narrow Channel Strips");

            if self.narrow_strips {
                self.strip_width = self.min_strip_width;
            } else {
                ui.add(
                    egui::Slider::new(
                        &mut self.strip_width,
                        self.min_strip_width..=self.max_strip_width,
                    )
                    .text("Width")
                    .show_value(false),
                );
            }

            ui.separator();

            if ui.button("Reset All").clicked() {
                // Reset all mixer settings
            }
        });
    }

    fn draw_mixer_channels(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.horizontal(|ui| {
            let binding = app.state.clone();
            let state = binding.lock().unwrap();

            // Ensure we have enough channel strips
            while self.channel_strips.len() < state.tracks.len() {
                self.channel_strips.push(ChannelStrip {
                    meter: LevelMeter::default(),
                    eq_enabled: false,
                    sends: Vec::new(),
                });
            }

            // Draw each track's channel strip
            for (idx, track) in state.tracks.iter().enumerate() {
                self.draw_channel_strip(ui, track, idx, app);
                ui.separator();
            }

            // Master channel
            self.draw_master_strip(ui, app);
        });
    }

    fn draw_channel_strip(
        &mut self,
        ui: &mut egui::Ui,
        track: &crate::state::Track,
        idx: usize,
        app: &mut super::app::YadawApp,
    ) {
        ui.allocate_ui(egui::vec2(self.strip_width, ui.available_height()), |ui| {
            ui.vertical(|ui| {
                // Channel name
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(&track.name);
                    ui.label(format!("#{}", idx + 1));
                });

                // Insert effects section
                if self.show_inserts {
                    ui.group(|ui| {
                        ui.set_min_height(80.0);
                        ui.label("Inserts");

                        for plugin in &track.plugin_chain {
                            let label = if plugin.bypass {
                                format!("⊘ {}", plugin.name)
                            } else {
                                plugin.name.clone()
                            };

                            if ui.small_button(&label).clicked() {
                                // Open plugin editor
                            }
                        }

                        if ui.small_button("+ Add").clicked() {
                            app.show_plugin_browser_for_track(idx);
                        }
                    });
                }

                // EQ section
                if self.show_eq {
                    ui.group(|ui| {
                        ui.set_min_height(100.0);
                        ui.label("EQ");

                        // Simple 3-band EQ visualization
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label("L");
                                let mut low = 0.0f32;
                                ui.add(
                                    egui::Slider::new(&mut low, -12.0..=12.0)
                                        .vertical()
                                        .show_value(false),
                                );
                            });

                            ui.vertical(|ui| {
                                ui.label("M");
                                let mut mid = 0.0f32;
                                ui.add(
                                    egui::Slider::new(&mut mid, -12.0..=12.0)
                                        .vertical()
                                        .show_value(false),
                                );
                            });

                            ui.vertical(|ui| {
                                ui.label("H");
                                let mut high = 0.0f32;
                                ui.add(
                                    egui::Slider::new(&mut high, -12.0..=12.0)
                                        .vertical()
                                        .show_value(false),
                                );
                            });
                        });

                        ui.checkbox(&mut self.channel_strips[idx].eq_enabled, "Enable");
                    });
                }

                // Sends section
                if self.show_sends {
                    ui.group(|ui| {
                        ui.set_min_height(60.0);
                        ui.label("Sends");

                        // Add some default sends
                        if self.channel_strips[idx].sends.is_empty() {
                            self.channel_strips[idx].sends.push(SendControl {
                                destination: "Reverb".to_string(),
                                level: 0.0,
                                pre_fader: false,
                            });
                            self.channel_strips[idx].sends.push(SendControl {
                                destination: "Delay".to_string(),
                                level: 0.0,
                                pre_fader: false,
                            });
                        }

                        for send in &mut self.channel_strips[idx].sends {
                            ui.horizontal(|ui| {
                                ui.label(&send.destination);
                                ui.add(
                                    egui::Slider::new(&mut send.level, 0.0..=1.0).show_value(false),
                                );
                            });
                        }
                    });
                }

                // Level meter
                ui.group(|ui| {
                    ui.set_min_height(150.0);
                    self.channel_strips[idx].meter.ui(ui, true);
                });

                // Fader and pan
                ui.group(|ui| {
                    // Volume fader
                    let mut volume = track.volume;
                    ui.vertical_centered(|ui| {
                        ui.add(
                            egui::Slider::new(&mut volume, 0.0..=1.2)
                                .vertical()
                                .show_value(false),
                        );
                        ui.label(format!("{:.1}", linear_to_db(volume)));
                    });

                    if (volume - track.volume).abs() > 0.001 {
                        let _ = app
                            .command_tx
                            .send(AudioCommand::SetTrackVolume(idx, volume));
                    }

                    ui.separator();

                    // Pan knob
                    let mut pan = track.pan;
                    ui.horizontal(|ui| {
                        ui.label("Pan:");
                        ui.add(egui::Slider::new(&mut pan, -1.0..=1.0).show_value(false));
                    });

                    if (pan - track.pan).abs() > 0.001 {
                        let _ = app.command_tx.send(AudioCommand::SetTrackPan(idx, pan));
                    }

                    let pan_text = format_pan(pan);
                    ui.label(pan_text);
                });

                // Channel controls
                ui.horizontal(|ui| {
                    // Mute
                    let mut muted = track.muted;
                    if ui
                        .selectable_label(muted, if muted { "M" } else { "m" })
                        .on_hover_text("Mute")
                        .clicked()
                    {
                        let _ = app.command_tx.send(AudioCommand::MuteTrack(idx, !muted));
                    }

                    // Solo
                    let mut solo = track.solo;
                    if ui
                        .selectable_label(solo, if solo { "S" } else { "s" })
                        .on_hover_text("Solo")
                        .clicked()
                    {
                        let _ = app.command_tx.send(AudioCommand::SoloTrack(idx, !solo));
                    }

                    // Record arm
                    if ui
                        .selectable_label(track.armed, if track.armed { "●" } else { "○" })
                        .on_hover_text("Record Arm")
                        .clicked()
                    {
                        // Handle record arm
                    }
                });
            });
        });
    }

    fn draw_master_strip(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
        ui.allocate_ui(
            egui::vec2(self.strip_width * 1.5, ui.available_height()),
            |ui| {
                ui.vertical(|ui| {
                    // Master channel header
                    ui.group(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.heading("Master");
                    });

                    // Master effects
                    if self.show_inserts {
                        ui.group(|ui| {
                            ui.set_min_height(80.0);
                            ui.label("Master Effects");

                            ui.checkbox(&mut self.master_strip.limiter_enabled, "Limiter");

                            if ui.small_button("+ Add").clicked() {
                                // Add master effect
                            }
                        });
                    }

                    // Master meter
                    ui.group(|ui| {
                        ui.set_min_height(200.0);
                        self.master_strip.meter.ui(ui, true);
                    });

                    // Master fader
                    ui.group(|ui| {
                        let mut master_volume = app.audio_state.master_volume.load();

                        ui.vertical_centered(|ui| {
                            ui.add(
                                egui::Slider::new(&mut master_volume, 0.0..=1.2)
                                    .vertical()
                                    .show_value(false),
                            );
                            ui.label(format!(
                                "{:.1} dB",
                                20.0 * master_volume.max(0.0001).log10()
                            ));
                        });

                        if (master_volume - app.audio_state.master_volume.load()).abs() > 0.001 {
                            app.audio_state.master_volume.store(master_volume);
                        }
                    });
                });
            },
        );
    }
}

impl Default for MixerWindow {
    fn default() -> Self {
        Self::new()
    }
}
