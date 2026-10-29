//! Terminal emulation and GPUI rendering.

mod audit;
pub mod block;
mod engine;
mod event;
pub mod highlight;
mod history_popup;
mod hyperlink;
mod input;
mod keyboard;
mod kitty_graphics;
mod osc;
pub mod record;
mod renderer;
mod search;
mod search_bar;
mod search_ticks;
mod sequence_scanner;
mod shell_integration;
pub mod trigger;
mod view;
mod view_ime;

pub use engine::TerminalEngine;
pub use event::{HistoryProvider, TerminalEvent, TerminalEventProxy};
pub use highlight::{CompiledHighlight, HighlightKind, HighlightSpan};
pub use input::{
    KeyEventKind, MouseButtonCode, encode_mouse, focus_bytes, keystroke_event_to_bytes,
    keystroke_to_bytes, paste_bytes, tab_bytes,
};
pub use record::{
    CastEvent, CastEventType, CastHeader, RecorderHandle, ReplayInputStep, SessionRecorder,
    extract_replay_inputs, parse_cast_file,
};
pub use shell_integration::{
    HANDSHAKE_TOKEN, IntegrationAction, IntegrationContext, IntegrationHalt, IntegrationSignal,
    ShellIntegration, ShellKind, detect_shell, get_integration_script, probe_command,
};
pub use trigger::{CompiledTrigger, TriggerEngine, TriggerMatchResult};
pub use view::{KeyFilter, TerminalView, TerminalViewParams};
