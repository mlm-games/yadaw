use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Theme {
    Dark,
    Light,
    Custom(CustomTheme),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTheme {
    pub name: String,
    pub background: Color32Ser,
    pub foreground: Color32Ser,
    pub primary: Color32Ser,
    pub secondary: Color32Ser,
    pub success: Color32Ser,
    pub warning: Color32Ser,
    pub error: Color32Ser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Color32Ser {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl From<egui::Color32> for Color32Ser {
    fn from(c: egui::Color32) -> Self {
        let [r, g, b, a] = c.to_array();
        Self { r, g, b, a }
    }
}

impl From<Color32Ser> for egui::Color32 {
    fn from(c: Color32Ser) -> Self {
        egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
    }
}

#[derive(Clone)]
pub struct ThemeManager {
    current_theme: Theme,
    custom_themes: Vec<CustomTheme>,
}

impl ThemeManager {
    pub fn new(theme: Theme) -> Self {
        Self {
            current_theme: theme,
            custom_themes: Vec::new(),
        }
    }

    pub fn apply_theme(&self, ctx: &egui::Context) {
        match &self.current_theme {
            Theme::Dark => {
                ctx.set_visuals(egui::Visuals::dark());
            }
            Theme::Light => {
                ctx.set_visuals(egui::Visuals::light());
            }
            Theme::Custom(custom) => {
                let mut visuals = egui::Visuals::dark();

                // Apply custom colors
                visuals.override_text_color = Some(custom.foreground.clone().into());
                visuals.panel_fill = custom.background.clone().into();
                visuals.window_fill = custom.background.clone().into();

                // Apply to widgets
                visuals.widgets.noninteractive.bg_fill = custom.background.clone().into();
                visuals.widgets.inactive.bg_fill = custom.secondary.clone().into();
                visuals.widgets.active.bg_fill = custom.primary.clone().into();
                visuals.widgets.hovered.bg_fill = custom.primary.clone().into();

                ctx.set_visuals(visuals);
            }
        }
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.current_theme = theme;
    }

    pub fn add_custom_theme(&mut self, theme: CustomTheme) {
        self.custom_themes.push(theme);
    }

    pub fn get_custom_themes(&self) -> &[CustomTheme] {
        &self.custom_themes
    }
}

// Predefined theme presets
impl ThemeManager {
    pub fn create_dark_blue_theme() -> CustomTheme {
        CustomTheme {
            name: "Dark Blue".to_string(),
            background: Color32Ser {
                r: 20,
                g: 25,
                b: 40,
                a: 255,
            },
            foreground: Color32Ser {
                r: 200,
                g: 200,
                b: 220,
                a: 255,
            },
            primary: Color32Ser {
                r: 80,
                g: 120,
                b: 200,
                a: 255,
            },
            secondary: Color32Ser {
                r: 40,
                g: 50,
                b: 70,
                a: 255,
            },
            success: Color32Ser {
                r: 80,
                g: 200,
                b: 120,
                a: 255,
            },
            warning: Color32Ser {
                r: 200,
                g: 180,
                b: 80,
                a: 255,
            },
            error: Color32Ser {
                r: 200,
                g: 80,
                b: 80,
                a: 255,
            },
        }
    }

    pub fn create_dark_green_theme() -> CustomTheme {
        CustomTheme {
            name: "Dark Green".to_string(),
            background: Color32Ser {
                r: 20,
                g: 30,
                b: 25,
                a: 255,
            },
            foreground: Color32Ser {
                r: 200,
                g: 220,
                b: 200,
                a: 255,
            },
            primary: Color32Ser {
                r: 80,
                g: 160,
                b: 100,
                a: 255,
            },
            secondary: Color32Ser {
                r: 40,
                g: 60,
                b: 50,
                a: 255,
            },
            success: Color32Ser {
                r: 100,
                g: 200,
                b: 120,
                a: 255,
            },
            warning: Color32Ser {
                r: 200,
                g: 180,
                b: 80,
                a: 255,
            },
            error: Color32Ser {
                r: 200,
                g: 80,
                b: 80,
                a: 255,
            },
        }
    }
}
