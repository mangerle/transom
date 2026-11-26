//! 会话表单中的 RDP 远程桌面连接配置渲染。
//!
//! 设计原理（Why）：仅展示已真正接入协议栈并生效的配置项（主机、端口、用户名、域、
//! 密码、色深、音频重定向与双向剪贴板），彻底杜绝未生效的假开关或填后丢弃的空壳表单。

use super::proxy::render_proxy_section;
use super::serial::serial_option_row;
use super::ssh::{number_form_field, required_form_field};
use crate::palette::Palette;
use crate::session_form::{SessionForm, SessionFormView};
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{RdpColorDepth, RdpPerformance, RdpSecurity};

/// 无必填标记的常规文本输入字段。
fn optional_form_field(label: String, input: Entity<InputState>, p: &Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(p.muted_foreground).child(label))
        .child(Input::new(&input).w_full())
        .into_any_element()
}

/// 统一的开关配置行（消除表单中的重复布局样板）。
fn rdp_switch_row(
    id: &'static str,
    label: String,
    checked: bool,
    on_toggle: impl Fn(&mut SessionForm, bool) + 'static,
    cx: &mut Context<SessionFormView>,
) -> AnyElement {
    h_flex()
        .justify_between()
        .items_center()
        .py_1()
        .child(div().text_sm().child(label))
        .child(
            Switch::new(id)
                .small()
                .checked(checked)
                .on_click(cx.listener(move |this, val: &bool, _, cx| {
                    let next = *val;
                    this.edit(cx, |form| on_toggle(form, next));
                })),
        )
        .into_any_element()
}

/// 统一的分段选择按钮组渲染器，消除色深、画质、安全协议与分辨率模式的重复代码。
fn render_segmented_picker<T: Copy + PartialEq + 'static>(
    prefix: &'static str,
    selected: T,
    options: &[(T, &str)],
    on_select: impl Fn(&mut SessionForm, T) + Copy + 'static,
    cx: &mut Context<SessionFormView>,
) -> AnyElement {
    h_flex()
        .gap_1()
        .children(options.iter().map(|&(val, label)| {
            Button::new(ElementId::Name(format!("{prefix}-{label}").into()))
                .small()
                .label(label.to_string())
                .selected(selected == val)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit(cx, |form| on_select(form, val));
                }))
        }))
        .into_any_element()
}

/// 渲染 RDP 的“基本信息”区（主机、端口、用户名、域、密码、分辨率、色深、画质、安全协议与外设开关）。
pub(crate) fn render_rdp_basic_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let pickers = RdpPickers::new(form, cx);
    let mut fields = v_flex()
        .gap_3()
        .child(render_rdp_identity_fields(form, p))
        .child(serial_option_row(
            rust_i18n::t!("form.rdp_resolution_mode").to_string(),
            pickers.resolution,
            p,
        ));
    if !form.rdp_dynamic_resolution {
        fields = fields.child(render_rdp_fixed_resolution(form, p));
    }
    fields
        .child(serial_option_row(
            rust_i18n::t!("form.rdp_color_depth").to_string(),
            pickers.color_depth,
            p,
        ))
        .child(serial_option_row(
            rust_i18n::t!("form.rdp_performance").to_string(),
            pickers.performance,
            p,
        ))
        .child(serial_option_row(
            rust_i18n::t!("form.rdp_security").to_string(),
            pickers.security,
            p,
        ))
        .child(optional_form_field(
            rust_i18n::t!("form.rdp_shared_dir").to_string(),
            form.rdp_shared_dir.clone(),
            p,
        ))
        .child(render_rdp_switches(form, cx))
        .into_any_element()
}

struct RdpPickers {
    resolution: AnyElement,
    color_depth: AnyElement,
    performance: AnyElement,
    security: AnyElement,
}

impl RdpPickers {
    fn new(form: &SessionForm, cx: &mut Context<SessionFormView>) -> Self {
        let dynamic = rust_i18n::t!("form.rdp_res_dynamic");
        let fixed = rust_i18n::t!("form.rdp_res_fixed");
        let resolution = render_segmented_picker(
            "rdp-res",
            form.rdp_dynamic_resolution,
            &[(true, &*dynamic), (false, &*fixed)],
            |form, value| form.rdp_dynamic_resolution = value,
            cx,
        );
        let color_depth = render_segmented_picker(
            "rdp-depth",
            form.rdp_color_depth,
            &[
                (RdpColorDepth::Depth16, "16-bit"),
                (RdpColorDepth::Depth24, "24-bit"),
                (RdpColorDepth::Depth32, "32-bit"),
            ],
            |form, value| form.rdp_color_depth = value,
            cx,
        );
        let lan = rust_i18n::t!("form.rdp_perf_lan");
        let broadband = rust_i18n::t!("form.rdp_perf_broadband");
        let wan = rust_i18n::t!("form.rdp_perf_wan");
        let performance = render_segmented_picker(
            "rdp-perf",
            form.rdp_performance,
            &[
                (RdpPerformance::Lan, &*lan),
                (RdpPerformance::Broadband, &*broadband),
                (RdpPerformance::Wan, &*wan),
            ],
            |form, value| form.rdp_performance = value,
            cx,
        );
        let negotiate = rust_i18n::t!("form.rdp_sec_negotiate");
        let rdp = rust_i18n::t!("form.rdp_sec_rdp");
        let security = render_segmented_picker(
            "rdp-sec",
            form.rdp_security,
            &[
                (RdpSecurity::Negotiate, &*negotiate),
                (RdpSecurity::Nla, "NLA"),
                (RdpSecurity::Tls, "TLS"),
                (RdpSecurity::Rdp, &*rdp),
            ],
            |form, value| form.rdp_security = value,
            cx,
        );
        Self {
            resolution,
            color_depth,
            performance,
            security,
        }
    }
}

fn render_rdp_identity_fields(form: &SessionForm, p: &Palette) -> AnyElement {
    v_flex()
        .gap_3()
        .child(
            h_flex()
                .gap_3()
                .items_end()
                .child(div().flex_1().child(required_form_field(
                    rust_i18n::t!("form.host").to_string(),
                    form.host.clone(),
                    p,
                )))
                .child(div().w(rems(8.5)).child(number_form_field(
                    rust_i18n::t!("form.port").to_string(),
                    true,
                    form.port.clone(),
                    p,
                ))),
        )
        .child(
            h_flex()
                .gap_3()
                .items_end()
                .child(div().flex_1().child(optional_form_field(
                    rust_i18n::t!("form.username").to_string(),
                    form.username.clone(),
                    p,
                )))
                .child(div().flex_1().child(optional_form_field(
                    rust_i18n::t!("form.rdp_domain").to_string(),
                    form.rdp_domain.clone(),
                    p,
                ))),
        )
        .child(optional_form_field(
            if form.base.is_some() {
                rust_i18n::t!("form.password_keep_saved").to_string()
            } else {
                rust_i18n::t!("form.password").to_string()
            },
            form.password.clone(),
            p,
        ))
        .into_any_element()
}

fn render_rdp_fixed_resolution(form: &SessionForm, p: &Palette) -> AnyElement {
    h_flex()
        .gap_3()
        .items_end()
        .child(div().flex_1().child(number_form_field(
            rust_i18n::t!("form.rdp_fixed_width").to_string(),
            true,
            form.rdp_width.clone(),
            p,
        )))
        .child(div().flex_1().child(number_form_field(
            rust_i18n::t!("form.rdp_fixed_height").to_string(),
            true,
            form.rdp_height.clone(),
            p,
        )))
        .into_any_element()
}

fn render_rdp_switches(form: &SessionForm, cx: &mut Context<SessionFormView>) -> AnyElement {
    v_flex()
        .gap_3()
        .child(rdp_switch_row(
            "toggle-rdp-audio",
            rust_i18n::t!("form.rdp_audio").to_string(),
            form.rdp_audio,
            |form, value| form.rdp_audio = value,
            cx,
        ))
        .child(rdp_switch_row(
            "toggle-rdp-clipboard",
            rust_i18n::t!("form.rdp_clipboard").to_string(),
            form.rdp_clipboard,
            |form, value| form.rdp_clipboard = value,
            cx,
        ))
        .into_any_element()
}

/// 渲染 RDP 的“网络与代理”区（支持经由 SSH 跳板机隧道转发 3389）。
pub(crate) fn render_rdp_network_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_3()
        .child(render_proxy_section(form, cx, p))
        .into_any_element()
}
