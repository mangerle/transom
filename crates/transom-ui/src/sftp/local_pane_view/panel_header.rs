//! SFTP 面板顶部通用标题栏与关闭入口。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SFTP 双栏文件浏览器主面板及全屏/单栏模式提供统一的标题条渲染；
//! - 核心优势：集中封装面板标题、关闭事件及额外的右侧按钮扩展点。

use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

impl SftpPanelView {
    /// 渲染 SFTP 面板顶部通用标题栏，包含标题、扩展按钮插槽与关闭按钮。
    pub(crate) fn render_sftp_panel_header(
        &self,
        title: String,
        extra: Option<AnyElement>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let close_btn = Button::new("close-sftp")
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip(rust_i18n::t!("sftp.nav.close_panel").to_string())
            .on_click(cx.listener(|this, _, _, cx| {
                cx.emit(crate::sftp::SftpPanelEvent::ClosePanel {
                    tab_id: this.tab_id,
                });
                cx.notify();
            }));

        let mut header = crate::widgets::PanelHeader::new(title, p);
        if let Some(extra_element) = extra {
            header = header.action(extra_element);
        }
        header.action(close_btn).render(p)
    }
}
