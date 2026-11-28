//! AI 面板后台异步任务槽位。

use gpui_kit::*;

/// AI 面板主视图持有的后台异步任务槽位（覆盖式取消旧任务）。
#[derive(Default)]
pub(super) struct AiPanelTasks {
    pub(super) copy_reset: Option<Task<()>>,
    pub(super) attachment: Option<Task<()>>,
    pub(super) _init_load: Option<Task<()>>,
    pub(super) refresh_index: Option<Task<()>>,
    pub(super) conversation_load: Option<Task<()>>,
    pub(super) memory_load: Option<Task<()>>,
}
