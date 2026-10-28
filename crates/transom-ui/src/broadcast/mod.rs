//! 底部广播输入条：把一条命令或多行脚本同时发送到选定的终端窗格。
//!
//! 与快捷命令（Snippets）的边界：Snippets 只填当前焦点窗格，目标固定且单值；
//! 这里引入独立的目标集合，默认取当前标签的窗格，用户可显式多选其他标签。
//! 目标单位是窗格而不是标签：每个窗格是一条独立连接，标签并排或标签内分屏
//! 都只是显示方式，广播逻辑不感知布局。
//!
//! 单行模式回车发送；多行模式同样回车发送，`Shift+Enter` 换行。多行文本走
//! 终端当前的括号粘贴语义，避免远端 shell 把脚本的每一行提前执行。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::workspace::pane_status_indicator;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Selectable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::popover::Popover;
use gpui_kit::*;
use std::collections::HashSet;
mod rows;

use rows::{BroadcastTargetRow, connection_icon, render_target_rows};
use transom_core::{GroupId, TabId};

/// 广播条的运行期状态。
///
/// 目标集合只活在本次打开期间：目标跟着"当前视觉标签"切换，同一视觉标签内
/// 点窗格、切焦点不会重置用户手动勾选的结果。`targets_scope` 记录当前目标是
/// 按哪一组标签组算出来的，用于区分"真的换了标签"与"同一标签内换焦点"。
pub(crate) struct BroadcastBarState {
    pub(crate) open: bool,
    pub(crate) multiline: bool,
    pub(crate) targets_open: bool,
    pub(crate) input: Entity<InputState>,
    pub(crate) textarea: Entity<TextareaState>,
    pub(crate) targets: HashSet<TabId>,
    pub(crate) targets_scope: Vec<GroupId>,
}

impl AppShell {
    pub(crate) fn toggle_broadcast_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.broadcast.open {
            self.close_broadcast_bar(window, cx);
        } else {
            self.open_broadcast_bar(window, cx);
        }
    }

    fn open_broadcast_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.broadcast.open = true;
        self.broadcast.multiline = false;
        self.broadcast.targets_open = false;
        let scope = self.current_visual_groups();
        self.broadcast.targets = self.default_broadcast_targets_for(&scope);
        self.broadcast.targets_scope = scope;
        self.broadcast.input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.broadcast
            .textarea
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn close_broadcast_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.broadcast.open = false;
        self.broadcast.targets_open = false;
        if let Some(active) = self.workspace.active_tab {
            self.activate_tab(active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// 单行与多行互切，文本随模式带过去，焦点落到新输入框。
    pub(crate) fn toggle_broadcast_multiline(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.broadcast.open {
            return;
        }
        let text = if self.broadcast.multiline {
            self.broadcast.textarea.read(cx).value().to_string()
        } else {
            self.broadcast.input.read(cx).value().to_string()
        };
        self.broadcast.multiline = !self.broadcast.multiline;
        if self.broadcast.multiline {
            self.broadcast.textarea.update(cx, |state, cx| {
                state.set_value(&text, window, cx);
                state.focus(window, cx);
            });
        } else {
            // 单行输入框不能承载换行；收起来时把换行折成空格，保留可编辑内容。
            let single = text.replace("\r\n", "\n").replace('\n', " ");
            self.broadcast.input.update(cx, |state, cx| {
                state.set_value(&single, window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    /// 把当前输入发送到所有已选目标；单行补回车，多行按粘贴语义整段发送。
    pub(crate) fn broadcast_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.broadcast.open {
            return;
        }
        let raw = if self.broadcast.multiline {
            self.broadcast.textarea.read(cx).value().to_string()
        } else {
            self.broadcast.input.read(cx).value().to_string()
        };
        let text = raw.trim_end_matches(['\r', '\n']).to_string();
        if text.trim().is_empty() {
            return;
        }
        // 目标集合必须仍然落在当前视觉标签内：标签切换或窗格关闭后，残留的
        // 旧目标不能继续收到命令。
        let allowed: HashSet<TabId> = self
            .current_visual_groups()
            .into_iter()
            .filter_map(|group_id| {
                self.workspace
                    .groups
                    .iter()
                    .find(|group| group.id == group_id)
            })
            .flat_map(|group| group.pane_ids())
            .collect();
        self.broadcast.targets.retain(|id| allowed.contains(id));
        let targets: Vec<TabId> = self.broadcast.targets.iter().copied().collect();
        if targets.is_empty() {
            self.status = Some(rust_i18n::t!("broadcast.no_targets").to_string());
            cx.notify();
            return;
        }

        let multiline = self.broadcast.multiline;
        let mut failed = 0usize;
        for tab_id in &targets {
            let Some(tab) = self.workspace.tabs.iter().find(|tab| tab.id == *tab_id) else {
                failed += 1;
                continue;
            };
            let Some(terminal) = tab.terminal() else {
                failed += 1;
                continue;
            };
            let sent = terminal.update(cx, |view, cx| {
                if multiline {
                    view.send_paste(&text, true, cx)
                } else {
                    view.send_text(&format!("{text}\r"), cx)
                }
            });
            if !sent {
                failed += 1;
            }
        }
        if failed > 0 {
            self.set_error(rust_i18n::t!("broadcast.send_failed").to_string());
        }
        // 命令内容可能含敏感信息，只记录目标数量与失败数，绝不落盘正文。
        log::info!(
            "广播命令已发送，目标窗格数: {}，失败数: {}",
            targets.len(),
            failed
        );

        self.broadcast
            .input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.broadcast
            .textarea
            .update(cx, |state, cx| state.set_value("", window, cx));
        if multiline {
            self.broadcast
                .textarea
                .update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.broadcast
                .input
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    pub(crate) fn set_broadcast_target(
        &mut self,
        tab_id: TabId,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        if selected {
            self.broadcast.targets.insert(tab_id);
        } else {
            self.broadcast.targets.remove(&tab_id);
        }
        cx.notify();
    }

    /// 当前"视觉标签"里的标签组，顺序固定为并排对的前后格或单标签本身。
    ///
    /// 用它作为目标作用域：并排时是两个标签组（同一个合并条目），配对挂起或
    /// 未并排时只有当前标签。顺序不跟随焦点，避免在并排的两格间点来点去时
    /// 误判成切换了标签。
    fn current_visual_groups(&self) -> Vec<GroupId> {
        let Some(active_index) = self.active_group_index() else {
            return Vec::new();
        };
        let active_group = self.workspace.groups[active_index].id;
        if let Some(tiled) = self.active_tiling() {
            return tiled.occupied_groups();
        }
        vec![active_group]
    }

    /// 标签切换、并排关系变化后，把广播目标重置到新的视觉标签。
    ///
    /// 同一视觉标签内换焦点（点窗格、切并排格）不会改变作用域，因此不会
    /// 覆盖用户手动增删的目标。
    pub(crate) fn sync_broadcast_targets(&mut self) {
        if !self.broadcast.open {
            return;
        }
        let scope = self.current_visual_groups();
        let same = scope.len() == self.broadcast.targets_scope.len()
            && scope
                .iter()
                .all(|group| self.broadcast.targets_scope.contains(group));
        if same {
            return;
        }
        self.broadcast.targets = self.default_broadcast_targets_for(&scope);
        self.broadcast.targets_scope = scope;
    }

    /// 按给定的视觉标签作用域收集默认目标：只取已有终端的窗格。
    fn default_broadcast_targets_for(&self, groups: &[GroupId]) -> HashSet<TabId> {
        let mut targets = HashSet::with_capacity(self.workspace.tabs.len());
        for group_id in groups {
            let Some(group) = self
                .workspace
                .groups
                .iter()
                .find(|group| group.id == *group_id)
            else {
                continue;
            };
            for pane_id in group.pane_ids() {
                if self
                    .workspace
                    .tabs
                    .iter()
                    .any(|tab| tab.id == pane_id && tab.terminal().is_some())
                {
                    targets.insert(pane_id);
                }
            }
        }
        targets
    }

    /// 目标选择列表：只列当前视觉标签内的窗格，不跨标签。
    fn broadcast_target_rows(&self, p: &Palette) -> Vec<BroadcastTargetRow> {
        let groups = self.current_visual_groups();
        let mut rows = Vec::with_capacity(self.workspace.tabs.len());
        for group_id in groups {
            let Some(group) = self
                .workspace
                .groups
                .iter()
                .find(|group| group.id == group_id)
            else {
                continue;
            };
            let pane_ids = group.pane_ids();
            let multi = pane_ids.len() > 1;
            for (position, pane_id) in pane_ids.iter().enumerate() {
                let Some(tab) = self
                    .workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.id == *pane_id && tab.terminal().is_some())
                else {
                    continue;
                };
                let (dot_color, _) = pane_status_indicator(&tab.status, p);
                let mut title = tab.display_name();
                if group.serial > 0 {
                    title = format!("{title} #{}", group.serial);
                }
                if multi {
                    let suffix =
                        rust_i18n::t!("broadcast.pane_suffix", index = position + 1).to_string();
                    title = format!("{title} · {suffix}");
                }
                rows.push(BroadcastTargetRow {
                    tab_id: *pane_id,
                    title,
                    dot_color,
                    icon: connection_icon(&tab.session.connection),
                    selected: self.broadcast.targets.contains(pane_id),
                });
            }
        }
        rows
    }

    /// 状态栏快捷命令旁的广播入口。
    pub(crate) fn render_broadcast_trigger(&mut self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("broadcast-trigger")
            .ghost()
            .xsmall()
            .icon(IconName::Megaphone)
            .selected(self.broadcast.open)
            .tooltip(rust_i18n::t!("broadcast.title").to_string())
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_broadcast_bar(window, cx);
            }))
            .into_any_element()
    }

    pub(crate) fn render_broadcast_bar(
        &mut self,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let multiline = self.broadcast.multiline;
        let target_trigger = Button::new("broadcast-targets-trigger")
            .ghost()
            .xsmall()
            .icon(IconName::SquareCheck)
            .label(rust_i18n::t!("broadcast.targets").to_string())
            .selected(self.broadcast.targets_open)
            .tooltip(rust_i18n::t!("broadcast.targets").to_string());
        let app = cx.entity().clone();
        let app_open = app.clone();
        let content_app = app.clone();
        let content_palette = p.clone();
        let targets_popover = Popover::new("broadcast-targets-popover")
            .anchor(Anchor::BottomLeft)
            .offset(px(-1.0))
            .trigger_style(StyleRefinement::default().flex_none().flex().items_center())
            .open(self.broadcast.targets_open)
            .on_open_change(move |open, _window, cx| {
                let open = *open;
                app_open.update(cx, |this, cx| {
                    if this.broadcast.targets_open != open {
                        this.broadcast.targets_open = open;
                        cx.notify();
                    }
                });
            })
            .trigger(target_trigger)
            .content(move |_, _, cx| {
                let rows = content_app.read(cx).broadcast_target_rows(&content_palette);
                render_target_rows(&content_app, rows, &content_palette)
            })
            .into_any_element();

        let close = Button::new("broadcast-close")
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip(rust_i18n::t!("app.close").to_string())
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_broadcast_bar(window, cx);
            }));
        let toggle = Button::new("broadcast-mode-toggle")
            .ghost()
            .xsmall()
            .icon(if multiline {
                IconName::ArrowDown
            } else {
                IconName::ArrowUp
            })
            .tooltip(if multiline {
                rust_i18n::t!("broadcast.collapse").to_string()
            } else {
                rust_i18n::t!("broadcast.expand").to_string()
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_broadcast_multiline(window, cx);
            }));

        if multiline {
            v_flex()
                .w_full()
                .bg(p.tab_bar)
                .border_t_1()
                .border_color(p.border)
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .child(targets_popover)
                        .child(div().flex_1())
                        .child(close)
                        .child(toggle),
                )
                .child(
                    div().px_2().pb_2().w_full().child(
                        Textarea::new(&self.broadcast.textarea)
                            .appearance(false)
                            .bordered(false)
                            .h(rems(7.0))
                            .w_full(),
                    ),
                )
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .h(rems(2.5))
                .px_2()
                .gap_2()
                .items_center()
                .bg(p.tab_bar)
                .border_t_1()
                .border_color(p.border)
                .child(targets_popover)
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.broadcast.input)
                            .appearance(false)
                            .bordered(false)
                            .w_full(),
                    ),
                )
                .child(close)
                .child(toggle)
                .into_any_element()
        }
    }
}
