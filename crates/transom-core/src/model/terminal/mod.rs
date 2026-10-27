//! 终端配置方案、调色板、控制流与键位映射模型。

mod io;
mod key_bindings;
mod profile;

#[cfg(test)]
mod tests;

pub use io::{TerminalControl, TerminalControlHandle, TerminalIo, TerminalSize};
pub use key_bindings::TerminalKeyBinding;
#[cfg(test)]
pub(crate) use key_bindings::decode_escape_sequence;
pub use profile::{
    FOLLOW_UI_THEME, RightClickBehavior, TerminalCursorShape, TerminalInteraction, TerminalPalette,
    TerminalProfile,
};
