//! SFTP 远程文件与内容检索模态弹窗视图渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为用户提供可视化的远程搜索交互界面，支持实时切换文件名模糊搜索与全文 Grep，
//!   并在检索结果列表中直接一键跳转定位或打开编辑目标文件；
//! - 核心优势：
//!   1. 检索面板状态与双栏视图完全解耦，不影响已打开目录的浏览与在途传输；
//!   2. 匹配结果以结构化虚拟列表展示，行号与匹配内容分色高亮，点击结果项即时导航跳转并关闭弹窗；
//! - 代价与局限：当前每次检索重走独立的 SSH Exec 通道，暂不维护远端常驻的倒排索引。

use super::entity::SftpPanelView;
use crate::palette::Palette;
use crate::sftp::{SftpRemotePaneRef, parent_remote};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteSearchQuery, RemoteSearchResultItem, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// SFTP 远程检索弹窗事件。
pub(crate) enum SftpSearchModalEvent {
    Execute {
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        query: RemoteSearchQuery,
    },
    Jump {
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        dir: String,
    },
    Cancel,
}

/// SFTP 远程检索模态弹窗实体视图。
pub(crate) struct SftpSearchModalView {
    pub(crate) state: SftpSearchModalState,
}

impl EventEmitter<SftpSearchModalEvent> for SftpSearchModalView {}

impl Render for SftpSearchModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

/// SFTP 远程检索模态弹窗状态。
#[derive(Clone)]
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

impl SftpPanelView {
    /// 打开目标远端列的检索模态对话框。
    pub(crate) fn open_search_modal(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((root_path, initial_query)) = self.state.remote_pane(target).map(|pane| {
            (
                pane.path.clone(),
                pane.filter_input.read(cx).value().trim().to_string(),
            )
        }) else {
            return;
        };
        let side_connection = match target {
            SftpRemotePaneRef::Main => None,
            SftpRemotePaneRef::Side(connection_id) => Some(connection_id),
        };
        let placeholder = rust_i18n::t!("sftp.search.placeholder").to_string();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        // 从内联过滤框按回车升级为远程检索时，保留用户已经输入的关键字，
        // 避免用户再重复输入一次。
        if !initial_query.is_empty() {
            input.update(cx, |state, cx| state.set_value(initial_query, window, cx));
        }
        input.update(cx, |state, cx| state.focus(window, cx));
        cx.emit(crate::sftp::SftpPanelEvent::OpenSearchModal(Box::new(
            SftpSearchModalState {
                tab_id: self.tab_id,
                connection_id: side_connection,
                root_path,
                input,
                is_content: false,
                case_sensitive: false,
                is_searching: false,
                results: Vec::new(),
                error: None,
            },
        )));
        cx.notify();
    }
}

impl SftpSearchModalView {
    pub(crate) fn new(state: SftpSearchModalState) -> Self {
        Self { state }
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pattern = self.state.input.read(cx).value().trim().to_string();
        if pattern.is_empty() {
            self.state.error = Some(rust_i18n::t!("sftp.search.error_empty").to_string());
            self.state.input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        self.state.error = None;
        self.state.is_searching = true;
        self.state.results.clear();

        let query = RemoteSearchQuery {
            path: self.state.root_path.clone(),
            pattern,
            is_content: self.state.is_content,
            case_sensitive: self.state.case_sensitive,
            max_depth: Some(10),
            max_results: 200,
        };
        log::info!(
            "发起远程检索请求，路径: {}，模式: {}，是否内容: {}",
            query.path,
            query.pattern,
            query.is_content
        );
        cx.emit(SftpSearchModalEvent::Execute {
            tab_id: self.state.tab_id,
            connection_id: self.state.connection_id,
            query,
        });
        cx.notify();
    }

    pub(crate) fn apply_results(
        &mut self,
        results: Vec<RemoteSearchResultItem>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.state.is_searching = false;
        self.state.results = results;
        self.state.error = error;
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpSearchModalEvent::Cancel);
    }

    fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let is_searching = self.state.is_searching;
        let header = render_search_header(&self.state, cx, p);

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
        } else if self.state.results.is_empty() {
            results_container = results_container.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.search.no_results").to_string()),
            );
        } else {
            for (idx, item) in self.state.results.iter().enumerate() {
                results_container = results_container.child(search_result_row(
                    idx,
                    item,
                    self.state.tab_id,
                    self.state.connection_id,
                    cx,
                    p,
                ));
            }
        }

        let content = v_flex().gap_3().child(header).child(results_container);

        let footer = h_flex().gap_2().child(
            Button::new("close-sftp-search")
                .ghost()
                .label(rust_i18n::t!("app.close").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx))),
        );

        crate::widgets::dialog_body(
            "sftp-search-modal",
            rust_i18n::t!("sftp.search.title").to_string(),
            None::<String>,
            content,
            Some(footer),
        )
    }
}

/// 检索弹窗头部：根路径、输入框、检索按钮、模式开关与错误提示。
fn render_search_header(
    modal: &SftpSearchModalState,
    cx: &mut Context<SftpSearchModalView>,
    p: &Palette,
) -> AnyElement {
    let root_path = modal.root_path.as_str();
    let is_content = modal.is_content;
    let case_sensitive = modal.case_sensitive;
    let is_searching = modal.is_searching;
    let error = modal.error.clone();
    let header = v_flex()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.search.root", path = root_path).to_string()),
        )
        .child(
            h_flex()
                .gap_2()
                .w_full()
                .child(Input::new(&modal.input).w_full())
                .child(
                    Button::new("btn-trigger-search")
                        .primary()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::SEARCH)).small())
                        .label(if is_searching {
                            rust_i18n::t!("sftp.search.searching_short").to_string()
                        } else {
                            rust_i18n::t!("sftp.search.search").to_string()
                        })
                        .disabled(is_searching)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.submit(window, cx);
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
                            this.state.is_content = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("chk-case-sensitive")
                        .checked(case_sensitive)
                        .label(rust_i18n::t!("sftp.search.case_sensitive").to_string())
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.state.case_sensitive = *checked;
                            cx.notify();
                        })),
                ),
        )
        .when_some(error, |this, err| {
            this.child(div().text_xs().text_color(p.danger).child(err))
        });
    header.into_any_element()
}

/// 单条检索结果：路径、行号、匹配内容与跳转。
fn search_result_row(
    idx: usize,
    item: &RemoteSearchResultItem,
    tab_id: TabId,
    connection_id: Option<ConnectionId>,
    cx: &mut Context<SftpSearchModalView>,
    p: &Palette,
) -> AnyElement {
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
        .hover(|s| s.bg(p.surface.hover))
        .cursor_pointer()
        .child(
            AppIconElement::new(AppIcon::for_file(&target_path))
                .with_rems(rems(crate::theme::tokens::control::ICON)),
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
        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
            let dir = parent_remote(&target_path);
            cx.emit(SftpSearchModalEvent::Jump {
                tab_id,
                connection_id,
                dir,
            });
        }));
    row.into_any_element()
}
