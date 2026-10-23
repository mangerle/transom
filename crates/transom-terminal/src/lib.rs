//! Terminal emulation and GPUI rendering.

mod engine;
mod event;
mod input;
mod osc;
mod renderer;
mod renderer_search;
mod search;
mod search_bar;
mod view;

pub use engine::TerminalEngine;
pub use event::{TerminalEvent, TerminalEventProxy};
pub use input::{MouseButtonCode, encode_mouse, focus_bytes, keystroke_to_bytes, paste_bytes};
pub use view::{KeyFilter, TerminalView};
