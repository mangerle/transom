//! 设置中心的"关于"页：只读呈现版本、许可与本机运行环境。
//!
//! 设计原理（Why）：
//! - 实现初衷：用户报障时最先要问的就是版本、发行版与配置目录三件事，与其在
//!   issue 模板里反复索要截图，不如在应用内一次给全，并把"打开主页/打开目录"
//!   做成可点入口，便于直接附上现场文件；
//! - 核心优势：信息全部来自构建期常量与只读系统调用，本页没有任何可写状态，
//!   因此不参与设置弹窗的保存/取消回滚语义，也不新增设置项；
//! - 代价与局限：发行版名读 `/etc/os-release`（仅 Linux 有效），其余平台退回
//!   `std::env::consts::OS`；rustc 版本与 git 提交号无法显示，因为工程刻意没有
//!   build.rs 注入构建元数据，避免每次改动都触发全量重编。

use crate::app::AppShell;
use crate::brand;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;
use std::sync::OnceLock;

/// 关于页品牌标志边长（rem），随字号密度档等比缩放。
const LOGO_SIZE: f32 = 3.0;

/// 本机运行环境标识：`发行版 · 架构`。
///
/// 渲染每帧都会走到这里，故文件读取只做一次并缓存；读不到（非 Linux 或文件
/// 缺失）退回 OS 族名——宁可粗略也不留空白，且绝不因为一个可选信息报错。
fn platform_label() -> &'static str {
    static LABEL: OnceLock<String> = OnceLock::new();
    LABEL
        .get_or_init(|| {
            let distro = std::fs::read_to_string("/etc/os-release")
                .ok()
                .map(|content| transom_core::protocol::system_stats::parse_os_name(&content))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| std::env::consts::OS.to_string());
            format!("{distro} · {}", std::env::consts::ARCH)
        })
        .as_str()
}

/// 只读值行：标题走 i18n，值本身是构建期常量或系统信息，不参与本地化。
fn value_row(title_key: &'static str, value: String, p: &Palette) -> AnyElement {
    AppShell::setting_row(
        title_key,
        None,
        div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(value)
            .into_any_element(),
        p,
    )
}

/// 值 + 打开按钮配置参数。
struct OpenableRowProps<F> {
    title_key: &'static str,
    description_key: Option<&'static str>,
    value: String,
    button_id: &'static str,
    button_key: &'static str,
    on_open: F,
}

/// 值 + 打开按钮行：主页地址与配置目录共用（两者都是"看一眼 + 可能要点开"）。
fn openable_row(
    props: OpenableRowProps<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    p: &Palette,
) -> AnyElement {
    let OpenableRowProps {
        title_key,
        description_key,
        value,
        button_id,
        button_key,
        on_open,
    } = props;
    AppShell::setting_row(
        title_key,
        description_key,
        h_flex()
            .gap_2()
            .items_center()
            .child(AppShell::setting_path_value(button_id, value, p))
            .child(
                Button::new(button_id)
                    .small()
                    .label(rust_i18n::t!(button_key).to_string())
                    .on_click(on_open),
            )
            .into_any_element(),
        p,
    )
}

impl AppShell {
    /// 渲染"关于"分组：标志与名称头部，外加版本、构建、许可、作者、主页、
    /// 配置目录与运行环境各行。
    ///
    /// 需要 `window`：品牌标志要按窗口 DPI 光栅化（见 `crate::brand`），拿不到
    /// 窗口就只能按 SVG 固有尺寸出图，再被 GPU 缩小采样成颗粒。
    pub(crate) fn render_settings_about(
        &mut self,
        window: &mut Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 主页地址是编译期常量，不接受任何外部输入，因此不存在被诱导打开
        // 任意目标的问题。
        let open_home =
            cx.listener(|_, _, _, _| crate::audit::open_web_page(crate::APP_REPOSITORY));
        let config_dir = transom_platform::paths::config_dir();
        let config_path_label = config_dir.display().to_string();
        let open_config_dir = cx.listener(move |this, _, _, cx| {
            crate::audit::reveal_in_file_manager(config_dir.clone());
            this.status = Some(rust_i18n::t!("settings.about_dir_revealed").to_string());
            cx.notify();
        });
        let build_label = if cfg!(debug_assertions) {
            rust_i18n::t!("settings.about_build_debug").to_string()
        } else {
            rust_i18n::t!("settings.about_build_release").to_string()
        };

        let header = h_flex()
            .gap_4()
            .items_center()
            .pb_2()
            .child(brand::logo_mark(LOGO_SIZE, window, cx))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .child(format!("{} v{}", crate::APP_NAME, crate::APP_VERSION)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("settings.about_summary").to_string()),
                    ),
            )
            .into_any_element();

        v_flex()
            .gap_2()
            .child(header)
            .child(value_row(
                "settings.about_version",
                format!("v{}", crate::APP_VERSION),
                p,
            ))
            .child(value_row("settings.about_build", build_label, p))
            .child(value_row(
                "settings.about_license",
                crate::APP_LICENSE.into(),
                p,
            ))
            .child(value_row(
                "settings.about_author",
                crate::APP_AUTHORS.into(),
                p,
            ))
            .child(value_row(
                "settings.about_platform",
                platform_label().into(),
                p,
            ))
            .child(openable_row(
                OpenableRowProps {
                    title_key: "settings.about_homepage",
                    description_key: None,
                    value: crate::APP_REPOSITORY.into(),
                    button_id: "about-open-homepage",
                    button_key: "settings.about_homepage_open",
                    on_open: open_home,
                },
                p,
            ))
            .child(openable_row(
                OpenableRowProps {
                    title_key: "settings.about_config_dir",
                    description_key: Some("settings.about_config_dir_desc"),
                    value: config_path_label,
                    button_id: "about-open-config-dir",
                    button_key: "settings.audit_dir_open",
                    on_open: open_config_dir,
                },
                p,
            ))
            .into_any_element()
    }
}
