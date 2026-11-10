//! 运行容器表单微表格组件：端口映射、数据卷挂载与环境变量的结构化输入行。
//!
//! 设计原理（Why）：
//! - 结构化微表格交互彻底替代传统松散手写的字符串，杜绝语法和分隔符格式错误；
//! - 各条目均包含直观的增删、协议与只读模式快速切换控制；
//! - 行内动作通过 [`cx.listener`] 回到实体具名方法，新增、切换与删除都只改
//!   本次打开期的状态，不再经过外壳转发。

use super::ContainerRunFormView;
use crate::container::types::RunContainerFormState;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl ContainerRunFormView {
    /// 渲染结构化端口映射区域。
    pub(super) fn render_port_mappings_section(
        &self,
        form: &RunContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = v_flex().gap_1p5().w_full();

        if form.ports.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1p5()
                    .text_xs()
                    .text_color(p.text.subtle)
                    .child(rust_i18n::t!("container.run.empty_ports_hint").to_string()),
            );
        } else {
            for (idx, item) in form.ports.iter().enumerate() {
                let port_id = item.id.clone();
                let del_port_id = port_id.clone();
                let is_udp = item.protocol == "udp";

                let row = h_flex()
                    .gap_2()
                    .items_center()
                    .w_full()
                    .child(
                        // 宿主端口输入
                        h_flex().flex_1().child(Input::new(&item.host_port).small()),
                    )
                    .child(div().text_xs().text_color(p.muted_foreground).child("➔"))
                    .child(
                        // 容器端口输入
                        h_flex()
                            .flex_1()
                            .child(Input::new(&item.container_port).small()),
                    )
                    .child(
                        // 协议切换按钮
                        Button::new(ElementId::Name(format!("toggle-proto-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .label(if is_udp { "UDP" } else { "TCP" })
                            .tooltip(if is_udp {
                                rust_i18n::t!("container.run.toggle_tcp").to_string()
                            } else {
                                rust_i18n::t!("container.run.toggle_udp").to_string()
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle_port_protocol(&port_id, cx);
                            })),
                    )
                    .child(
                        // 删除按钮
                        Button::new(ElementId::Name(format!("del-port-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .accessibility_label("Delete port mapping")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.remove_port(&del_port_id, cx);
                            })),
                    );

                list = list.child(row);
            }
        }

        v_flex()
            .gap_1p5()
            .w_full()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.run.section_ports").to_string()),
                    )
                    .child(
                        Button::new("add-port-btn")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("container.run.add_port").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_port(window, cx);
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    /// 渲染结构化数据卷挂载区域。
    pub(super) fn render_volume_mappings_section(
        &self,
        form: &RunContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = v_flex().gap_1p5().w_full();

        if form.volumes.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1p5()
                    .text_xs()
                    .text_color(p.text.subtle)
                    .child(rust_i18n::t!("container.run.empty_volumes_hint").to_string()),
            );
        } else {
            for (idx, item) in form.volumes.iter().enumerate() {
                let vol_id = item.id.clone();
                let del_vol_id = vol_id.clone();
                let read_only = item.read_only;

                let row = h_flex()
                    .gap_2()
                    .items_center()
                    .w_full()
                    .child(h_flex().flex_1().child(Input::new(&item.host_path).small()))
                    .child(div().text_xs().text_color(p.muted_foreground).child("➔"))
                    .child(
                        h_flex()
                            .flex_1()
                            .child(Input::new(&item.container_path).small()),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("toggle-ro-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .label(if read_only {
                                rust_i18n::t!("container.run.read_only").to_string()
                            } else {
                                rust_i18n::t!("container.run.read_write").to_string()
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle_volume_read_only(&vol_id, cx);
                            })),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("del-vol-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .accessibility_label("Delete volume mapping")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.remove_volume(&del_vol_id, cx);
                            })),
                    );

                list = list.child(row);
            }
        }

        v_flex()
            .gap_1p5()
            .w_full()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.run.section_volumes").to_string()),
                    )
                    .child(
                        Button::new("add-volume-btn")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("container.run.add_volume").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_volume(window, cx);
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    /// 渲染结构化环境变量区域。
    pub(super) fn render_env_vars_section(
        &self,
        form: &RunContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = v_flex().gap_1p5().w_full();

        if form.envs.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1p5()
                    .text_xs()
                    .text_color(p.text.subtle)
                    .child(rust_i18n::t!("container.run.empty_env_hint").to_string()),
            );
        } else {
            for (idx, item) in form.envs.iter().enumerate() {
                let env_id = item.id.clone();

                let row = h_flex()
                    .gap_2()
                    .items_center()
                    .w_full()
                    .child(h_flex().flex_1().child(Input::new(&item.key).small()))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.muted_foreground)
                            .child("="),
                    )
                    .child(h_flex().flex_1().child(Input::new(&item.value).small()))
                    .child(
                        Button::new(ElementId::Name(format!("del-env-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .accessibility_label("Delete environment variable")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.remove_env(&env_id, cx);
                            })),
                    );

                list = list.child(row);
            }
        }

        v_flex()
            .gap_1p5()
            .w_full()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.run.section_env").to_string()),
                    )
                    .child(
                        Button::new("add-env-btn")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("container.run.add_env").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_env(window, cx);
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
