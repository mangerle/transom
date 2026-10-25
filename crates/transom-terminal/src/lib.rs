//! Terminal emulation and GPUI rendering.

mod audit;
mod engine;
mod event;
mod history_popup;
mod input;
mod osc;
mod renderer;
mod renderer_search;
mod search;
mod search_bar;
mod search_ticks;
mod view;
mod view_ime;

pub use engine::TerminalEngine;
pub use event::{HistoryProvider, TerminalEvent, TerminalEventProxy};
pub use input::{MouseButtonCode, encode_mouse, focus_bytes, keystroke_to_bytes, paste_bytes};
pub use view::{KeyFilter, TerminalView};
