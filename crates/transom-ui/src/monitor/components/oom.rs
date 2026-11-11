//! OOM 告警横幅：最近一次 OOM killer 记录存在时展示。
//!
//! 设计原理（Why）：抽屉与全屏看板各自实现过一份同样的横幅，文案键与
//! 布局容易分叉；统一组件只依赖统计快照与调色板。
//!
//! 代价与局限：文案固定为“完整 OOM 记录”，不再区分抽屉/看板的简写。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染 OOM 横幅；无记录返回 `None`。
pub(crate) fn oom_banner(stats: &SystemStats, p: &Palette) -> Option<AnyElement> {
    let oom = stats.recent_oom.as_ref()?;
    Some(
        h_flex()
            .w_full()
            .gap_2()
            .p_2p5()
            .rounded_md()
            .bg(p.danger.opacity(0.15))
            .border_1()
            .border_color(p.danger.opacity(0.4))
            .items_center()
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::WARNING))
                    .small()
                    .text_color(p.danger),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.danger)
                            .child(rust_i18n::t!("monitor.view.oom_title").to_string()),
                    )
                    .child(
                        div().text_xs().text_color(p.foreground).child(
                            rust_i18n::t!(
                                "monitor.view.oom_killed_full",
                                name = &oom.process_name,
                                pid = oom.pid
                            )
                            .to_string(),
                        ),
                    ),
            )
            .into_any_element(),
    )
}
