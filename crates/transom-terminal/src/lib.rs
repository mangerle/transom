//! Terminal emulation and GPUI rendering.

mod audit;
mod engine;
mod event;
mod history_popup;
mod input;
mod osc;
pub mod record;
mod renderer;
mod renderer_search;
mod search;
mod search_bar;
mod search_ticks;
pub mod trigger;
mod view;
mod view_ime;

pub use engine::TerminalEngine;
pub use event::{HistoryProvider, TerminalEvent, TerminalEventProxy};
pub use input::{MouseButtonCode, encode_mouse, focus_bytes, keystroke_to_bytes, paste_bytes};
pub use record::{
    CastEvent, CastEventType, CastHeader, RecorderHandle, ReplayInputStep, SessionRecorder,
    extract_replay_inputs, parse_cast_file,
};
pub use trigger::{CompiledTrigger, TriggerEngine, TriggerMatchResult};
pub use view::{KeyFilter, TerminalView};
