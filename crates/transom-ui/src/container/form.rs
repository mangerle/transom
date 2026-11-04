//! 容器引擎连接表单：保存多个引擎连接，与终端会话表单并列。
//!
//! 设计原理（Why）：引擎连接是独立持久化对象（`SessionCatalog.engines`），
//! 表单只负责编辑端点与名称；"测试连接"走控制器探测，不在 UI 线程建连接。
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
    ContainerCommand, ContainerEngineKind, ContainerRequestId, SessionColor, SessionIcon,
};

mod appearance;
mod connection;
mod create;
mod engine_actions;
mod render;
mod run;

/// 重启策略档位：与引擎允许集合保持一致，留空表示 `no`。
const RUN_RESTART_POLICIES: [&str; 4] = ["no", "on-failure", "always", "unless-stopped"];
impl AppShell {
    /// 打开新建/编辑容器连接表单。
    pub(crate) fn open_container_form(
        &mut self,
        profile: Option<transom_core::ContainerEngineProfile>,
        folder: Option<transom_core::FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = ContainerFormState::new(window, cx, profile.as_ref(), folder);
        self.state.containers.form = Some(form);
        cx.notify();
    }

    /// 容器连接表单的弹窗体；外观与关闭由 Dialog 容器负责。
    pub(crate) fn render_container_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.form.as_ref() else {
            return div().into_any_element();
        };
        let title = if form.editing.is_some() {
            rust_i18n::t!("container.form.edit_title").to_string()
        } else {
            rust_i18n::t!("container.form.new_title").to_string()
        };

        // 基本信息卡：名称 + 分组（同排）
        let basic_card = self.render_engine_form_basic_card(app, form, p);
        // 外观卡：图标 + 环境色
        let appearance_card = self.render_engine_form_appearance_card(app, form, p);
        // 连接类型卡：引擎种类 + 端点分组
        let connection_card = self.render_engine_form_connection_card(app, form, p);

        let mut content = v_flex()
            .id(ElementId::Name("engine-form-body".into()))
            .max_h(px(520.0))
            .overflow_y_scroll()
            .gap_3()
            .child(basic_card)
            .child(appearance_card)
            .child(connection_card);

        // 动态扩展字段：根据端点档位追加
        if let Some(extra) = self.render_engine_form_extra_fields(app, form, p) {
            content = content.child(extra);
        }

        // 测试连接状态提示
        if let Some(status) = &form.test_status {
            content = content.child(
                div()
                    .px_1()
                    .text_sm()
                    .text_color(if form.test_ok { p.primary } else { p.danger })
                    .child(status.clone()),
            );
        }

        let footer = self.render_engine_form_footer(app, form, p);

        crate::widgets::dialog_body("container-form-body", title, None, content, Some(footer))
    }
}
