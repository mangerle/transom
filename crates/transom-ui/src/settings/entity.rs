//! 设置弹窗实体：持有状态、输入控件、页面渲染与保存流程。
//!
//! # 设计原理（Why）
//! - 设置弹窗是窗口级浮层，生命周期与窗口一致；状态、渲染、表单收集与落盘
//!   都属于同一个领域，因此收进一个实体，外壳只持有句柄并订阅意图事件。
//! - 设置值读 `Global`，写 `Global`；需要跨域协调的动作（下发到活窗格、
//!   搬动 SFTP 列宽、重启监控采样）走 [`SettingsEvent`]，事件自带数据，
//!   外壳处理时不得回读本实体。
//! - 渲染由 [`crate::dialog_host`] 的弹窗体通过 `Entity::update` 调用
//!   [`SettingsView::render_body`]：弹窗容器与页面不在同一个租借周期内，
//!   页面里的 `cx.listener` 才能在稍后的绘制阶段安全执行。
//!
//! # 代价与局限
//! 跨域动作多一次事件派发；保存失败时弹窗保持打开并把错误交给外壳通知层，
//! 实体自身不持有窗口级提示控件。

use super::SettingsModalState;
use super::event::SettingsEvent;
use crate::app::Persistence;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{AppSettings, SecretStore, SessionCatalog};

/// 设置实体运行所需的依赖。
///
/// 只注入它真正用到的服务：保存走共享写盘通道，凭据模板与云同步口令走
/// 系统钥匙串。刻意不注入命令通道与事件接收端，避免设置实体越权触碰
/// 连接控制面。
#[derive(Clone)]
pub(crate) struct SettingsServices {
    pub(crate) secret_store: Arc<dyn SecretStore>,
    pub(crate) persistence: Persistence,
    /// 命令历史内存快照：历史开关页与终端共用同一份，清空/重载必须落到同一容器。
    pub(crate) history: Arc<parking_lot::RwLock<Vec<String>>>,
    /// 历史落盘存储：重新开启历史时从文件恢复。
    pub(crate) history_store: Arc<dyn transom_core::HistoryStore>,
}

/// 设置弹窗实体。
pub(crate) struct SettingsView {
    /// 弹窗的输入控件与临时状态。
    pub(crate) state: SettingsModalState,
    /// 保存与钥匙串依赖。
    pub(crate) services: SettingsServices,
    /// 打开时注入的目录快照：凭据模板页在本地编辑，变更后整体回传外壳。
    pub(crate) catalog: SessionCatalog,
    /// 弹窗内部输入控件的订阅：随实体生死，不再挂在外壳的任务容器里。
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    /// 创建设置弹窗实体。
    ///
    /// 渲染所需的字体清单、输入框与滑块都在这里一次性建立；`catalog` 是
    /// 凭据模板页的只读基线，外壳在每次打开弹窗时刷新。
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        settings: &AppSettings,
        services: SettingsServices,
        catalog: SessionCatalog,
    ) -> Self {
        let state = SettingsModalState::new(window, cx, settings);
        let mut this = Self {
            state,
            services,
            catalog,
            _subscriptions: Vec::new(),
        };
        this.install_subscriptions(window, cx);
        this
    }

    /// 弹窗是否处于打开状态。
    pub(crate) fn is_open(&self) -> bool {
        self.state.open
    }

    /// 弹窗状态只读访问。
    pub(crate) fn state(&self) -> &SettingsModalState {
        &self.state
    }

    /// 弹窗状态可变访问。
    pub(crate) fn state_mut(&mut self) -> &mut SettingsModalState {
        &mut self.state
    }

    /// 刷新凭据模板页使用的目录快照。
    pub(crate) fn set_catalog(&mut self, catalog: SessionCatalog) {
        self.catalog = catalog;
    }

    /// 把错误交给外壳的通知层。
    pub(crate) fn report_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Error(message.into()));
    }

    /// 把状态提示交给外壳的状态栏。
    pub(crate) fn report_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Status(message.into()));
    }
}
