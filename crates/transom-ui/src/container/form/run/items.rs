//! 运行容器表单微表格组件：端口映射、数据卷挂载与环境变量的结构化输入行。
//!
//! 设计原理（Why）：
//! - 结构化微表格交互彻底替代传统松散手写的字符串，杜绝语法和分隔符格式错误；
//! - 各条目均包含直观的增删、协议与只读模式快速切换控制。

use crate::app::AppShell;
use crate::container::types::run_form::{EnvVarItem, PortMappingItem, VolumeMappingItem};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use gpui_kit::*;

impl AppShell {
    /// 渲染结构化端口映射区域。
    pub(super) fn render_port_mappings_section(
        &self,
        app: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return div().into_any_element();
        };

        let mut list = v_flex().gap_1p5().w_full();

        if form.ports.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1p5()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.8))
                    .child(rust_i18n::t!("container.run.empty_ports_hint").to_string()),
            );
        } else {
            for (idx, item) in form.ports.iter().enumerate() {
                let port_id = item.id.clone();
                let del_port_id = port_id.clone();
                let protocol = item.protocol.clone();
                let is_udp = protocol == "udp";

                let row = h_flex()
                    .gap_2()
                    .items_center()
                    .w_full()
                    .child(
                        // 宿主端口输入
                        h_flex()
                            .flex_1()
                            .child(Input::new(&item.host_port).small()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child("➔"),
                    )
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        if let Some(target) = f.ports.iter_mut().find(|p| p.id == port_id) {
                                            target.protocol = if is_udp { "tcp".to_string() } else { "udp".to_string() };
                                            cx.notify();
                                        }
                                    }
                                },
                            )),
                    )
                    .child(
                        // 删除按钮
                        Button::new(ElementId::Name(format!("del-port-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        f.ports.retain(|p| p.id != del_port_id);
                                        cx.notify();
                                    }
                                },
                            )),
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, window, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        f.ports.push(PortMappingItem::new(None, None, "tcp", window, cx));
                                        cx.notify();
                                    }
                                },
                            )),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    /// 渲染结构化数据卷挂载区域。
    pub(super) fn render_volume_mappings_section(
        &self,
        app: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return div().into_any_element();
        };

        let mut list = v_flex().gap_1p5().w_full();

        if form.volumes.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1p5()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.8))
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
                    .child(
                        h_flex()
                            .flex_1()
                            .child(Input::new(&item.host_path).small()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child("➔"),
                    )
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        if let Some(target) = f.volumes.iter_mut().find(|v| v.id == vol_id) {
                                            target.read_only = !read_only;
                                            cx.notify();
                                        }
                                    }
                                },
                            )),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("del-vol-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        f.volumes.retain(|v| v.id != del_vol_id);
                                        cx.notify();
                                    }
                                },
                            )),
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, window, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        f.volumes.push(VolumeMappingItem::new(None, None, false, window, cx));
                                        cx.notify();
                                    }
                                },
                            )),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    /// 渲染结构化环境变量区域。
    pub(super) fn render_env_vars_section(
        &self,
        app: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return div().into_any_element();
        };

        let mut list = v_flex().gap_1p5().w_full();

        if form.envs.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1p5()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.8))
                    .child(rust_i18n::t!("container.run.empty_env_hint").to_string()),
            );
        } else {
            for (idx, item) in form.envs.iter().enumerate() {
                let env_id = item.id.clone();

                let row = h_flex()
                    .gap_2()
                    .items_center()
                    .w_full()
                    .child(
                        h_flex()
                            .flex_1()
                            .child(Input::new(&item.key).small()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.muted_foreground)
                            .child("="),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .child(Input::new(&item.value).small()),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("del-env-{idx}").into()))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Trash)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        f.envs.retain(|e| e.id != env_id);
                                        cx.notify();
                                    }
                                },
                            )),
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, window, cx| {
                                    if let Some(f) = this.state.containers.run_form.as_mut() {
                                        f.envs.push(EnvVarItem::new(None, None, window, cx));
                                        cx.notify();
                                    }
                                },
                            )),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
