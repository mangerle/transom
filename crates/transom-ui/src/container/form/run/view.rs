//! 运行容器表单双栏整体布局与左侧核心配置区。
//!
//! 设计原理（Why）：
//! - 左侧 65% 聚焦结构化配置输入，右侧 35% 实时反馈 CLI 命令与镜像看板；
//! - 基础配置与高级选项直观平铺展开，兼顾清晰易懂与深度定制；
//! - 渲染与字段动作都归 [`ContainerRunFormView`]，回调只经具名状态方法修改
//!   本次打开期的表单，不再回交外壳。

use super::ContainerRunFormView;
use crate::container::form::RUN_RESTART_POLICIES;
use crate::container::types::RunContainerFormState;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Icon, Selectable, Sizable};
use gpui_kit::*;
use transom_core::ContainerImageSummary;

impl ContainerRunFormView {
    /// 渲染运行容器表单整体弹窗体；镜像摘要由弹窗宿主在只读外壳里快照注入。
    pub(crate) fn render_body(
        &self,
        image_meta: Option<&ContainerImageSummary>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(form) = self.state() else {
            return div().into_any_element();
        };

        let mut left_panel = v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .child(self.render_basic_card(form, p))
            .child(self.render_port_mappings_section(form, p, cx))
            .child(self.render_volume_mappings_section(form, p, cx))
            .child(self.render_advanced_fields(form, p, cx));

        if let Some(error) = form.error.clone() {
            left_panel = left_panel.child(run_error_banner(error, p));
        }

        // 双栏主体容器
        let dual_columns = h_flex()
            .w_full()
            .gap_4()
            .items_start()
            .child(left_panel)
            .child(self.render_run_preview_panel(form, image_meta, p, cx));

        let is_recreate = form.recreate_container_id.is_some();
        let dialog_title = if is_recreate {
            rust_i18n::t!("container.run.recreate_title").to_string()
        } else {
            rust_i18n::t!("container.run.title").to_string()
        };
        let dialog_desc = if is_recreate {
            rust_i18n::t!("container.run.recreate_desc").to_string()
        } else {
            rust_i18n::t!("container.run.desc").to_string()
        };

        let body = div()
            .id("container-run-body")
            .max_h(px(520.0))
            .overflow_y_scroll()
            .child(dual_columns);

        crate::widgets::dialog_body(
            "container-run-form-body",
            dialog_title,
            Some(dialog_desc),
            body,
            Some(self.render_footer(is_recreate, cx)),
        )
    }

    /// 基本信息卡：镜像与容器名。
    fn render_basic_card(&self, form: &RunContainerFormState, p: &Palette) -> AnyElement {
        v_flex()
            .gap_1p5()
            .w_full()
            .child(section_title(
                rust_i18n::t!("container.run.section_basic").to_string(),
                p,
            ))
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.run.image").to_string(),
                form.image.clone(),
                p,
            ))
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.run.name").to_string(),
                form.name.clone(),
                p,
            ))
            .into_any_element()
    }

    /// 高级选项区：环境变量、启动命令、资源限额、网络身份与安全开关。
    fn render_advanced_fields(
        &self,
        form: &RunContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("container.run.advanced_title").to_string()),
            )
            // 结构化环境变量
            .child(self.render_env_vars_section(form, p, cx))
            // 自定义启动命令
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.run.command").to_string(),
                form.command.clone(),
                p,
            ))
            // 资源限额
            .child(
                h_flex()
                    .gap_2()
                    .w_full()
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.memory").to_string(),
                        form.memory.clone(),
                        p,
                    )))
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.cpus").to_string(),
                        form.cpus.clone(),
                        p,
                    ))),
            )
            // 网络、身份与安全选项
            .child(
                h_flex()
                    .gap_2()
                    .w_full()
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.network_mode").to_string(),
                        form.network_mode.clone(),
                        p,
                    )))
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.hostname").to_string(),
                        form.hostname.clone(),
                        p,
                    ))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .w_full()
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.user").to_string(),
                        form.user.clone(),
                        p,
                    )))
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.working_dir").to_string(),
                        form.working_dir.clone(),
                        p,
                    ))),
            )
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.run.labels").to_string(),
                form.labels.clone(),
                p,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .w_full()
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.cap_add").to_string(),
                        form.cap_add.clone(),
                        p,
                    )))
                    .child(h_flex().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.run.cap_drop").to_string(),
                        form.cap_drop.clone(),
                        p,
                    ))),
            )
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.run.dns").to_string(),
                form.dns.clone(),
                p,
            ))
            .child(self.render_run_toggles(form, p, cx))
            .child(self.render_restart_picker(form, p, cx))
            .into_any_element()
    }

    /// 布尔开关行：自动移除、只读根文件系统、特权模式与 init。
    fn render_run_toggles(
        &self,
        form: &RunContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_3()
            .flex_wrap()
            .children([
                run_toggle(
                    cx,
                    "container-run-auto-remove",
                    rust_i18n::t!("container.run.auto_remove").to_string(),
                    form.auto_remove,
                    Self::set_auto_remove,
                    p,
                ),
                run_toggle(
                    cx,
                    "container-run-read-only",
                    rust_i18n::t!("container.run.read_only_rootfs").to_string(),
                    form.read_only_rootfs,
                    Self::set_read_only_rootfs,
                    p,
                ),
                run_toggle(
                    cx,
                    "container-run-privileged",
                    rust_i18n::t!("container.run.privileged").to_string(),
                    form.privileged,
                    Self::set_privileged,
                    p,
                ),
                run_toggle(
                    cx,
                    "container-run-init",
                    rust_i18n::t!("container.run.init").to_string(),
                    form.init,
                    Self::set_init,
                    p,
                ),
            ])
            .into_any_element()
    }

    /// 重启策略胶囊组；再次点击当前档位表示留空。
    fn render_restart_picker(
        &self,
        form: &RunContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let restart = form.restart.as_deref();
        let mut buttons = Vec::with_capacity(RUN_RESTART_POLICIES.len());
        for policy in RUN_RESTART_POLICIES {
            buttons.push(
                Button::new(ElementId::Name(
                    format!("container-run-restart-{policy}").into(),
                ))
                .xsmall()
                .ghost()
                .selected(restart == Some(policy))
                .label(policy.to_string())
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_restart(policy, cx)),
                )
                .into_any_element(),
            );
        }

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.run.restart").to_string()),
            )
            .child(h_flex().gap_1().children(buttons))
            .into_any_element()
    }

    /// 页脚：取消与提交；重建态使用专用提交文案。
    fn render_footer(&self, is_recreate: bool, cx: &mut Context<Self>) -> AnyElement {
        let cancel_btn = Button::new("container-run-cancel")
            .ghost()
            .label(rust_i18n::t!("common.cancel").to_string())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
            .into_any_element();

        let submit_label = if is_recreate {
            rust_i18n::t!("container.run.recreate_submit").to_string()
        } else {
            rust_i18n::t!("container.run.submit").to_string()
        };
        let submit_btn = Button::new("container-run-submit")
            .primary()
            .icon(IconName::Play)
            .label(submit_label)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx)))
            .into_any_element();

        crate::widgets::dialog_actions([cancel_btn, submit_btn])
    }
}

/// 表单区段标题。
fn section_title(label: String, p: &Palette) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.foreground)
        .child(label)
        .into_any_element()
}

/// 校验失败提示条。
fn run_error_banner(error: String, p: &Palette) -> AnyElement {
    h_flex()
        .gap_1p5()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.tone.danger_subtle)
        .items_center()
        .child(
            Icon::new(IconName::CircleAlert)
                .with_size(px(14.0))
                .text_color(p.danger),
        )
        .child(div().text_xs().text_color(p.danger).child(error))
        .into_any_element()
}

/// 运行表单的布尔开关行。
fn run_toggle(
    cx: &mut Context<ContainerRunFormView>,
    id: &'static str,
    label: String,
    checked: bool,
    set: fn(&mut ContainerRunFormView, bool, &mut Context<ContainerRunFormView>),
    p: &Palette,
) -> AnyElement {
    h_flex()
        .gap_1p5()
        .items_center()
        .child(
            Checkbox::new(id)
                .small()
                .checked(checked)
                .on_click(cx.listener(move |this, &next: &bool, _, cx| set(this, next, cx))),
        )
        .child(div().text_xs().text_color(p.muted_foreground).child(label))
        .into_any_element()
}
