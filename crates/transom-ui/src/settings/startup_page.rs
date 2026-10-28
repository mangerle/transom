//! 偏好设置中的"启动"分组：启动时标签区内容与会话列表展开状态。
//!
//! 设计原理（Why）：
//! - 两项都做成分段按钮而不是开关：档位互斥且都是三个，开关凑不出第三种行为，
//!   而"恢复上次 + 每次再叠一个本地终端"这类叠加组合恰恰是用户不想要的（会凭空
//!   多出标签），所以宁可让档位互斥、也不要两个布尔互相覆盖。
//! - 与同页其他分组的差别：这里改的都是**下次启动**才生效的策略，说明文字里必须
//!   写明，否则用户会以为界面会立刻变（空白工作台尤其容易被误认为"设置坏了"）。
//! - 侧栏的"记住上次"为什么不额外加开关：见 `transom_core::SidebarStartup` 的文档——
//!   记住与被自己上次行为绑架是同一个开关的两面，合成三档才是两个显式选择。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::*;
use transom_core::{SidebarStartup, StartupTabs};

/// 启动档位对应的文案键。
///
/// 用 `match` 返回静态串而不是运行时拼 key：拼出来的键在语言文件缺项时只会静默
/// 回退成英文原文，漏翻根本发现不了。
fn startup_tabs_label(tabs: StartupTabs) -> &'static str {
    match tabs {
        StartupTabs::RestoreLast => "settings.startup_restore_last",
        StartupTabs::EmptyWorkspace => "settings.startup_empty",
        StartupTabs::NewLocalTerminal => "settings.startup_local",
    }
}

/// 会话列表启动策略对应的文案键。
fn sidebar_startup_label(startup: SidebarStartup) -> &'static str {
    match startup {
        SidebarStartup::RememberLast => "settings.sidebar_remember",
        SidebarStartup::AlwaysShow => "settings.sidebar_show",
        SidebarStartup::AlwaysHide => "settings.sidebar_hide",
    }
}

impl AppShell {
    /// 启动组：标签区内容与会话列表展开状态两行分段选择。
    pub(crate) fn render_settings_startup(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tabs = self.settings.window.startup_tabs;
        let sidebar = self.settings.window.sidebar_startup;

        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.startup_tabs",
                Some("settings.startup_tabs_desc"),
                Self::setting_dropdown(
                    "dropdown-startup-tabs",
                    tabs,
                    StartupTabs::ALL
                        .into_iter()
                        .map(|tab| (tab, rust_i18n::t!(startup_tabs_label(tab)).to_string()))
                        .collect(),
                    |this, tab, cx| {
                        this.settings.window.startup_tabs = tab;
                        cx.notify();
                    },
                    cx,
                ),
                p,
            ))
            .child(Self::setting_row(
                "settings.sidebar_startup",
                Some("settings.sidebar_startup_desc"),
                Self::setting_dropdown(
                    "dropdown-sidebar-startup",
                    sidebar,
                    SidebarStartup::ALL
                        .into_iter()
                        .map(|startup| {
                            (
                                startup,
                                rust_i18n::t!(sidebar_startup_label(startup)).to_string(),
                            )
                        })
                        .collect(),
                    |this, startup, cx| {
                        this.settings.window.sidebar_startup = startup;
                        cx.notify();
                    },
                    cx,
                ),
                p,
            ))
            .into_any_element()
    }
}
