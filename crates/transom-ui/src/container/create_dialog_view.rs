//! 拉取镜像 / 新建卷与网络的弹窗实体。
//!
//! # 设计原理（Why）
//! 弹窗此前由 `ContainerUiState.create_dialog` 与 `impl AppShell` 的打开、提交、
//! 渲染三处方法共同维护，一次打开期的输入控件散落在外壳状态里。实体化后，
//! 附加输入（驱动、子网、网关、Dockerfile、tag 与 `internal` 开关）与弹窗同生
//! 共死，外壳只持有句柄；按钮与开关回调也只重绘本实体。
//!
//! 主输入值仍归各标签页工作区的 `create_input`：它同时承担分类切换时的清空与
//! 列表工具条的占位提示，搬进弹窗会让这两条既有路径失去归属。因此实体只读
//! 附加输入，提交时上交 [`ContainerCreateDialogEvent::Submit`]，由外壳补齐主
//! 输入、校验空值、投递命令并在成功后关闭弹窗。
//!
//! # 代价与局限
//! 实体无法判断主输入是否为空，提交后必须等外壳回写错误；因此 `submit` 不
//! 关闭弹窗，关闭动作由外壳在命令投递成功后显式执行。事件只携带附加字段，
//! 外壳处理时禁止回读实体，避免实体派发期间的双重租借。

use super::*;
use crate::container::list::create_button_label;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::*;

/// 拉取/新建弹窗交给外壳处理的提交意图。
///
/// 事件自带全部附加字段：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。主输入值不在事件里，由外壳从目标工作区读取。
pub(crate) enum ContainerCreateDialogEvent {
    /// 提交本次创建；实体保持打开，由外壳校验主输入后决定关闭或回写错误。
    Submit {
        /// 目标容器工作区标签。
        tab_id: TabId,
        /// 当前分类，决定命令种类与字段含义。
        section: ContainerSection,
        /// 驱动输入；镜像拉取为 `None`。
        driver: Option<String>,
        /// 子网 CIDR；仅网络有值。
        subnet: Option<String>,
        /// 网关地址；仅网络有值。
        gateway: Option<String>,
        /// Dockerfile 相对路径；仅镜像构建有值。
        dockerfile: Option<String>,
        /// 镜像标签；仅镜像构建有值。
        tag: Option<String>,
        /// 仅宿主机内部可见（`--internal`）。
        internal: bool,
    },
}

impl EventEmitter<ContainerCreateDialogEvent> for ContainerCreateDialogView {}

/// 拉取镜像 / 新建卷与网络的弹窗实体。
pub(crate) struct ContainerCreateDialogView {
    /// 未打开时为 `None`；每次打开都重建附加输入，避免残留上次填写内容。
    state: Option<ContainerCreateDialog>,
}

impl ContainerCreateDialogView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new() -> Self {
        Self { state: None }
    }

    /// 弹窗是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染、脏判定与焦点定位共用同一入口。
    pub(crate) fn state(&self) -> Option<&ContainerCreateDialog> {
        self.state.as_ref()
    }

    /// 打开弹窗：镜像只有一个引用输入，卷与网络多一个驱动输入，网络再加
    /// 子网、网关与「仅内部访问」；Dockerfile 与 tag 只用于镜像构建。
    pub(crate) fn open(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let driver = matches!(
            section,
            ContainerSection::Volumes | ContainerSection::Networks
        )
        .then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.driver_placeholder").to_string(),
                )
            })
        });
        let network = section == ContainerSection::Networks;
        let subnet = network.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.subnet_placeholder").to_string(),
                )
            })
        });
        let gateway = network.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.gateway_placeholder").to_string(),
                )
            })
        });
        let is_image = section == ContainerSection::Images;
        let dockerfile = is_image.then(|| {
            cx.new(|cx| InputState::new(window, cx).placeholder("Dockerfile".to_string()))
        });
        let tag = is_image.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.tag_placeholder").to_string(),
                )
            })
        });
        self.state = Some(ContainerCreateDialog {
            tab_id,
            section,
            driver,
            subnet,
            gateway,
            dockerfile,
            tag,
            internal: false,
            error: None,
        });
        cx.notify();
    }

    /// 关闭弹窗并丢弃本次打开期的附加输入控件。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.state = None;
        cx.notify();
    }

    /// 把外壳校验失败的原因写回弹窗；弹窗保持打开，等待用户就地改正。
    pub(crate) fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.state.as_mut() {
            dialog.error = Some(message);
        }
        cx.notify();
    }

    /// 切换「仅内部访问」；关闭状态下忽略。
    fn set_internal(&mut self, internal: bool, cx: &mut Context<Self>) {
        if let Some(dialog) = self.state.as_mut() {
            dialog.internal = internal;
        }
        cx.notify();
    }

    /// 附加输入的当前值，转发给弹窗脏判定；主输入由外壳另行补入。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        let Some(dialog) = self.state.as_ref() else {
            return Vec::new();
        };
        let mut values = Vec::with_capacity(3);
        for input in [&dialog.driver, &dialog.subnet, &dialog.gateway]
            .into_iter()
            .flatten()
        {
            values.push(input.read(cx).value().to_string());
        }
        values
    }

    /// 读取附加输入并上交提交意图；不关闭弹窗。
    ///
    /// 主输入为空、网络参数非法等校验都归外壳：实体读不到工作区的
    /// `create_input`，先关闭会让校验失败时丢失整次填写。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.state.as_ref() else {
            return;
        };
        let event = ContainerCreateDialogEvent::Submit {
            tab_id: dialog.tab_id,
            section: dialog.section,
            driver: read_input(dialog.driver.as_ref(), cx),
            subnet: read_input(dialog.subnet.as_ref(), cx),
            gateway: read_input(dialog.gateway.as_ref(), cx),
            dockerfile: read_input(dialog.dockerfile.as_ref(), cx),
            tag: read_input(dialog.tag.as_ref(), cx),
            internal: dialog.internal,
        };
        if let Some(dialog) = self.state.as_mut() {
            dialog.error = None;
        }
        cx.emit(event);
        cx.notify();
    }

    /// 弹窗主体：主输入由外壳注入，附加字段按分类展开。
    pub(crate) fn render_body(
        &self,
        create_input: Entity<InputState>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(dialog) = self.state.as_ref() else {
            return div().into_any_element();
        };
        let title = create_button_label(dialog.section)
            .unwrap_or_else(|| rust_i18n::t!("container.create").to_string());
        let content = self.render_content(dialog, create_input, p, cx);
        let footer = self.render_footer(dialog, &title, cx);
        crate::widgets::dialog_body("container-create-body", title, None, content, Some(footer))
    }

    /// 内容区：主输入、分类附加字段、`internal` 开关与错误提示。
    fn render_content(
        &self,
        dialog: &ContainerCreateDialog,
        create_input: Entity<InputState>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name_label = if dialog.section == ContainerSection::Images {
            rust_i18n::t!("container.field.context_path").to_string()
        } else {
            rust_i18n::t!("container.field.name").to_string()
        };
        let mut content =
            v_flex()
                .gap_3()
                .child(crate::widgets::form_field(name_label, create_input, p));
        if let Some(dockerfile) = dialog.dockerfile.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.dockerfile").to_string(),
                dockerfile,
                p,
            ));
        }
        if let Some(tag) = dialog.tag.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.tag").to_string(),
                tag,
                p,
            ));
        }
        if let Some(driver) = dialog.driver.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.driver").to_string(),
                driver,
                p,
            ));
        }
        if let Some(subnet) = dialog.subnet.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.subnet").to_string(),
                subnet,
                p,
            ));
        }
        if let Some(gateway) = dialog.gateway.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.gateway").to_string(),
                gateway,
                p,
            ));
        }
        if dialog.section == ContainerSection::Networks {
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("container-create-internal")
                            .small()
                            .checked(dialog.internal)
                            .on_click(
                                cx.listener(|this, &checked, _, cx| this.set_internal(checked, cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("container.create_dialog.internal").to_string()),
                    ),
            );
        }
        if let Some(error) = &dialog.error {
            content = content.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        content.into_any_element()
    }

    /// 页脚：取消只关闭弹窗，提交交由外壳校验并投递命令。
    fn render_footer(
        &self,
        dialog: &ContainerCreateDialog,
        title: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("container-create-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx))),
            )
            .child(
                Button::new("container-create-submit")
                    .primary()
                    .icon(if dialog.section == ContainerSection::Images {
                        IconName::Plus
                    } else {
                        IconName::Check
                    })
                    .label(title.to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx))),
            )
            .into_any_element()
    }
}

/// 读取一个可选附加输入；空白值按未填写处理。
fn read_input(input: Option<&Entity<InputState>>, cx: &App) -> Option<String> {
    input
        .map(|input| input.read(cx).value().trim().to_string())
        .filter(|value| !value.is_empty())
}
