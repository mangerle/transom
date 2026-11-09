//! 容器引擎连接表单：保存多个引擎连接，与终端会话表单并列。
//!
//! 设计原理（Why）：引擎连接是独立持久化对象（`SessionCatalog.engines`），
//! 表单只负责编辑端点与名称；"测试连接"走控制器探测，不在 UI 线程建连接。
//! 一次打开期的状态、输入控件、动作与渲染都归 [`ContainerFormView`]，
//! 外壳只保留打开入口、分组快照注入与保存/删除落目录。
//!
//! 布局参考新建会话表单（session_form），分层卡片式结构：
//! - 基本信息卡：名称、分组（同排）
//! - 外观卡：图标、环境色
//! - 连接类型卡：引擎种类（Docker/Podman）+ 端点分组（本地/远端）
//! - 动态扩展字段卡：根据端点档位追加 SSH / TLS / CLI 参数字段

use crate::app::AppShell;
use crate::app_assets::AppIcon;
use crate::container::list::create_button_label;
use crate::container::types::{
    ContainerFormState, ContainerRunForm, ContainerSection, EndpointChoice,
};
use crate::palette::Palette;
use crate::session_appearance::{
    SESSION_COLOR_CHOICES, SESSION_ICON_CHOICES, session_color, session_icon,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{
    AppCommand, ContainerCommand, ContainerEngineKind, ContainerEngineProfile, ContainerRequestId,
    EngineProfileId, FolderId, SessionColor, SessionIcon,
};

mod appearance;
mod connection;
mod create;
mod entity;
mod render;
mod run;

pub(crate) use entity::{ContainerFormEvent, ContainerFormView};
pub(crate) use render::ContainerFormFolders;

/// 重启策略档位：与引擎允许集合保持一致，留空表示 `no`。
const RUN_RESTART_POLICIES: [&str; 4] = ["no", "on-failure", "always", "unless-stopped"];

impl AppShell {
    /// 打开新建/编辑容器连接表单。
    pub(crate) fn open_container_form(
        &mut self,
        profile: Option<ContainerEngineProfile>,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.modals.tools.container_form.clone();
        view.update(cx, |view, cx| view.open(profile, folder, window, cx));
        // 弹窗栈在外壳渲染里对账，实体变脏不会自动跑外壳的 reconcile。
        cx.notify();
    }
}
