//! 运行容器表单双栏整体布局与左侧核心配置区。
//!
//! 设计原理（Why）：
//! - 左侧 65% 聚焦结构化配置输入，右侧 35% 实时反馈 CLI 命令与镜像看板；
//! - 基础配置与高级选项直观平铺展开，兼顾清晰易懂与深度定制。

use crate::app::AppShell;
use crate::container::form::RUN_RESTART_POLICIES;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Icon, Selectable, Sizable};
use gpui_kit::*;

impl AppShell {
    /// 渲染运行容器表单整体弹窗体。
    pub(crate) fn render_container_run_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return div().into_any_element();
        };

        let tab_id = form.tab_id;
        let restart = form.restart.clone();
        let error = form.error.clone();

        // 左侧核心配置区 (65%)
        let mut left_panel =
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                // 1. 基本信息
                .child(
                    v_flex()
                        .gap_1p5()
                        .w_full()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("container.run.section_basic").to_string()),
                        )
                        .child(crate::widgets::form_field(
                            rust_i18n::t!("container.run.image").to_string(),
                            form.image.clone(),
                            p,
                        ))
                        .child(crate::widgets::form_field(
                            rust_i18n::t!("container.run.name").to_string(),
                            form.name.clone(),
                            p,
                        )),
                )
                // 2. 结构化端口映射
                .child(self.render_port_mappings_section(app, p))
                // 3. 结构化数据卷挂载
                .child(self.render_volume_mappings_section(app, p))
                // 4. 高级选项（直接展开平铺呈现）
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("container.run.advanced_title").to_string()),
                )
                // 结构化环境变量
                .child(self.render_env_vars_section(app, p))
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
                .child(h_flex().gap_3().flex_wrap().children([
                    run_toggle(
                        app,
                        "container-run-auto-remove",
                        rust_i18n::t!("container.run.auto_remove").to_string(),
                        form.auto_remove,
                        |form, checked| form.auto_remove = checked,
                        p,
                    ),
                    run_toggle(
                        app,
                        "container-run-read-only",
                        rust_i18n::t!("container.run.read_only_rootfs").to_string(),
                        form.read_only_rootfs,
                        |form, checked| form.read_only_rootfs = checked,
                        p,
                    ),
                    run_toggle(
                        app,
                        "container-run-privileged",
                        rust_i18n::t!("container.run.privileged").to_string(),
                        form.privileged,
                        |form, checked| form.privileged = checked,
                        p,
                    ),
                    run_toggle(
                        app,
                        "container-run-init",
                        rust_i18n::t!("container.run.init").to_string(),
                        form.init,
                        |form, checked| form.init = checked,
                        p,
                    ),
                ]))
                // 重启策略胶囊组
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.run.restart").to_string()),
                        )
                        .child(h_flex().gap_1().children(RUN_RESTART_POLICIES.iter().map(
                            |policy| {
                                Button::new(ElementId::Name(
                                    format!("container-run-restart-{policy}").into(),
                                ))
                                .xsmall()
                                .ghost()
                                .selected(restart.as_deref() == Some(*policy))
                                .label((*policy).to_string())
                                .on_click(
                                    crate::dialog_host::on_event(
                                        app,
                                        move |this, _: &ClickEvent, _, cx| {
                                            let Some(form) =
                                                this.state.containers.run_form.as_mut()
                                            else {
                                                return;
                                            };
                                            form.restart =
                                                if form.restart.as_deref() == Some(*policy) {
                                                    None
                                                } else {
                                                    Some((*policy).to_string())
                                                };
                                            cx.notify();
                                        },
                                    ),
                                )
                            },
                        ))),
                );

        if let Some(err) = error {
            left_panel = left_panel.child(
                h_flex()
                    .gap_1p5()
                    .p_2()
                    .rounded(p.radius.md)
                    .bg(p.danger.opacity(0.12))
                    .items_center()
                    .child(
                        Icon::new(IconName::CircleAlert)
                            .with_size(px(14.0))
                            .text_color(p.danger),
                    )
                    .child(div().text_xs().text_color(p.danger).child(err)),
            );
        }

        // 双栏主体容器
        let dual_columns = h_flex()
            .w_full()
            .gap_4()
            .items_start()
            .child(left_panel)
            .child(self.render_run_preview_panel(app, tab_id, p, cx));

        let cancel_btn = Button::new("container-run-cancel")
            .ghost()
            .label(rust_i18n::t!("common.cancel").to_string())
            .on_click(crate::dialog_host::on_event(
                app,
                |this, _: &ClickEvent, _, cx| this.dismiss_container_run(cx),
            ))
            .into_any_element();

        let is_recreate = form.recreate_container_id.is_some();
        let submit_label = if is_recreate {
            rust_i18n::t!("container.run.recreate_submit").to_string()
        } else {
            rust_i18n::t!("container.run.submit").to_string()
        };
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

        let submit_btn = Button::new("container-run-submit")
            .primary()
            .icon(IconName::Play)
            .label(submit_label)
            .on_click(crate::dialog_host::on_event(
                app,
                move |this, _: &ClickEvent, _, cx| {
                    this.submit_container_run(tab_id, cx);
                },
            ))
            .into_any_element();

        let footer = crate::widgets::dialog_actions([cancel_btn, submit_btn]);

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
            Some(footer),
        )
    }
}

/// 运行表单的布尔开关行。
fn run_toggle(
    app: &Entity<AppShell>,
    id: &'static str,
    label: String,
    checked: bool,
    set: fn(&mut crate::container::types::RunContainerFormState, bool),
    p: &Palette,
) -> AnyElement {
    h_flex()
        .gap_1p5()
        .items_center()
        .child(
            Checkbox::new(id)
                .small()
                .checked(checked)
                .on_click(crate::dialog_host::on_event(
                    app,
                    move |this, &next: &bool, _, cx| {
                        if let Some(form) = this.state.containers.run_form.as_mut() {
                            set(form, next);
                        }
                        cx.notify();
                    },
                )),
        )
        .child(div().text_xs().text_color(p.muted_foreground).child(label))
        .into_any_element()
}
