use super::app::YadawApp;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TutorialStep {
    Welcome,
    AddTrack,
    RecordAudio,
    AddMidiNotes,
    UsePlugins,
    MixTracks,
    SaveProject,
    Completed,
}

pub struct InteractiveTutorial {
    active: bool,
    current_step: TutorialStep,
    highlight_rect: Option<egui::Rect>,
    completed_steps: Vec<TutorialStep>,
}

impl InteractiveTutorial {
    pub fn new() -> Self {
        Self {
            active: false,
            current_step: TutorialStep::Welcome,
            highlight_rect: None,
            completed_steps: Vec::new(),
        }
    }

    pub fn start(&mut self) {
        self.active = true;
        self.current_step = TutorialStep::Welcome;
        self.completed_steps.clear();
    }

    pub fn stop(&mut self) {
        self.active = false;
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut YadawApp) {
        if !self.active {
            return;
        }

        // Draw overlay for dimming
        if let Some(highlight) = self.highlight_rect {
            self.draw_highlight_overlay(ctx, highlight);
        }

        // Show tutorial window
        egui::Window::new("🎓 Tutorial")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
            .show(ctx, |ui| {
                self.draw_step_content(ui, app);
            });

        // Check for step completion
        self.check_step_completion(app);
    }

    fn draw_step_content(&mut self, ui: &mut egui::Ui, app: &mut YadawApp) {
        ui.heading(self.get_step_title());
        ui.separator();

        match self.current_step {
            TutorialStep::Welcome => {
                ui.label("Welcome to YADAW! This tutorial will guide you through the basics.");
                ui.label("");
                ui.label("You'll learn how to:");
                ui.label("• Add tracks");
                ui.label("• Record audio");
                ui.label("• Create MIDI patterns");
                ui.label("• Add effects");
                ui.label("• Mix your project");

                if ui.button("Let's Start! →").clicked() {
                    self.next_step();
                }
            }

            TutorialStep::AddTrack => {
                ui.label("Let's add your first track!");
                ui.label("");
                ui.label("👉 Click the '➕ Audio Track' button");
                ui.label("   in the Tracks panel on the left.");

                // Highlight the add track button area
                // (You'd calculate the actual rect from the UI)

                ui.separator();
                ui.label(format!(
                    "Tracks in project: {}",
                    app.state.lock().unwrap().tracks.len()
                ));
            }

            TutorialStep::RecordAudio => {
                ui.label("Great! Now let's record some audio.");
                ui.label("");
                ui.label("1. Click the '○' button on your audio track");
                ui.label("   to arm it for recording (it will turn red)");
                ui.label("");
                ui.label("2. Click the Record button ⏺ in the transport");
                ui.label("");
                ui.label("3. Click Play ▶ to start recording");

                if ui.button("Skip this step").clicked() {
                    self.next_step();
                }
            }

            TutorialStep::AddMidiNotes => {
                ui.label("Now let's add a MIDI track and create notes.");
                ui.label("");
                ui.label("1. Click '➕ MIDI Track' to add a MIDI track");
                ui.label("");
                ui.label("2. Select the MIDI track");
                ui.label("");
                ui.label("3. The Piano Roll will appear");
                ui.label("");
                ui.label("4. Click in the grid to add notes!");

                // Check if user has MIDI track with notes
                let has_midi_notes = app
                    .state
                    .lock()
                    .unwrap()
                    .tracks
                    .iter()
                    .any(|t| t.is_midi && !t.patterns.is_empty());

                if has_midi_notes {
                    ui.colored_label(egui::Color32::GREEN, "✓ MIDI notes detected!");
                    if ui.button("Continue →").clicked() {
                        self.next_step();
                    }
                }
            }

            TutorialStep::UsePlugins => {
                ui.label("Let's add some effects!");
                ui.label("");
                ui.label("1. In the Tracks panel, find your track");
                ui.label("");
                ui.label("2. Click the '➕' button next to 'Plugins'");
                ui.label("");
                ui.label("3. Choose a plugin from the browser");
                ui.label("");
                ui.label("4. Adjust the parameters to your liking");

                let has_plugins = app
                    .state
                    .lock()
                    .unwrap()
                    .tracks
                    .iter()
                    .any(|t| !t.plugin_chain.is_empty());

                if has_plugins {
                    ui.colored_label(egui::Color32::GREEN, "✓ Plugin added!");
                    if ui.button("Continue →").clicked() {
                        self.next_step();
                    }
                }
            }

            TutorialStep::MixTracks => {
                ui.label("Time to mix your tracks!");
                ui.label("");
                ui.label("• Adjust Volume sliders for each track");
                ui.label("• Use Pan to position tracks left/right");
                ui.label("• Press 'M' to open the Mixer window");
                ui.label("• Use Solo (S) to isolate tracks");
                ui.label("• Use Mute (M) to silence tracks");

                if ui.button("Continue →").clicked() {
                    self.next_step();
                }
            }

            TutorialStep::SaveProject => {
                ui.label("Finally, let's save your project!");
                ui.label("");
                ui.label("Press Ctrl+S (or Cmd+S on Mac)");
                ui.label("or use File → Save from the menu.");

                if app.project_path.is_some() {
                    ui.colored_label(egui::Color32::GREEN, "✓ Project saved!");
                    if ui.button("Complete Tutorial →").clicked() {
                        self.next_step();
                    }
                }
            }

            TutorialStep::Completed => {
                ui.colored_label(egui::Color32::GREEN, "🎉 Congratulations!");
                ui.label("");
                ui.label("You've completed the tutorial!");
                ui.label("");
                ui.label("Explore more features:");
                ui.label("• Automation lanes");
                ui.label("• Audio editing");
                ui.label("• MIDI editing");
                ui.label("• Export your music");

                if ui.button("Close Tutorial").clicked() {
                    self.stop();
                }
            }
        }

        ui.separator();

        // Navigation
        ui.horizontal(|ui| {
            if self.current_step != TutorialStep::Welcome {
                if ui.button("← Previous").clicked() {
                    self.previous_step();
                }
            }

            if ui.button("Exit Tutorial").clicked() {
                self.stop();
            }
        });

        // Progress indicator
        ui.separator();
        let progress = self.current_step as usize as f32 / 7.0;
        ui.add(egui::ProgressBar::new(progress).text("Progress"));
    }

    fn get_step_title(&self) -> &str {
        match self.current_step {
            TutorialStep::Welcome => "Welcome to YADAW!",
            TutorialStep::AddTrack => "Step 1: Add a Track",
            TutorialStep::RecordAudio => "Step 2: Record Audio",
            TutorialStep::AddMidiNotes => "Step 3: Create MIDI",
            TutorialStep::UsePlugins => "Step 4: Add Effects",
            TutorialStep::MixTracks => "Step 5: Mix Your Tracks",
            TutorialStep::SaveProject => "Step 6: Save Your Work",
            TutorialStep::Completed => "Tutorial Complete!",
        }
    }

    fn next_step(&mut self) {
        self.completed_steps.push(self.current_step);
        self.current_step = match self.current_step {
            TutorialStep::Welcome => TutorialStep::AddTrack,
            TutorialStep::AddTrack => TutorialStep::RecordAudio,
            TutorialStep::RecordAudio => TutorialStep::AddMidiNotes,
            TutorialStep::AddMidiNotes => TutorialStep::UsePlugins,
            TutorialStep::UsePlugins => TutorialStep::MixTracks,
            TutorialStep::MixTracks => TutorialStep::SaveProject,
            TutorialStep::SaveProject => TutorialStep::Completed,
            TutorialStep::Completed => TutorialStep::Completed,
        };
    }

    fn previous_step(&mut self) {
        if let Some(prev) = self.completed_steps.pop() {
            self.current_step = prev;
        }
    }

    fn check_step_completion(&mut self, app: &YadawApp) {
        match self.current_step {
            TutorialStep::AddTrack => {
                // Auto-advance if tracks were added
                if app.state.lock().unwrap().tracks.len() > 2 {
                    self.next_step();
                }
            }
            _ => {}
        }
    }

    fn draw_highlight_overlay(&self, ctx: &egui::Context, highlight: egui::Rect) {
        // Draw semi-transparent overlay with a hole for the highlighted area
        let screen = ctx.screen_rect();

        // This is a simple version - need to draw 4 rectangles around the highlight
        // to create a "spotlight" effect
    }
}

// Add to YadawApp:
impl YadawApp {
    pub fn start_tutorial(&mut self) {
        self.tutorial.start();
    }
}

// Add to menu bar:
fn help_menu(&mut self, ui: &mut egui::Ui, app: &mut super::app::YadawApp) {
    ui.menu_button("Help", |ui| {
        if ui.button("Interactive Tutorial").clicked() {
            app.start_tutorial();
            ui.close();
        }
        // TODO: rest of help menu
    });
}
