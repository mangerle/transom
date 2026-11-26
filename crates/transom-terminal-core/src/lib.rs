//! 终端仿真核心：不依赖 GPUI，负责 VTE 引擎、OSC 解析、录制、检索、触发器和图像协议。
//!
//! 设计原理（Why）：
//! - 终端引擎属于业务核心，测试不应为了验证网格、OSC 或 Kitty 协议而构造 GPUI
//!   窗口；把核心放入独立 crate 后，依赖图会直接阻止 UI 类型反向污染；
//! - `transom-terminal` 只保留 GPUI 渲染与交互，通过重导出保持现有调用方路径稳定；
//! - `TerminalImage` 使用纯字节缓冲而不是 GPUI 纹理类型，渲染层在真正上传纹理时
//!   再做一次适配，核心因此可以独立编译、测试和复用。

rust_i18n::i18n!("../../locales", fallback = "en");

pub mod block;
pub mod engine;
pub mod event;
pub mod highlight;
pub mod input;
pub mod keyboard;
pub mod kitty_graphics;
pub mod osc;
pub mod record;
pub mod search;
pub mod sequence_scanner;
pub mod shell_integration;
pub mod trigger;

mod search_ticks;

pub use block::{CommandBlock, VisualSlot};
pub use engine::TerminalEngine;
pub use event::{CompletionProvider, TerminalEvent, TerminalEventProxy};
pub use highlight::{CompiledHighlight, HighlightKind, HighlightSpan};
pub use kitty_graphics::GraphicsPlacement;
pub use osc::ProgressState;
pub use record::{
    CastEvent, CastEventType, CastHeader, RecorderHandle, ReplayInputStep, SessionRecorder,
    extract_replay_inputs, parse_cast_file,
};
pub use shell_integration::{
    HANDSHAKE_TOKEN, IntegrationAction, IntegrationContext, IntegrationHalt, IntegrationSignal,
    ShellIntegration, ShellKind, detect_shell, get_integration_script, probe_command,
};
pub use trigger::{CompiledTrigger, TriggerEngine, TriggerMatchResult};
