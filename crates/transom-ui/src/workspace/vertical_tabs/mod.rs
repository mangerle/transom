//! 纵向标签面板：参照 Warp 的 Vertical Tabs，把终端标签从顶部横向条
//! 迁移为左侧槽位的纵向列表。
//!
//! 设计原理（Why）：
//! - 状态层完全复用 `WorkspaceState.groups` 的数组顺序与现有激活、关闭逻辑，
//!   面板只负责把"组顺序"换成纵向布局，避免维护第二套标签模型；
//! - 面板与会话树共用同一个左侧槽位，由活动栏的同级条目互斥切换：
//!   连接入口与运行实例是同一层导航，不再并排占用两条竖栏；
//! - 默认仍是顶部横向条（见 `TabPosition`），左侧模式是显式外观选择；
//!   槽位不显示标签面板时回退顶部横向条，保证任何时刻都有切换入口；
//! - 第一版按"每组一行"展示（已确认的粒度），组内分屏用角标与数目表达，
//!   不展开逐窗格行；这与 Warp 折叠态组头的信息结构一致。
//!
//! 代价与局限：不做逐窗格行、组内搜索跳转与详情侧栏；标签数量级只有几十，
//! 列表使用普通滚动容器而非虚拟列表，没有测量与回收成本。

mod drag;
mod item;

use crate::app::AppShell;
use crate::connection::session_host;
use crate::palette::Palette;
use crate::session_appearance::session_icon;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{GroupId, SessionId, TabPosition};

impl AppShell {
    /// 左侧槽位当前是否显示纵向标签面板。
    ///
    /// 标签面板与会话树共用同一个槽位，面板可见时主区顶部不再绘制横向标签条；
    /// 顶部模式下该入口整体不可用，活动栏也不渲染对应图标。
    pub(crate) fn show_vertical_tabs_panel(&self) -> bool {
        self.settings.window.tab_position == TabPosition::Left
            && self.sidebar.visible
            && self.vertical_tabs_open
    }

    /// 切到左侧槽位的标签面板；顶部模式或面板已可见时为空操作。
    ///
    /// 会话树里的连接动作完成后调用：连接入口在会话树，连接完成后用户要看
    /// 的是终端列表，因此自动把槽位切到标签面板。
    pub(crate) fn reveal_vertical_tabs_panel(&mut self, cx: &mut Context<Self>) {
        if self.settings.window.tab_position != TabPosition::Left {
            return;
        }
        if self.sidebar.visible && self.vertical_tabs_open {
            return;
        }
        self.sidebar.visible = true;
        self.vertical_tabs_open = true;
        cx.notify();
    }

    /// 渲染纵向标签面板：控制条（搜索、新增会话菜单）与按组展开的标签列表。
    pub(crate) fn render_vertical_tabs_panel(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.vertical_tabs_search.read(cx).value().to_string();
        let query_lower = query.trim().to_lowercase();
        let total = self.workspace.groups.len();
        let mut visible: Vec<GroupId> = Vec::with_capacity(total);
        for group in &self.workspace.groups {
            if self.group_matches(group.id, &query_lower) {
                visible.push(group.id);
            }
        }

        let list = if total == 0 {
            Self::render_vertical_tabs_empty(false, p)
        } else if visible.is_empty() {
            Self::render_vertical_tabs_empty(true, p)
        } else {
            let mut rows = Vec::with_capacity(visible.len());
            for group_id in visible {
                if let Some(row) = item::render_vertical_tab_row(self, group_id, p, cx) {
                    rows.push(row);
                }
            }
            v_flex()
                .id("vertical-tabs-list")
                .flex_1()
                .w_full()
                .overflow_y_scroll()
                .p_1p5()
                .gap_0p5()
                .children(rows)
                .into_any_element()
        };

        v_flex()
            .size_full()
            .h_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .child(self.render_vertical_tabs_control_bar(p, cx))
            .child(list)
            .into_any_element()
    }

    /// 面板控制条：搜索框与"新增会话"下拉菜单。
    ///
    /// 高度与顶部标签条一致，切换槽位内容时主区起始位置不发生跳动。
    fn render_vertical_tabs_control_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let menu_data = self.new_session_menu_data();
        let menu_app = cx.entity().clone();
        h_flex()
            .w_full()
            .h(rems(size::TAB_BAR_HEIGHT))
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(p.sidebar_border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.vertical_tabs_search).small().w_full()),
            )
            .child(
                Button::new("vertical-tabs-new")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip(rust_i18n::t!("app.new_session").to_string())
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                        build_new_session_menu(menu, window, cx, &menu_app, &menu_data)
                    }),
            )
            .into_any_element()
    }

    /// 空状态：区分"没有打开的终端"与"搜索无结果"。
    fn render_vertical_tabs_empty(filtered: bool, p: &Palette) -> AnyElement {
        let (title, hint) = if filtered {
            (
                rust_i18n::t!("tab.vertical_no_matches").to_string(),
                rust_i18n::t!("tab.vertical_no_matches_hint").to_string(),
            )
        } else {
            (
                rust_i18n::t!("tab.vertical_empty").to_string(),
                rust_i18n::t!("tab.vertical_empty_hint").to_string(),
            )
        };
        v_flex()
            .flex_1()
            .w_full()
            .p_4()
            .gap_2()
            .items_center()
            .justify_center()
            .child(Icon::new(IconName::Terminal).text_color(p.muted_foreground))
            .child(div().text_sm().text_color(p.muted_foreground).child(title))
            .child(div().text_xs().text_color(p.muted_foreground).child(hint))
            .into_any_element()
    }

    /// 组是否命中搜索词：按活动窗格的标题、序号与主机地址匹配。
    ///
    /// 只匹配组代表行（活动窗格）而不是全部窗格，与"每组一行"的展示粒度
    /// 保持一致；用户切换窗格后该组会以新标题重新参与过滤。
    fn group_matches(&self, group_id: GroupId, query_lower: &str) -> bool {
        if query_lower.is_empty() {
            return true;
        }
        let Some(group) = self.workspace.group(group_id) else {
            return false;
        };
        let Some(tab) = self.workspace.tab(group.active_pane) else {
            return false;
        };
        let mut text = tab.display_name();
        if group.serial > 0 {
            text.push_str(&format!(" #{}", group.serial));
        }
        if let Some(host) = session_host(&tab.session) {
            text.push(' ');
            text.push_str(host);
        }
        text_matches_query(query_lower, &text)
    }

    /// 收集"新增会话"菜单所需的会话快照：根级会话平铺，文件夹各成一个子菜单。
    ///
    /// 菜单只持有展示所需的标题与图标，不借用目录；会话或文件夹在菜单打开前
    /// 被删除时，点击只会落到已失效的 id，连接入口自身会再校验一次。
    fn new_session_menu_data(&self) -> NewSessionMenuData {
        let mut folders: Vec<NewSessionFolder> = self
            .catalog
            .folders
            .iter()
            .map(|folder| NewSessionFolder {
                folder_id: folder.id,
                label: self.folder_menu_label(folder),
                entries: Vec::new(),
            })
            .collect();
        let mut root = Vec::new();
        for session in &self.catalog.sessions {
            let entry = NewSessionEntry {
                session_id: session.id,
                label: if session.name.trim().is_empty() {
                    session_host(session)
                        .map(str::to_string)
                        .unwrap_or_else(|| session.name.clone())
                } else {
                    session.name.clone()
                },
                icon: session_icon(session.icon)
                    .unwrap_or_else(|| item::connection_icon(&session.connection)),
            };
            let Some(folder_id) = session.folder_id else {
                root.push(entry);
                continue;
            };
            if let Some(folder) = folders
                .iter_mut()
                .find(|folder| folder.folder_id == folder_id)
            {
                folder.entries.push(entry);
            } else {
                root.push(entry);
            }
        }
        NewSessionMenuData { root, folders }
    }

    /// 文件夹在菜单里的显示名：带上级路径，避免不同层级的同名文件夹混淆。
    fn folder_menu_label(&self, folder: &transom_core::SessionFolder) -> String {
        let mut names = vec![folder.name.clone()];
        let mut seen = HashSet::with_capacity(4);
        seen.insert(folder.id);
        let mut current = folder.parent;
        while let Some(parent_id) = current {
            if !seen.insert(parent_id) {
                break;
            }
            let Some(parent) = self
                .catalog
                .folders
                .iter()
                .find(|item| item.id == parent_id)
            else {
                break;
            };
            names.push(parent.name.clone());
            current = parent.parent;
        }
        names.reverse();
        names.join(" / ")
    }
}

/// "新增会话"菜单的单个会话项快照。
#[derive(Clone)]
struct NewSessionEntry {
    session_id: SessionId,
    label: String,
    icon: IconName,
}

/// "新增会话"菜单的文件夹分组。
struct NewSessionFolder {
    folder_id: transom_core::FolderId,
    label: String,
    entries: Vec<NewSessionEntry>,
}

/// "新增会话"菜单数据：根级会话与按文件夹分组的会话。
struct NewSessionMenuData {
    root: Vec<NewSessionEntry>,
    folders: Vec<NewSessionFolder>,
}

/// 构建"新增会话"下拉菜单。
///
/// 参照 Warp 的控制条：顶部是新建本地终端，其余是现有会话的快捷入口；
/// 文件夹走二级菜单，避免会话多时把一级菜单撑得过长。
fn build_new_session_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut App,
    app: &Entity<AppShell>,
    data: &NewSessionMenuData,
) -> PopupMenu {
    let local_app = app.clone();
    let mut menu = menu
        .scrollable(true)
        .max_h(px(460.0))
        .item(
            PopupMenuItem::new(rust_i18n::t!("app.new_local").to_string())
                .icon(IconName::Terminal)
                .on_click(move |_, window, cx| {
                    local_app.update(cx, |this, cx| this.open_local_terminal(window, cx));
                }),
        )
        .item(PopupMenuItem::separator());
    for entry in &data.root {
        menu = menu.item(session_menu_item(entry, app));
    }
    for folder in &data.folders {
        if folder.entries.is_empty() {
            continue;
        }
        let entries = folder.entries.clone();
        let submenu_app = app.clone();
        let submenu = PopupMenu::build(window, cx, move |menu, _window, _cx| {
            let mut menu = menu.scrollable(true).max_h(px(360.0));
            for entry in &entries {
                menu = menu.item(session_menu_item(entry, &submenu_app));
            }
            menu
        });
        menu =
            menu.item(PopupMenuItem::submenu(folder.label.clone(), submenu).icon(IconName::Folder));
    }
    let tree_app = app.clone();
    menu.item(PopupMenuItem::separator()).item(
        PopupMenuItem::new(rust_i18n::t!("vertical_tabs.open_session_tree").to_string())
            .icon(IconName::Server)
            .on_click(move |_, _, cx| {
                tree_app.update(cx, |this, cx| {
                    this.sidebar.visible = true;
                    this.vertical_tabs_open = false;
                    cx.notify();
                });
            }),
    )
}

/// 单个会话的菜单项：选中后直接发起连接。
fn session_menu_item(entry: &NewSessionEntry, app: &Entity<AppShell>) -> PopupMenuItem {
    let app = app.clone();
    let session_id = entry.session_id;
    PopupMenuItem::new(entry.label.clone())
        .icon(entry.icon)
        .on_click(move |_, window, cx| {
            app.update(cx, |this, cx| this.connect_session(session_id, window, cx));
        })
}

/// 大小写不敏感的包含匹配；查询词已由调用方转小写。
fn text_matches_query(query_lower: &str, text: &str) -> bool {
    text.to_lowercase().contains(query_lower)
}

#[cfg(test)]
mod tests {
    use super::text_matches_query;

    #[test]
    fn search_is_case_insensitive_and_substring_based() {
        assert!(text_matches_query("prod", "Prod-DB"));
        assert!(text_matches_query("db", "Prod-DB"));
        assert!(!text_matches_query("staging", "Prod-DB"));
        assert!(text_matches_query("", "任意文本"), "空查询视为全部命中");
    }
}
