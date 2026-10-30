//! SFTP 远程文件与内容检索模态弹窗视图渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为用户提供可视化的远程搜索交互界面，支持实时切换文件名模糊搜索与全文 Grep，
//!   并在检索结果列表中直接一键跳转定位或打开编辑目标文件；
//! - 核心优势：
//!   1. 检索面板状态与双栏视图完全解耦，不影响已打开目录的浏览与在途传输；
//!   2. 匹配结果以结构化虚拟列表展示，行号与匹配内容分色高亮，点击结果项即时导航跳转并关闭弹窗；
//! - 代价与局限：当前每次检索重走独立的 SSH Exec 通道，暂不维护远端常驻的倒排索引。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::sftp::{SftpRemotePaneRef, parent_remote};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteSearchQuery, RemoteSearchResultItem, SftpOperation, TabId};

/// SFTP 远程检索模态弹窗状态。
pub(crate) struct SftpSearchModalState {
    pub(crate) tab_id: TabId,
    /// 检索目标连接；`None` 表示标签页主远端，`Some` 表示并列远端列。
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) root_path: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) is_content: bool,
    pub(crate) case_sensitive: bool,
    pub(crate) is_searching: bool,
    pub(crate) results: Vec<RemoteSearchResultItem>,
    pub(crate) error: Option<String>,
}

impl AppShell {
    /// 打开目标远端列的检索模态对话框。
    pub(crate) fn sftp_open_search_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((root_path, side_connection)) = self
            .remote_pane_scope(tab_id, target)
            .map(|scope| (scope.pane.path.clone(), scope.side_connection))
        else {
            return;
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("sftp.search.placeholder").to_string())
        });
        input.update(cx, |state, cx| state.focus(window, cx));

        self.sftp_modals.search = Some(SftpSearchModalState {
            tab_id,
            connection_id: side_connection,
            root_path,
            input,
            is_content: false,
            case_sensitive: false,
            is_searching: false,
            results: Vec::new(),
            error: None,
        });
        cx.notify();
    }

    /// 触发执行远程搜索请求。
    pub(crate) fn sftp_execute_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref mut modal) = self.sftp_modals.search else {
            return;
        };
        let pattern = modal.input.read(cx).value().trim().to_string();
        if pattern.is_empty() {
            modal.error = Some(rust_i18n::t!("sftp.search.error_empty").to_string());
            let input = modal.input.clone();
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        modal.error = None;
        modal.is_searching = true;
        modal.results.clear();

        let tab_id = modal.tab_id;
        let target_connection = modal.connection_id;
        let query = RemoteSearchQuery {
            path: modal.root_path.clone(),
            pattern,
            is_content: modal.is_content,
            case_sensitive: modal.case_sensitive,
            max_depth: Some(10),
            max_results: 200,
        };

        log::info!(
            "发起远程检索请求，路径: {}，模式: {}，是否内容: {}",
            query.path,
            query.pattern,
            query.is_content
        );
        let Some(connection_id) = target_connection
            .or_else(|| self.workspace.tab(tab_id).and_then(|tab| tab.connection_id))
        else {
            return;
        };
        self.send_sftp_op_for_connection(
            tab_id,
            connection_id,
            SftpOperation::Search { query },
            cx,
        );
        cx.notify();
    }

    /// 渲染远程搜索弹窗视图。
    pub(crate) fn render_sftp_search_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.search.as_ref() else {
            return div().into_any_element();
        };

        let root_path = modal.root_path.clone();
        let input = modal.input.clone();
        let is_content = modal.is_content;
        let case_sensitive = modal.case_sensitive;
        let is_searching = modal.is_searching;
        let results = modal.results.clone();
        let error = modal.error.clone();
        let target_connection = modal.connection_id;

        let header =
            v_flex()
                .gap_2()
                .child(div().text_xs().text_color(p.muted_foreground).child(
                    rust_i18n::t!("sftp.search.root", path = root_path.as_str()).to_string(),
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .w_full()
                        .child(Input::new(&input).w_full())
                        .child(
                            Button::new("btn-trigger-search")
                                .primary()
                                .icon(IconName::Search)
                                .label(if is_searching {
                                    rust_i18n::t!("sftp.search.searching_short").to_string()
                                } else {
                                    rust_i18n::t!("sftp.search.search").to_string()
                                })
                                .disabled(is_searching)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.sftp_execute_search(window, cx);
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_4()
                        .child(
                            Checkbox::new("chk-is-content")
                                .checked(is_content)
                                .label(rust_i18n::t!("sftp.search.content_mode").to_string())
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    if let Some(ref mut m) = this.sftp_modals.search {
                                        m.is_content = *checked;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            Checkbox::new("chk-case-sensitive")
                                .checked(case_sensitive)
                                .label(rust_i18n::t!("sftp.search.case_sensitive").to_string())
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    if let Some(ref mut m) = this.sftp_modals.search {
                                        m.case_sensitive = *checked;
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .when_some(error, |this, err| {
                    this.child(div().text_xs().text_color(p.danger).child(err))
                });

        let mut results_container = v_flex()
            .id("sftp-search-results-list")
            .gap_1()
            .w_full()
            .h(px(280.0))
            .overflow_y_scrollbar();

        if is_searching {
            results_container = results_container.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.search.searching_hint").to_string()),
            );
        } else if results.is_empty() {
            results_container = results_container.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.search.no_results").to_string()),
            );
        } else {
            for (idx, item) in results.into_iter().enumerate() {
                let tab_id = modal.tab_id;
                let target_path = item.path.clone();
                let line_info = item
                    .line_number
                    .map(|num| rust_i18n::t!("sftp.search.line", line = num).to_string());
                let content_excerpt = item.line_content.clone();

                let row = h_flex()
                    .id(("search-res", idx))
                    .gap_2()
                    .p_1()
                    .rounded_md()
                    .hover(|s| s.bg(p.muted.opacity(0.3)))
                    .cursor_pointer()
                    .child(
                        Icon::new(IconName::File)
                            .with_size(px(14.0))
                            .text_color(p.muted_foreground),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.foreground)
                            .child(target_path.clone()),
                    )
                    .when_some(line_info, |this, line_text| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(p.primary)
                                .bg(p.secondary)
                                .px_1()
                                .rounded_sm()
                                .child(line_text),
                        )
                    })
                    .when_some(content_excerpt, |this, excerpt| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!(": {excerpt}")),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // 点击条目：关闭弹窗并导航到目标所在文件夹。
                        // 并列远端的检索结果必须回到该列自己的连接，不能落到主远端。
                        this.sftp_modals.search = None;
                        let dir = parent_remote(&target_path);
                        if let Some(connection_id) = target_connection {
                            this.request_sftp_list_for_connection(tab_id, connection_id, dir, cx);
                        } else {
                            this.request_sftp_list(tab_id, dir, cx);
                        }
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    }));

                results_container = results_container.child(row);
            }
        }

        let content = v_flex().gap_3().child(header).child(results_container);

        let footer = h_flex().gap_2().child(
            Button::new("close-sftp-search")
                .ghost()
                .label(rust_i18n::t!("app.close").to_string())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.sftp_modals.search = None;
                    this.focus_active_sftp_panel(window, cx);
                    cx.notify();
                })),
        );

        let dialog = Self::dialog_frame(
            "sftp-search-modal",
            rust_i18n::t!("sftp.search.title").to_string(),
            None::<String>,
            content,
            Some(footer),
            px(560.0),
            p,
        );

        Self::overlay(dialog, p)
    }
}
