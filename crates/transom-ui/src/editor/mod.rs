//! 远端文本编辑器子系统。
//!
//! 设计说明（Why）：
//! - 实现初衷：从 SFTP 文件管理器中完整剥离文本编辑器的全部生命周期
//!   （文件打开、暂存、冲突检测、Diff、编码转换、行跳转与拖拽布局）；
//! - 核心优势：
//!   1. 解除 SFTP 状态机与编辑器状态机的深度耦合；
//!   2. 编辑器作为独立的浮层子系统，具备可拖拽、可折叠、非阻塞的生命周期；
//!   3. SFTP 仅作为“在编辑器中打开”的触发来源之一，职责清晰单纯。

mod actions;
mod banner;
mod close;
mod code_link;
mod code_open;
mod diff;
mod drag_tracker;
mod list;
mod manager;
mod navigate;
mod open;
mod ops;
mod panel;
mod view;

pub(crate) use close::PendingEditorClose;
pub(crate) use code_open::RemoteCodeLocation;
pub(crate) use drag_tracker::EditorDragTracker;
pub(crate) use list::render_editor_list_popover;
pub(crate) use manager::EditorManager;
pub(crate) use panel::{EditorPhase, caret_of, editor_language_token};
