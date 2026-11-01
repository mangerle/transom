//! 渲染期窗口动作队列。
//!
//! 设计原理（Why）：面板实体在事件派发阶段被 GPUI 租借，拿不到 `Window`，
//! 需要窗口的动作（打开弹窗、聚焦、调整布局）必须延后到渲染期。此前四个
//! 领域各有一个 `pending_*_requests` 向量，外壳每帧按固定顺序分别消费；
//! 队列合并后只有一个消费入口，新增请求类型不会漏接 flush。
//!
//! 代价与局限：合并后按入队顺序执行，跨领域动作的先后由产生顺序决定；
//! 需要严格先后的动作应在产生点排好，而不是依赖队列分组的固定次序。

use crate::editor::EditorPanelEvent;
use crate::monitor::MonitorPanelEvent;
use crate::settings::SettingsRequest;
use crate::sftp::SftpPanelEvent;

/// 一条需要窗口才能执行的请求。
pub(crate) enum WindowRequest {
    Sftp(SftpPanelEvent),
    Monitor(MonitorPanelEvent),
    Editor(EditorPanelEvent),
    Settings(SettingsRequest),
}

/// 有界请求队列：只在当前帧内积压，渲染期一次取空。
#[derive(Default)]
pub(crate) struct WindowRequests {
    items: Vec<WindowRequest>,
}

impl WindowRequests {
    pub(crate) fn push_sftp(&mut self, request: SftpPanelEvent) {
        self.items.push(WindowRequest::Sftp(request));
    }

    pub(crate) fn push_monitor(&mut self, request: MonitorPanelEvent) {
        self.items.push(WindowRequest::Monitor(request));
    }

    pub(crate) fn push_editor(&mut self, request: EditorPanelEvent) {
        self.items.push(WindowRequest::Editor(request));
    }

    pub(crate) fn push_settings(&mut self, request: SettingsRequest) {
        self.items.push(WindowRequest::Settings(request));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 取走本轮全部请求；渲染期逐个执行。
    pub(crate) fn take(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.items)
    }
}
