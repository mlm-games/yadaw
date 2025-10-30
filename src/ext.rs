use crate::{
    messages::{AudioCommand, UIUpdate},
    ui::YadawApp,
};
use once_cell::sync::Lazy;
use parking_lot::RwLock;

pub trait AppExtension: Send + Sync {
    fn id(&self) -> &'static str;
    fn menu_items(&self) -> Vec<(&'static str, fn(&mut YadawApp))> {
        Vec::new()
    }
    fn panels(&self) -> Vec<fn(&mut egui::Ui, &mut YadawApp)> {
        Vec::new()
    }
    fn on_ui_update(&self, _update: &UIUpdate, _app: &mut YadawApp) {}
    fn on_command(&self, _cmd: &AudioCommand, _app: &mut YadawApp) {}
}

// Mutable-at-runtime registry
static EXTENSIONS: Lazy<RwLock<Vec<&'static dyn AppExtension>>> =
    Lazy::new(|| RwLock::new(Vec::new()));

pub fn register(exts: Vec<&'static dyn AppExtension>) {
    EXTENSIONS.write().extend(exts);
}

pub fn all() -> Vec<&'static dyn AppExtension> {
    EXTENSIONS.read().clone()
}
