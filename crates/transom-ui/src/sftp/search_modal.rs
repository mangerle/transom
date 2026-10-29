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
use crate::sftp::parent_remote;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{RemoteSearchQuery, RemoteSearchResultItem, SftpOperation, TabId};

/// SFTP 远程检索模态弹窗状态。
pub(crate) struct SftpSearchModalState {
    pub(crate) tab_id: TabId,
    pub(crate) root_path: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) is_content: bool,
    pub(crate) case_sensitive: bool,
    pub(crate) is_searching: bool,
    pub(crate) results: Vec<RemoteSearchResultItem>,
    pub(crate) error: Option<String>,
}

impl AppShell {
    /// 打开 SFTP 远程检索模态对话框。
    pub(crate) fn sftp_open_search_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp() else {
            return;
        };
        let root_path = panel.remote.path.clone();
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("键入文件名关键字或搜索文本内容..."));
        input.update(cx, |state, cx| state.focus(window, cx));

        self.sftp_modals.search = Some(SftpSearchModalState {
            tab_id,
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
            modal.error = Some("搜索关键字不能为空".to_string());
            let input = modal.input.clone();
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        modal.error = None;
        modal.is_searching = true;
        modal.results.clear();

        let tab_id = modal.tab_id;
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
        self.send_sftp_op(tab_id, SftpOperation::Search { query }, cx);
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

        let header = v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!("检索根目录: {root_path}")),
            )
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
                                "检索中..."
                            } else {
                                "检索"
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
                            .label("全文内容检索 (Grep)")
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
                            .label("区分大小写")
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
                    .child("正在远程服务器执行检索，请稍候..."),
            );
        } else if results.is_empty() {
            results_container = results_container.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child("暂无匹配的条目，请调整关键字后重新检索"),
            );
        } else {
            for (idx, item) in results.into_iter().enumerate() {
                let tab_id = modal.tab_id;
                let target_path = item.path.clone();
                let line_info = item.line_number.map(|num| format!("行 {num}"));
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
                        // 点击条目：关闭弹窗并导航到目标所在文件夹
                        this.sftp_modals.search = None;
                        let dir = parent_remote(&target_path);
                        this.request_sftp_list(tab_id, dir, cx);
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
                .label("关闭")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.sftp_modals.search = None;
                    this.focus_active_sftp_panel(window, cx);
                    cx.notify();
                })),
        );

        let dialog = Self::dialog_frame(
            "sftp-search-modal",
            "远程文件与内容检索".to_string(),
            None::<String>,
            content,
            Some(footer),
            px(560.0),
            p,
        );

        Self::overlay(dialog, p)
    }
}
