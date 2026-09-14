pub mod actions;
pub mod gestures;
pub mod shortcuts;

use actions::{ActionContext, AppAction};
use gestures::{GestureAction, GestureRecognizer};
use shortcuts::ShortcutRegistry;

use egui::{Context, Key, Modifiers};

pub struct InputManager {
    shortcuts: ShortcutRegistry,
    gestures: GestureRecognizer,
    current_context: ActionContext,
}

impl InputManager {
    pub fn new() -> Self {
        Self {
            shortcuts: ShortcutRegistry::default(),
            gestures: GestureRecognizer::new(),
            current_context: ActionContext::Global,
        }
    }

    /// Load custom shortcuts from config
    pub fn load_shortcuts(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.shortcuts = ShortcutRegistry::load(path)?;
        Ok(())
    }

    /// Load shortcuts from JSON string (used on wasm)
    pub fn load_shortcuts_from_json(&mut self, json: &str) -> anyhow::Result<()> {
        self.shortcuts = ShortcutRegistry::load_from_json(json)?;
        Ok(())
    }

    /// Save shortcuts
    pub fn save_shortcuts(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.shortcuts.save(path)
    }

    /// Set current UI context (affects which shortcuts are active)
    pub fn set_context(&mut self, context: ActionContext) {
        self.current_context = context;
    }

    /// Process input and return triggered actions
    pub fn poll_actions(&mut self, ctx: &Context) -> Vec<AppAction> {
        // Don't process shortcuts when text input has focus (dialogs, BPM field, etc.)
        if ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                return vec![AppAction::Escape];
            }
            return vec![];
        }

        let mut actions = Vec::new();
        let modifiers = ctx.input(|i| i.modifiers);

        use std::collections::HashSet;
        let mut fired_actions: HashSet<AppAction> = HashSet::new();
        let pressed: Vec<(Key, Modifiers)> = ctx.input(|i| {
            let mut out = Vec::new();
            let mut seen_keys: HashSet<Key> = HashSet::new();
            for binds in self.shortcuts.bindings.values() {
                for bind in binds {
                    let key: Key = bind.key.into();
                    if seen_keys.insert(key) && i.key_pressed(key) {
                        out.push((key, i.modifiers));
                    }
                }
            }
            out
        });
        let _ = modifiers;
        for (key, mods) in pressed {
            if let Ok(code) = crate::input::shortcuts::KeyCode::try_from(key) {
                let bind = crate::input::shortcuts::Keybind {
                    modifiers: crate::input::shortcuts::ModifierSet::from(mods),
                    key: code,
                };
                if let Some(action) = self.shortcuts.get_action(&bind) {
                    if !action.contexts().contains(&self.current_context)
                        && !action.contexts().contains(&ActionContext::Global)
                    {
                        continue;
                    }
                    if fired_actions.insert(action) {
                        actions.push(action);
                    }
                }
            }
        }

        // Touch gestures
        for gesture in self.gestures.process(ctx) {
            match gesture {
                GestureAction::DoubleTap { .. } => {
                    // Context-dependent action
                    match self.current_context {
                        ActionContext::Timeline => actions.push(AppAction::Duplicate),
                        _ => {}
                    }
                }
                GestureAction::LongPress { .. } => {
                    // TODO: Show context menu in far future
                }
                _ => {} // Pan/Pinch handled separately in views
            }
        }

        actions
    }

    /// Get reference to shortcut registry (for UI editing)
    pub fn shortcuts(&self) -> &ShortcutRegistry {
        &self.shortcuts
    }

    pub fn shortcuts_mut(&mut self) -> &mut ShortcutRegistry {
        &mut self.shortcuts
    }
}
