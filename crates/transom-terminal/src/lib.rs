//! Terminal emulation and GPUI rendering.

// 终端 crate 自己渲染右键菜单、搜索栏与块横幅，文案必须与应用主界面共用同一份
// 资源。rust-i18n 的 backend 由 i18n! 宏按 crate 编译内嵌，无法直接复用
// transom-ui 的初始化，因此这里以相对路径加载同一套 locales；语言切换是全局
// locale，两个 backend 会同步生效。
rust_i18n::i18n!("../transom-ui/locales", fallback = "en");

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
pub use view::{
    CONTEXT as TERMINAL_CONTEXT, KeyFilter, TerminalCopy, TerminalPaste, TerminalView,
    TerminalViewParams,
};
