//! 主工作区无打开会话时的欢迎与快速连接看板。
//!
//! 设计原理（Why）：
//! - 实现初衷：当用户未打开任何标签时，中央工作区应提供高效且符合直觉的复工入口。
//!   以 Logo 作为视觉重心锚点，下方提供 2 行（每行最多 5 个，总计最多 10 个）快捷标签卡片；
//! - 核心优势：
//!   1. 首位固定放置高频使用的【本地终端】；
//!   2. 中间依序展示最近打开过的会话，末尾固定追加【新建会话】入口（无最近记录时自然处于第二位）；
//!   3. 最近会话卡片支持悬浮移除按钮与右键上下文菜单，便于用户灵活清理历史记录；
//!   4. 避免了机械罗列左侧静态会话树前几项导致的冗余展示，与左侧全量资产库形成“动态工作台 vs 静态资产”的明确分工；
//! - 代价与局限：最近会话按使用时间降序截断展示（最多 8 个），更多历史仍需从左侧栏或全局搜索触达。

use crate::app::AppShell;
use crate::brand;
use crate::palette::Palette;
use crate::session_appearance::{session_color, session_icon};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{Connection, Session};

/// 品牌 Logo 显示边长（rem）。
const LOGO_SIZE: f32 = 3.5;

/// 每行最大卡片容量。
const ROW_CAPACITY: usize = 5;

/// 最大展示卡片总数（两行 × 5 个）。
const MAX_CARDS: usize = 10;

/// 卡片固定宽度（像素）。
const CARD_WIDTH: f32 = 160.0;

/// 卡片固定高度（像素）。
const CARD_HEIGHT: f32 = 64.0;

/// 卡片之间的水平间距（像素，对应 gap_3 即 12px）。
const CARD_GAP: f32 = 12.0;

/// 5 列卡片网格的总宽度（像素），用于跨行时两行严格按列左对齐。
const GRID_WIDTH: f32 = (ROW_CAPACITY as f32) * CARD_WIDTH + ((ROW_CAPACITY - 1) as f32) * CARD_GAP;

impl AppShell {
    /// 渲染空状态主工作区：包含顶部品牌标志与下方的快捷/最近打开会话卡片网格。
    pub(crate) fn render_empty_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let logo_section = v_flex()
            .items_center()
            .gap_2()
            .child(brand::logo_mark(LOGO_SIZE, window, cx))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("app.name").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("app.empty_welcome_hint").to_string()),
            );

        // 筛选存在于当前目录中的有效最近会话
        let valid_recent: Vec<&Session> = self
            .settings
            .window
            .recent_sessions
            .iter()
            .filter_map(|id| self.catalog.sessions.iter().find(|s| s.id == *id))
            .collect();

        let mut cards = Vec::with_capacity(MAX_CARDS);
        // 第一位固定为本地终端
        cards.push(self.render_empty_local_terminal_card(p, cx));

        // 中间依序放入最近打开的会话（最多 8 个，为末尾的新建会话预留槽位）
        for session in valid_recent.into_iter().take(MAX_CARDS - 2) {
            cards.push(self.render_empty_session_card(session, p, cx));
        }

        // 新建会话永远追加在卡片列表的最后一个；无最近记录时自然处于第二位
        cards.push(self.render_empty_new_session_card(p, cx));

        let (first_row, second_row) = if cards.len() <= ROW_CAPACITY {
            (cards, Vec::new())
        } else {
            let second = cards.split_off(ROW_CAPACITY);
            (cards, second)
        };

        let mut grid = v_flex().gap_3().items_center();
        if second_row.is_empty() {
            // 只有单行时，自然居中展示
            grid = grid.child(h_flex().gap_3().justify_center().children(first_row));
        } else {
            // 跨到第二行时，两行定宽并严格按列左对齐，避免第二行出现突兀居中错位
            grid = grid.child(
                h_flex()
                    .w(px(GRID_WIDTH))
                    .gap_3()
                    .justify_start()
                    .children(first_row),
            );
            grid = grid.child(
                h_flex()
                    .w(px(GRID_WIDTH))
                    .gap_3()
                    .justify_start()
                    .children(second_row),
            );
        }

        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_6()
            .child(logo_section)
            .child(grid)
            .into_any_element()
    }

    /// 渲染本地终端快捷卡片。
    fn render_empty_local_terminal_card(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let shell_desc = match self.settings.local_shell.as_deref() {
            Some(shell) => shell.to_string(),
            None => rust_i18n::t!("app.empty_local_shell").to_string(),
        };

        empty_card_shell(
            ElementId::Name("empty-quick-local".into()),
            IconName::Terminal,
            p.primary,
            rust_i18n::t!("app.new_local").to_string(),
            shell_desc,
            p,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            this.open_local_terminal(window, cx);
        }))
        .into_any_element()
    }

    /// 渲染新建会话快捷卡片。
    fn render_empty_new_session_card(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        empty_card_shell(
            ElementId::Name("empty-quick-new-session".into()),
            IconName::Plus,
            p.primary,
            rust_i18n::t!("app.new_session").to_string(),
            rust_i18n::t!("app.empty_new_session_hint").to_string(),
            p,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            this.open_new_session(window, cx);
        }))
        .into_any_element()
    }

    /// 渲染单个最近打开的会话卡片。
    fn render_empty_session_card(
        &self,
        session: &Session,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = session.id;
        let (default_icon, detail_text) = match &session.connection {
            Connection::Ssh(ssh) => (IconName::Server, format!("{}@{}", ssh.username, ssh.host)),
            Connection::Telnet(telnet) => (
                IconName::Network,
                format!("{}:{}", telnet.host, telnet.port),
            ),
            Connection::Local(local) => {
                let shell = local.shell.as_deref().unwrap_or("sh");
                (IconName::Terminal, shell.to_string())
            }
            Connection::Serial(serial) => (
                IconName::Cable,
                format!("{} @ {}", serial.port, serial.baud_rate),
            ),
            _ => (IconName::Server, String::new()),
        };

        let icon = session_icon(session.icon).unwrap_or(default_icon);
        let color = session_color(session.color).unwrap_or(p.foreground);
        let app_entity = cx.entity().clone();

        // 悬浮微型移除按钮：仅在卡片处于 hover 时显示，点击阻止冒泡避免触发打开会话
        let remove_button = div()
            .id(ElementId::Name(
                format!("remove-recent-{}", session.id).into(),
            ))
            .absolute()
            .top(px(4.0))
            .right(px(4.0))
            .size(px(18.0))
            .rounded(p.radius.sm)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .opacity(0.0)
            .group_hover("recent-session-card", |s| s.opacity(1.0))
            .text_color(p.muted_foreground)
            .hover(|s| s.bg(p.muted).text_color(p.foreground))
            .tooltip(move |window, cx| {
                Tooltip::new(rust_i18n::t!("app.remove_from_recent").to_string()).build(window, cx)
            })
            .child(Icon::new(IconName::X).size(px(11.0)))
            .on_click(cx.listener(move |this, _, _window, cx| {
                cx.stop_propagation();
                this.remove_recent_session(session_id, cx);
            }));

        let card = empty_card_shell(
            ElementId::Name(format!("empty-recent-{}", session.id).into()),
            icon,
            color,
            session.name.clone(),
            detail_text,
            p,
        )
        .group("recent-session-card")
        .child(remove_button)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.connect_session(session_id, window, cx);
        }))
        .context_menu({
            let app = app_entity.clone();
            move |menu, _window, _cx| {
                let app_connect = app.clone();
                let app_remove = app.clone();
                menu.item(
                    PopupMenuItem::new(rust_i18n::t!("menu.connect").to_string())
                        .icon(IconName::Terminal)
                        .on_click(move |_, window, cx| {
                            app_connect.update(cx, |this, cx| {
                                this.connect_session(session_id, window, cx);
                            });
                        }),
                )
                .item(PopupMenuItem::separator())
                .item(
                    PopupMenuItem::new(rust_i18n::t!("app.remove_from_recent").to_string())
                        .icon(IconName::X)
                        .on_click(move |_, _window, cx| {
                            app_remove.update(cx, |this, cx| {
                                this.remove_recent_session(session_id, cx);
                            });
                        }),
                )
            }
        });

        card.into_any_element()
    }
}

/// 快捷卡片的基础样式外壳。
fn empty_card_shell(
    id: ElementId,
    icon: IconName,
    icon_color: Hsla,
    title: String,
    subtitle: String,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .w(px(CARD_WIDTH))
        .h(px(CARD_HEIGHT))
        .p_2p5()
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.border)
        .bg(p.muted.opacity(0.35))
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(p.muted.opacity(0.75))
                .border_color(p.primary.opacity(0.6))
        })
        .child(
            v_flex()
                .h_full()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(Icon::new(icon).size(px(15.0)).text_color(icon_color))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pr(px(14.0))
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .truncate()
                        .child(subtitle),
                ),
        )
}
