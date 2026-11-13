//! The side-effect queue.
//!
//! The panels are pure descriptions of state; they never hold the socket or the
//! settings file. A click pushes an effect here, and the host drains the queue
//! once per frame and performs the work. That keeps the drawing code testable and
//! keeps the one place that talks to the core in one place.

use std::cell::RefCell;

/// Something the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum SideEffect {
    ArmPairing,
    DisarmPairing,
    StartStream,
    StopStream,
    SetVcam(bool),
    RestartVcam,
    RefreshUsb,
    ConnectUsb(Option<String>),
    DisconnectUsb,
    RestartAdb,
    OpenPlatformToolsUrl,
    ForgetDevice,
    ResolvePair(bool),
    RefreshState,
    SetTab(Tab),
    ToggleFocusPreview,
    SetPreviewEnabled(bool),
    SetMirror(bool),
    SetRotation(u32),
    SetOutputSize(u32, u32),
    SetOutputFollowsStream(bool),
    SetCamera(String),
    SetResolution(String),
    SetVideo { fps: Option<u32>, bitrate: u64 },
    SetQualityMode(bool),
    SetScreenMode(phonecam_protocol::ScreenMode),
    SetTorch(bool),
    SetZoom(f32),
    SetLogLevel(phonecam_protocol::LogLevel),
    SetLogFilterLevel(Option<phonecam_protocol::LogLevel>),
    SetLogFilterSource(Option<crate::views::state::LogSource>),
    DismissNotice(u64),
    ExportDiagnostics,
    CopyDiagnosticsReport,
    ClearLogs,
    OpenLogFile,
    ToggleAlwaysOnTop,
    SelectPairingNetwork(usize),
    CopyPairingPayload,
}

/// The four destinations in the rail, in the order the job happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Setup,
    Capture,
    Publish,
    Diagnostics,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Setup, Tab::Capture, Tab::Publish, Tab::Diagnostics];

    pub fn label(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Capture => "Capture",
            Self::Publish => "Publish",
            Self::Diagnostics => "Diagnostics",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Setup => "Pair a phone (Ctrl+1)",
            Self::Capture => "Choose what the phone captures (Ctrl+2)",
            Self::Publish => "The camera other apps open (Ctrl+3)",
            Self::Diagnostics => "Live numbers and the service log (Ctrl+4)",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Setup => 0,
            Self::Capture => 1,
            Self::Publish => 2,
            Self::Diagnostics => 3,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

thread_local! {
    static QUEUE: RefCell<Vec<SideEffect>> = const { RefCell::new(Vec::new()) };
}

/// Records an effect for the host to perform.
pub fn push_effect(effect: SideEffect) {
    QUEUE.with(|queue| queue.borrow_mut().push(effect));
}

/// Takes everything queued since the last drain.
pub fn drain_effects() -> Vec<SideEffect> {
    QUEUE.with(|queue| std::mem::take(&mut *queue.borrow_mut()))
}

/// Runs `f` with the queue, for tests that need to observe clicks.
pub fn with_queue<R>(f: impl FnOnce(&mut Vec<SideEffect>) -> R) -> R {
    QUEUE.with(|queue| f(&mut queue.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_accumulate_then_drain() {
        // The queue is thread-local, so this test owns its own.
        drain_effects();
        push_effect(SideEffect::StartStream);
        push_effect(SideEffect::StopStream);
        let drained = drain_effects();
        assert_eq!(drained, vec![SideEffect::StartStream, SideEffect::StopStream]);
        assert!(drain_effects().is_empty(), "a drain empties the queue");
    }

    #[test]
    fn tabs_map_to_ctrl_1_to_4() {
        for (index, tab) in Tab::ALL.iter().enumerate() {
            assert_eq!(tab.index(), index);
            assert_eq!(Tab::from_index(index), Some(*tab));
        }
        assert_eq!(Tab::from_index(4), None);
    }

    #[test]
    fn tab_labels_are_verbs_not_nouns() {
        // "Camera" and "Output" as a pair read as synonyms; these cannot.
        assert_eq!(Tab::Capture.label(), "Capture");
        assert_eq!(Tab::Publish.label(), "Publish");
    }

    #[test]
    fn parity_effects_queue_and_drain() {
        drain_effects();
        push_effect(SideEffect::SetOutputFollowsStream(false));
        push_effect(SideEffect::CopyPairingPayload);
        assert_eq!(
            drain_effects(),
            vec![
                SideEffect::SetOutputFollowsStream(false),
                SideEffect::CopyPairingPayload,
            ]
        );
    }
}

