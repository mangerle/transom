//! Terminal emulation and GPUI rendering.

// 终端 crate 自己渲染右键菜单、搜索栏与块横幅，文案必须与应用主界面共用同一份
// 资源。rust-i18n 的 backend 由 i18n! 宏按 crate 编译内嵌，无法直接复用
// transom-ui 的初始化，因此这里加载 workspace 根下的共享 locales；语言切换是
// 全局 locale，两个 backend 会同步生效。
rust_i18n::i18n!("../../locales", fallback = "en");

mod audit;
mod history_popup;
mod hyperlink;
mod input;
mod renderer;
mod search_bar;
mod view;
mod view_ime;

// 视图层通过同一组名称访问核心类型，迁移不影响既有模块路径。
pub use input::{
    KeyEventKind, MouseButtonCode, encode_mouse, focus_bytes, keystroke_event_to_bytes,
    keystroke_to_bytes, paste_bytes, tab_bytes,
};
pub use transom_terminal_core::{
    CastEvent, CastEventType, CastHeader, CommandBlock, CompiledHighlight, CompiledTrigger,
    CompletionProvider, GraphicsPlacement, HANDSHAKE_TOKEN, HighlightKind, HighlightSpan,
    IntegrationAction, IntegrationContext, IntegrationHalt, IntegrationSignal, ProgressState,
    RecorderHandle, ReplayInputStep, SessionRecorder, ShellIntegration, ShellKind, TerminalEngine,
    TerminalEvent, TerminalEventProxy, TriggerEngine, TriggerMatchResult, detect_shell,
    extract_replay_inputs, get_integration_script, parse_cast_file, probe_command, record, trigger,
};
pub(crate) use transom_terminal_core::{
    block, engine, event, highlight, kitty_graphics, osc, search, shell_integration,
};
pub use view::{
    CONTEXT as TERMINAL_CONTEXT, KeyFilter, SendBacktab, SendTab, TerminalCopy,
    TerminalDirectoryError, TerminalPaste, TerminalReattachParams, TerminalView,
    TerminalViewParams,
};
