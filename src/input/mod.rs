pub mod actions;
pub mod gestures;
pub mod shortcuts;

use actions::{ActionContext, AppAction};
use gestures::{GestureAction, GestureRecognizer};
use shortcuts::ShortcutRegistry;

use egui::Context;

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
        let mut actions = Vec::new();

        // Keyboard shortcuts
        for (&action, bindings) in &self.shortcuts.bindings {
            // Check if action is valid in current context
            if !action.contexts().contains(&self.current_context)
                && !action.contexts().contains(&ActionContext::Global)
            {
                continue;
            }

            for bind in bindings {
                if ctx.input_mut(|i| i.consume_shortcut(&bind.to_egui())) {
                    actions.push(action);
                    break; // Only trigger once per action
                }
            }
        }

        // Touch gestures (map to actions contextually)
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
