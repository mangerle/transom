//! 终端交互脚本录制与回放管理模块。
//!
//! 回放选择弹窗的状态与渲染归 [`ReplayModalView`]；外壳入口在 `ops.rs`，
//! 录制底栏徽标在 `view.rs`，实体意图消费在 `shell.rs`。

mod entity;
mod ops;
mod render;
mod shell;
mod types;
mod view;

pub(crate) use entity::{ReplayEvent, ReplayModalView};
pub(crate) use ops::{
    open_cast_player, open_replay_modal, toggle_recording, toggle_recording_for_active_tab,
};
pub(crate) use shell::on_replay_event;
pub(crate) use types::ReplayModalState;
pub(crate) use view::render_recording_badge;
