use super::actions::{ActionContext, AppAction};
use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single keybind (modifier + key)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Keybind {
    pub modifiers: ModifierSet,
    pub key: KeyCode,
}

/// Serializable modifier flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModifierSet {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub command: bool,
}

impl From<Modifiers> for ModifierSet {
    fn from(m: Modifiers) -> Self {
        Self {
            ctrl: m.ctrl,
            shift: m.shift,
            alt: m.alt,
            command: m.command,
        }
    }
}

impl From<ModifierSet> for Modifiers {
    fn from(m: ModifierSet) -> Self {
        let mut mods = Modifiers::NONE;

        #[cfg(target_os = "macos")]
        {
            if m.command {
                mods = mods | Modifiers::COMMAND;
            }
            if m.ctrl {
                mods = mods | Modifiers::CTRL;
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            // Treat "command" as "ctrl"
            if m.ctrl || m.command {
                mods = mods | Modifiers::CTRL | Modifiers::COMMAND;
            }
        }

        if m.shift {
            mods = mods | Modifiers::SHIFT;
        }
        if m.alt {
            mods = mods | Modifiers::ALT;
        }
        mods
    }
}

/// Serializable key enum (subset of egui::Key)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Space,
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl From<KeyCode> for Key {
    fn from(k: KeyCode) -> Self {
        use egui::Key as K;
        match k {
            KeyCode::A => K::A,
            KeyCode::B => K::B,
            KeyCode::C => K::C,
            KeyCode::D => K::D,
            KeyCode::E => K::E,
            KeyCode::F => K::F,
            KeyCode::G => K::G,
            KeyCode::H => K::H,
            KeyCode::I => K::I,
            KeyCode::J => K::J,
            KeyCode::K => K::K,
            KeyCode::L => K::L,
            KeyCode::M => K::M,
            KeyCode::N => K::N,
            KeyCode::O => K::O,
            KeyCode::P => K::P,
            KeyCode::Q => K::Q,
            KeyCode::R => K::R,
            KeyCode::S => K::S,
            KeyCode::T => K::T,
            KeyCode::U => K::U,
            KeyCode::V => K::V,
            KeyCode::W => K::W,
            KeyCode::X => K::X,
            KeyCode::Y => K::Y,
            KeyCode::Z => K::Z,

            KeyCode::Num0 => K::Num0,
            KeyCode::Num1 => K::Num1,
            KeyCode::Num2 => K::Num2,
            KeyCode::Num3 => K::Num3,
            KeyCode::Num4 => K::Num4,
            KeyCode::Num5 => K::Num5,
            KeyCode::Num6 => K::Num6,
            KeyCode::Num7 => K::Num7,
            KeyCode::Num8 => K::Num8,
            KeyCode::Num9 => K::Num9,

            KeyCode::Space => K::Space,
            KeyCode::Enter => K::Enter,
            KeyCode::Escape => K::Escape,
            KeyCode::Backspace => K::Backspace,
            KeyCode::Delete => K::Delete,
            KeyCode::Tab => K::Tab,

            KeyCode::ArrowUp => K::ArrowUp,
            KeyCode::ArrowDown => K::ArrowDown,
            KeyCode::ArrowLeft => K::ArrowLeft,
            KeyCode::ArrowRight => K::ArrowRight,

            KeyCode::Home => K::Home,
            KeyCode::End => K::End,
            KeyCode::PageUp => K::PageUp,
            KeyCode::PageDown => K::PageDown,

            KeyCode::F1 => K::F1,
            KeyCode::F2 => K::F2,
            KeyCode::F3 => K::F3,
            KeyCode::F4 => K::F4,
            KeyCode::F5 => K::F5,
            KeyCode::F6 => K::F6,
            KeyCode::F7 => K::F7,
            KeyCode::F8 => K::F8,
            KeyCode::F9 => K::F9,
            KeyCode::F10 => K::F10,
            KeyCode::F11 => K::F11,
            KeyCode::F12 => K::F12,
        }
    }
}

impl TryFrom<Key> for KeyCode {
    type Error = ();
    fn try_from(k: Key) -> Result<Self, ()> {
        use egui::Key as K;
        Ok(match k {
            K::A => KeyCode::A,
            K::B => KeyCode::B,
            K::C => KeyCode::C,
            K::D => KeyCode::D,
            K::E => KeyCode::E,
            K::F => KeyCode::F,
            K::G => KeyCode::G,
            K::H => KeyCode::H,
            K::I => KeyCode::I,
            K::J => KeyCode::J,
            K::K => KeyCode::K,
            K::L => KeyCode::L,
            K::M => KeyCode::M,
            K::N => KeyCode::N,
            K::O => KeyCode::O,
            K::P => KeyCode::P,
            K::Q => KeyCode::Q,
            K::R => KeyCode::R,
            K::S => KeyCode::S,
            K::T => KeyCode::T,
            K::U => KeyCode::U,
            K::V => KeyCode::V,
            K::W => KeyCode::W,
            K::X => KeyCode::X,
            K::Y => KeyCode::Y,
            K::Z => KeyCode::Z,

            K::Num0 => KeyCode::Num0,
            K::Num1 => KeyCode::Num1,
            K::Num2 => KeyCode::Num2,
            K::Num3 => KeyCode::Num3,
            K::Num4 => KeyCode::Num4,
            K::Num5 => KeyCode::Num5,
            K::Num6 => KeyCode::Num6,
            K::Num7 => KeyCode::Num7,
            K::Num8 => KeyCode::Num8,
            K::Num9 => KeyCode::Num9,

            K::Space => KeyCode::Space,
            K::Enter => KeyCode::Enter,
            K::Escape => KeyCode::Escape,
            K::Backspace => KeyCode::Backspace,
            K::Delete => KeyCode::Delete,
            K::Tab => KeyCode::Tab,

            K::ArrowUp => KeyCode::ArrowUp,
            K::ArrowDown => KeyCode::ArrowDown,
            K::ArrowLeft => KeyCode::ArrowLeft,
            K::ArrowRight => KeyCode::ArrowRight,

            K::Home => KeyCode::Home,
            K::End => KeyCode::End,
            K::PageUp => KeyCode::PageUp,
            K::PageDown => KeyCode::PageDown,

            K::F1 => KeyCode::F1,
            K::F2 => KeyCode::F2,
            K::F3 => KeyCode::F3,
            K::F4 => KeyCode::F4,
            K::F5 => KeyCode::F5,
            K::F6 => KeyCode::F6,
            K::F7 => KeyCode::F7,
            K::F8 => KeyCode::F8,
            K::F9 => KeyCode::F9,
            K::F10 => KeyCode::F10,
            K::F11 => KeyCode::F11,
            K::F12 => KeyCode::F12,
            _ => return Err(()),
        })
    }
}

impl Keybind {
    pub fn to_egui(&self) -> KeyboardShortcut {
        KeyboardShortcut::new(self.modifiers.into(), self.key.into())
    }

    /// Format for display ("Ctrl+Shift+S")
    pub fn to_string(&self) -> String {
        let mut parts = Vec::new();

        #[cfg(target_os = "macos")]
        let cmd_key = "Cmd";
        #[cfg(not(target_os = "macos"))]
        let cmd_key = "Ctrl";

        if self.modifiers.command {
            parts.push(cmd_key);
        }
        if self.modifiers.ctrl && !cfg!(target_os = "macos") {
            parts.push("Ctrl");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }

        // FIX: Create a longer-lived String for the key name
        let key_str = format!("{:?}", self.key);
        parts.push(&key_str);

        parts.join("+")
    }
}

/// Central shortcut registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutRegistry {
    /// Action -> List of keybinds (allows multiple binds per action)
    pub bindings: HashMap<AppAction, Vec<Keybind>>,

    /// Reverse lookup for conflict detection
    #[serde(skip)]
    keybind_to_action: HashMap<Keybind, AppAction>,
}

impl Default for ShortcutRegistry {
    fn default() -> Self {
        Self::default_bindings()
    }
}

impl ShortcutRegistry {
    /// Load default keybinds
    pub fn default_bindings() -> Self {
        let mut reg = Self {
            bindings: HashMap::new(),
            keybind_to_action: HashMap::new(),
        };

        use AppAction::*;
        use KeyCode::*;

        // Transport (no modifiers - global hotkeys)
        reg.bind(PlayPause, Keybind::none(Space));
        reg.bind(Stop, Keybind::none(KeyCode::Escape));
        reg.bind(GoToStart, Keybind::none(Home));

        // Edit (Cmd/Ctrl)
        reg.bind(Undo, Keybind::cmd(Z));
        reg.bind(Redo, Keybind::cmd_shift(Z));
        reg.bind(Cut, Keybind::cmd(X));
        reg.bind(Copy, Keybind::cmd(C));
        reg.bind(Paste, Keybind::cmd(V));
        reg.bind(SelectAll, Keybind::cmd(A));
        reg.bind(AppAction::Delete, Keybind::none(KeyCode::Delete));

        // File
        reg.bind(NewProject, Keybind::cmd(N));
        reg.bind(OpenProject, Keybind::cmd(O));
        reg.bind(SaveProject, Keybind::cmd(S));
        reg.bind(SaveProjectAs, Keybind::cmd_shift(S));

        // View
        reg.bind(ToggleMixer, Keybind::cmd(M));
        reg.bind(ToggleLoop, Keybind::none(L));
        reg.bind(SetLoopToSelection, Keybind::cmd(L));
        reg.bind(ClearLoop, Keybind::shift(L));

        // Piano Roll Navigation
        reg.bind(NudgeLeft, Keybind::none(ArrowLeft));
        reg.bind(NudgeRight, Keybind::none(ArrowRight));
        reg.bind(NudgeLeftFine, Keybind::alt(ArrowLeft));
        reg.bind(NudgeRightFine, Keybind::alt(ArrowRight));
        reg.bind(NudgeLeftCoarse, Keybind::shift(ArrowLeft));
        reg.bind(NudgeRightCoarse, Keybind::shift(ArrowRight));

        reg.bind(TransposeUp, Keybind::none(ArrowUp));
        reg.bind(TransposeDown, Keybind::none(ArrowDown));
        reg.bind(TransposeOctaveUp, Keybind::shift(ArrowUp));
        reg.bind(TransposeOctaveDown, Keybind::shift(ArrowDown));

        reg.bind(VelocityUp, Keybind::cmd(ArrowUp));
        reg.bind(VelocityDown, Keybind::cmd(ArrowDown));

        reg
    }

    /// Add a binding (allows duplicates)
    pub fn bind(&mut self, action: AppAction, keybind: Keybind) {
        self.bindings.entry(action).or_default().push(keybind);
        self.keybind_to_action.insert(keybind, action);
    }

    /// Remove a specific binding
    pub fn unbind(&mut self, keybind: &Keybind) {
        self.keybind_to_action.remove(keybind);
        for binds in self.bindings.values_mut() {
            binds.retain(|b| b != keybind);
        }
    }

    /// Get all keybinds for an action
    pub fn get_bindings(&self, action: AppAction) -> &[Keybind] {
        self.bindings
            .get(&action)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Find action for a keybind
    pub fn get_action(&self, keybind: &Keybind) -> Option<AppAction> {
        self.keybind_to_action.get(keybind).copied()
    }

    /// Check if a keybind is already used
    pub fn has_conflict(
        &self,
        keybind: &Keybind,
        exclude_action: Option<AppAction>,
    ) -> Option<AppAction> {
        self.keybind_to_action
            .get(keybind)
            .copied()
            .filter(|&a| exclude_action.map_or(true, |ex| ex != a))
    }

    /// Save to file
    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from file
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let mut reg: Self = serde_json::from_str(&json)?;
        reg.rebuild_reverse_index();
        Ok(reg)
    }

    fn rebuild_reverse_index(&mut self) {
        self.keybind_to_action.clear();
        for (&action, binds) in &self.bindings {
            for &bind in binds {
                self.keybind_to_action.insert(bind, action);
            }
        }
    }
}

impl Keybind {
    pub fn none(key: KeyCode) -> Self {
        Self {
            modifiers: ModifierSet::NONE,
            key,
        }
    }

    pub fn cmd(key: KeyCode) -> Self {
        Self {
            modifiers: ModifierSet::COMMAND,
            key,
        }
    }

    pub fn shift(key: KeyCode) -> Self {
        Self {
            modifiers: ModifierSet::SHIFT,
            key,
        }
    }

    pub fn alt(key: KeyCode) -> Self {
        Self {
            modifiers: ModifierSet::ALT,
            key,
        }
    }

    pub fn cmd_shift(key: KeyCode) -> Self {
        Self {
            modifiers: ModifierSet {
                command: true,
                shift: true,
                ..Default::default()
            },
            key,
        }
    }
}

impl ModifierSet {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        command: false,
    };
    pub const COMMAND: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        command: true,
    };
    pub const SHIFT: Self = Self {
        ctrl: false,
        shift: true,
        alt: false,
        command: false,
    };
    pub const ALT: Self = Self {
        ctrl: false,
        shift: false,
        alt: true,
        command: false,
    };
}

impl Default for ModifierSet {
    fn default() -> Self {
        Self::NONE
    }
}

impl std::fmt::Display for Keybind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}
