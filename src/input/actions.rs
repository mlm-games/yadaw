use serde::{Deserialize, Serialize};

/// All possible user actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppAction {
    // Transport
    PlayPause,
    Stop,
    Record,
    GoToStart,
    Rewind,
    FastForward,

    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    DeselectAll,
    Duplicate,

    // File
    NewProject,
    OpenProject,
    SaveProject,
    SaveProjectAs,
    ImportAudio,
    ExportAudio,

    // View
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    ToggleMixer,
    TogglePianoRoll,
    ToggleTimeline,

    // Loop
    ToggleLoop,
    SetLoopToSelection,
    ClearLoop,

    // Piano Roll Specific
    NudgeLeft,
    NudgeRight,
    NudgeLeftFine,
    NudgeRightFine,
    NudgeLeftCoarse,
    NudgeRightCoarse,
    TransposeUp,
    TransposeDown,
    TransposeOctaveUp,
    TransposeOctaveDown,
    VelocityUp,
    VelocityDown,

    // Timeline Specific
    SplitAtPlayhead,
    Normalize,
    Reverse,
    FadeIn,
    FadeOut,

    // Clip Operations
    QuantizeDialog,
    TransposeDialog,
    HumanizeDialog,

    // Other
    Escape,
}

/// Context where an action is valid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionContext {
    Global,
    PianoRoll,
    Timeline,
    Mixer,
    NoteSelection,
    ClipSelection,
}

impl AppAction {
    /// Get all actions (for UI enumeration)
    pub fn all() -> &'static [AppAction] {
        use AppAction::*;
        &[
            PlayPause,
            Stop,
            Record,
            GoToStart,
            Rewind,
            FastForward,
            Undo,
            Redo,
            Cut,
            Copy,
            Paste,
            Delete,
            SelectAll,
            DeselectAll,
            Duplicate,
            NewProject,
            OpenProject,
            SaveProject,
            SaveProjectAs,
            ImportAudio,
            ExportAudio,
            ZoomIn,
            ZoomOut,
            ZoomToFit,
            ToggleMixer,
            TogglePianoRoll,
            ToggleTimeline,
            ToggleLoop,
            SetLoopToSelection,
            ClearLoop,
            NudgeLeft,
            NudgeRight,
            NudgeLeftFine,
            NudgeRightFine,
            NudgeLeftCoarse,
            NudgeRightCoarse,
            TransposeUp,
            TransposeDown,
            TransposeOctaveUp,
            TransposeOctaveDown,
            VelocityUp,
            VelocityDown,
            SplitAtPlayhead,
            Normalize,
            Reverse,
            FadeIn,
            FadeOut,
            QuantizeDialog,
            TransposeDialog,
            HumanizeDialog,
            Escape,
        ]
    }

    pub fn contexts(&self) -> &'static [ActionContext] {
        use ActionContext::*;
        match self {
            // Global transport
            Self::PlayPause
            | Self::Stop
            | Self::Record
            | Self::GoToStart
            | Self::Rewind
            | Self::FastForward => &[Global],

            // Global edit
            Self::Undo
            | Self::Redo
            | Self::Cut
            | Self::Copy
            | Self::Paste
            | Self::SelectAll
            | Self::DeselectAll
            | Self::Duplicate => &[Global],

            // Global file
            Self::NewProject
            | Self::OpenProject
            | Self::SaveProject
            | Self::SaveProjectAs
            | Self::ImportAudio
            | Self::ExportAudio => &[Global],

            // Global view
            Self::ZoomIn
            | Self::ZoomOut
            | Self::ZoomToFit
            | Self::ToggleMixer
            | Self::TogglePianoRoll
            | Self::ToggleTimeline => &[Global],

            // Global loop
            Self::ToggleLoop | Self::SetLoopToSelection | Self::ClearLoop => &[Global],

            // Piano roll only
            Self::NudgeLeft
            | Self::NudgeRight
            | Self::NudgeLeftFine
            | Self::NudgeRightFine
            | Self::NudgeLeftCoarse
            | Self::NudgeRightCoarse
            | Self::TransposeUp
            | Self::TransposeDown
            | Self::TransposeOctaveUp
            | Self::TransposeOctaveDown
            | Self::VelocityUp
            | Self::VelocityDown => &[PianoRoll],

            // Timeline only
            Self::SplitAtPlayhead
            | Self::Normalize
            | Self::Reverse
            | Self::FadeIn
            | Self::FadeOut => &[Timeline],

            // Dialogs
            Self::QuantizeDialog | Self::TransposeDialog | Self::HumanizeDialog => &[PianoRoll],

            Self::Delete => &[Global, PianoRoll, Timeline],
            Self::Escape => &[Global],
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::PlayPause => "Play/Pause",
            Self::Stop => "Stop",
            Self::Record => "Record",
            Self::GoToStart => "Go to Start",
            Self::Rewind => "Rewind",
            Self::FastForward => "Fast Forward",

            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::Delete => "Delete",
            Self::SelectAll => "Select All",
            Self::DeselectAll => "Deselect All",
            Self::Duplicate => "Duplicate",

            Self::NewProject => "New Project",
            Self::OpenProject => "Open Project",
            Self::SaveProject => "Save Project",
            Self::SaveProjectAs => "Save Project As",
            Self::ImportAudio => "Import Audio",
            Self::ExportAudio => "Export Audio",

            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
            Self::ZoomToFit => "Zoom to Fit",
            Self::ToggleMixer => "Toggle Mixer",
            Self::TogglePianoRoll => "Switch to Piano Roll",
            Self::ToggleTimeline => "Switch to Timeline",

            Self::ToggleLoop => "Toggle Loop",
            Self::SetLoopToSelection => "Set Loop to Selection",
            Self::ClearLoop => "Clear Loop",

            Self::NudgeLeft => "Nudge Left (Grid)",
            Self::NudgeRight => "Nudge Right (Grid)",
            Self::NudgeLeftFine => "Nudge Left (Fine)",
            Self::NudgeRightFine => "Nudge Right (Fine)",
            Self::NudgeLeftCoarse => "Nudge Left (Coarse)",
            Self::NudgeRightCoarse => "Nudge Right (Coarse)",

            Self::TransposeUp => "Transpose Up",
            Self::TransposeDown => "Transpose Down",
            Self::TransposeOctaveUp => "Transpose Octave Up",
            Self::TransposeOctaveDown => "Transpose Octave Down",

            Self::VelocityUp => "Velocity Up",
            Self::VelocityDown => "Velocity Down",

            Self::SplitAtPlayhead => "Split at Playhead",
            Self::Normalize => "Normalize",
            Self::Reverse => "Reverse",
            Self::FadeIn => "Fade In",
            Self::FadeOut => "Fade Out",

            Self::QuantizeDialog => "Quantize...",
            Self::TransposeDialog => "Transpose...",
            Self::HumanizeDialog => "Humanize...",

            Self::Escape => "Escape",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::PlayPause
            | Self::Stop
            | Self::Record
            | Self::GoToStart
            | Self::Rewind
            | Self::FastForward => "Transport",

            Self::Undo
            | Self::Redo
            | Self::Cut
            | Self::Copy
            | Self::Paste
            | Self::Delete
            | Self::SelectAll
            | Self::DeselectAll
            | Self::Duplicate => "Edit",

            Self::NewProject
            | Self::OpenProject
            | Self::SaveProject
            | Self::SaveProjectAs
            | Self::ImportAudio
            | Self::ExportAudio => "File",

            Self::ZoomIn
            | Self::ZoomOut
            | Self::ZoomToFit
            | Self::ToggleMixer
            | Self::TogglePianoRoll
            | Self::ToggleTimeline => "View",

            Self::ToggleLoop | Self::SetLoopToSelection | Self::ClearLoop => "Loop",

            Self::NudgeLeft
            | Self::NudgeRight
            | Self::NudgeLeftFine
            | Self::NudgeRightFine
            | Self::NudgeLeftCoarse
            | Self::NudgeRightCoarse
            | Self::TransposeUp
            | Self::TransposeDown
            | Self::TransposeOctaveUp
            | Self::TransposeOctaveDown
            | Self::VelocityUp
            | Self::VelocityDown
            | Self::QuantizeDialog
            | Self::TransposeDialog
            | Self::HumanizeDialog => "Piano Roll",

            Self::SplitAtPlayhead
            | Self::Normalize
            | Self::Reverse
            | Self::FadeIn
            | Self::FadeOut => "Timeline",

            Self::Escape => "Other",
        }
    }
}
