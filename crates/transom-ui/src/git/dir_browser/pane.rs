//! 目录树面板的两行工具条与画行。
//!
//! 设计原理（Why）：渲染单独成文件是因为它只读面板状态，动作全部经 `cx.listener`
//! 回到 `AppShell`；画行与状态改动放在一起会让「谁在什么时候改状态」看不清。

use super::listing::GitDirEntry;
use super::rows::{GitRow, RowBody, build_rows};
use super::{GitDirBrowser, GitHost};
use crate::app::AppShell;
use crate::format::format_size;
use crate::git::dir_browser::DirPicker;
use crate::palette::Palette;
use crate::sftp::quick_paths::LocalQuickPaths;
use crate::theme::tokens::{control, size, typography};
use crate::widgets::{IconButton, IconButtonSize};
use gpui_kit::base::h_flex;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

/// 每级缩进（逻辑像素）：与会话侧栏同一档。
const INDENT_STEP: f32 = 16.0;
/// 一行的固定高度：通过 `tokens::size::FILE_ROW_HEIGHT` 与 SFTP 列表统一并随密度缩放。
///
/// 为什么：视口高度取行高的整数倍时，停在偏移 0 的列表上下边缘都落在行的边界上；
/// 行高随字号浮动时，可见区永远切在某一行的中间，看上去就是"上下都被截断"。
const ROW_HEIGHT: f32 = size::FILE_ROW_HEIGHT;

/// 渲染目录树面板。`rows` 是可见行数，由调用方按弹窗高度预算给。
///
/// `show_path` 控制树头里的路径框：项目弹窗没有单独的路径字段，必须保留；
/// 克隆弹窗上方已有目录输入框，展开树时再画一个就是重复输入。
///
/// 刻意做成不借用 `self` 的函数：调用方此刻正持有 `self.state.modals.git.project` 的
/// 不可变借用；动作全部通过 `app.update` 回调回写状态。
pub(crate) fn render_git_browser(
    picker: &DirPicker,
    target: GitHost,
    rows: usize,
    show_path: bool,
    p: &Palette,
    app: &Entity<AppShell>,
    _cx: &App,
) -> AnyElement {
    let quick_paths = &picker.browser.quick_paths;
    let mut head = v_flex()
        .w_full()
        .gap_1()
        .p_1()
        .bg(p.muted)
        .border_b_1()
        .border_color(p.border);
    if show_path {
        head = head.child(nav_row(picker));
    }
    let head = head.child(actions_row(picker, target, quick_paths, app));
    v_flex()
        .w_full()
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.border)
        .bg(p.background)
        // 头部（路径框 + 工具条）用 `muted` 底色并压一条下边线，与下面的目录列表
        // 分开：底色一致时看不出哪一块是可点的目录、哪一块是输入与动作。
        .overflow_hidden()
        .child(head)
        .child(render_tree(picker, target, rows, p, app))
        .into_any_element()
}

/// 第一行：只剩路径框。
///
/// 上一级不做按钮——树里父级就在上面一行，滚过去就是；输入的完整路径由底部
/// 确认按钮直接采用，也不需要「进入」这一步。
fn nav_row(picker: &DirPicker) -> AnyElement {
    // 只剩路径框：上一级在树里就是往上看一行，不需要按钮；输入的完整路径
    // 由底部确认按钮直接采用，也不需要「进入」这一步。
    div()
        .w_full()
        .child(Input::new(&picker.path).w_full())
        .into_any_element()
}

/// 第二行：快捷目录 + 新建目录 + 显示隐藏条目 + 刷新。
fn actions_row(
    picker: &DirPicker,
    target: GitHost,
    quick_paths: &LocalQuickPaths,
    app: &Entity<AppShell>,
) -> AnyElement {
    let mkdir_app = app.clone();
    let hidden_app = app.clone();
    let refresh_app = app.clone();
    h_flex()
        .gap_1()
        .items_center()
        .child(quick_path_button(
            "git-browser-home",
            target,
            AppIcon::Ui(UiIcon::HOME_FOLDER),
            "sftp.home",
            quick_paths.home.clone(),
            app,
        ))
        .child(quick_path_button(
            "git-browser-desktop",
            target,
            AppIcon::Ui(UiIcon::DESKTOP),
            "sftp.desktop",
            quick_paths.desktop.clone(),
            app,
        ))
        .child(quick_path_button(
            "git-browser-downloads",
            target,
            AppIcon::Ui(UiIcon::DOWNLOAD),
            "sftp.downloads",
            quick_paths.downloads.clone(),
            app,
        ))
        .child(
            IconButton::new(
                "git-browser-mkdir",
                UiIcon::NEW_FOLDER,
                rust_i18n::t!("git.browser.mkdir").to_string(),
            )
            .render()
            .on_click(move |_, window, cx| {
                mkdir_app.update(cx, |this, cx| {
                    this.git_start_dir_draft(target, window, cx);
                });
            }),
        )
        .child(
            IconButton::new(
                "git-browser-hidden",
                if picker.browser.show_hidden {
                    AppIcon::Ui(UiIcon::SHOW)
                } else {
                    AppIcon::Custom(CustomIcon::EYE_OFF)
                },
                if picker.browser.show_hidden {
                    rust_i18n::t!("sftp.nav.hide_dotfiles").to_string()
                } else {
                    rust_i18n::t!("sftp.nav.show_dotfiles").to_string()
                },
            )
            .selected(picker.browser.show_hidden)
            .render()
            .on_click(move |_, _, cx| {
                hidden_app.update(cx, |this, cx| this.git_toggle_hidden(target, cx));
            }),
        )
        .child(
            IconButton::new(
                "git-browser-refresh",
                UiIcon::REFRESH,
                rust_i18n::t!("sftp.nav.refresh_local_dir").to_string(),
            )
            .render()
            .on_click(move |_, _, cx| {
                refresh_app.update(cx, |this, cx| this.git_refresh_browser(target, cx));
            }),
        )
        .into_any_element()
}

/// 快捷目录按钮：目录不存在时置灰而不是隐藏，用户才知道「这个入口有，只是本机没有」。
fn quick_path_button(
    id: &'static str,
    target: GitHost,
    icon: AppIcon,
    tooltip_key: &'static str,
    path: Option<PathBuf>,
    app: &Entity<AppShell>,
) -> AnyElement {
    let click_app = app.clone();
    IconButton::new(
        ElementId::Name(id.into()),
        icon,
        rust_i18n::t!(tooltip_key).to_string(),
    )
    .disabled(path.is_none())
    .render()
    .on_click(move |_, window, cx| {
        click_app.update(cx, |this, cx| {
            this.git_browse_quick(target, path.clone(), window, cx);
        });
    })
    .into_any_element()
}

/// 树本体：把展开中的层级扁平化后逐行画出来；根目录空且已读完时补一句空态。
fn render_tree(
    picker: &DirPicker,
    target: GitHost,
    rows: usize,
    p: &Palette,
    app: &Entity<AppShell>,
) -> AnyElement {
    let container = v_flex()
        .w_full()
        .h(rems(ROW_HEIGHT * rows as f32))
        // 元素 id 带上根路径：gpui 的滚动偏移存在按 id 归位的元素状态里，静态 id 会让
        // 换根后的新列表沿用旧偏移——停在半行上的表现就是"第一行被切掉一截"。
        .id(ElementId::Name(
            format!("git-browser-list-{}", picker.browser.root.display()).into(),
        ))
        .overflow_y_scrollbar()
        .rounded(p.radius.sm);
    let mut list = build_rows(&picker.browser)
        .into_iter()
        .fold(container, |list, row| {
            list.child(render_row(row, &picker.browser, target, p, app))
        });
    let root_listing = picker.browser.listed.get(&picker.browser.root);
    let empty = root_listing
        .as_ref()
        .is_some_and(|listing| !listing.loading && listing.entries.is_empty());
    let failed = root_listing
        .as_ref()
        .is_some_and(|listing| listing.error.is_some());
    if empty && !failed {
        list = list.child(plain_note(
            rust_i18n::t!("git.browser.empty").to_string(),
            p,
        ));
    }
    list.into_any_element()
}

/// 一行：条目行、说明行、行内新建目录的草稿行。
///
/// 三角只管展开/收起，行身只管选中，文件行两者都没有。
fn render_row(
    row: GitRow,
    browser: &GitDirBrowser,
    target: GitHost,
    p: &Palette,
    app: &Entity<AppShell>,
) -> AnyElement {
    let depth = row.depth;
    match row.body {
        RowBody::Note(text) => indented_note(text, depth, p),
        RowBody::Draft => draft_row(browser, target, depth, p, app),
        RowBody::Entry { entry, expanded } => {
            entry_row(entry, depth, expanded, &browser.selected, target, p, app)
        }
    }
}

/// 条目行：目录可展开可选，文件只作展示。
fn entry_row(
    entry: GitDirEntry,
    depth: usize,
    expanded: bool,
    selected: &Option<PathBuf>,
    target: GitHost,
    p: &Palette,
    app: &Entity<AppShell>,
) -> AnyElement {
    let indent = px(4.0 + depth as f32 * INDENT_STEP);
    let band = h_flex()
        .id(ElementId::Name(
            format!("git-tree-{}", entry.path.display()).into(),
        ))
        .h(rems(ROW_HEIGHT))
        .pl(indent)
        .pr_2()
        .gap_1()
        .items_center()
        .rounded(p.radius.sm);
    if !entry.is_dir {
        // 目录与文件同排显示，但文件行不给任何点击行为：选仓库要的是目录。
        return band
            .child(div().flex_none().size(rems(size::TREE_CHEVRON)))
            .child(
                AppIconElement::new(AppIcon::for_file(&entry.path.to_string_lossy()))
                    .with_rems(rems(control::ICON)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(rems(typography::BODY))
                    .text_color(p.text.subtle)
                    .child(entry.name),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.text.subtle)
                    .child(format_size(entry.size)),
            )
            .into_any_element();
    }
    let is_selected = selected.as_ref() == Some(&entry.path);
    let path = entry.path.clone();
    let select_app = app.clone();
    let select_path = path.clone();
    let mut band = band
        .cursor_pointer()
        .hover(|style| style.bg(p.list_hover))
        .text_color(if is_selected {
            p.sidebar_accent_foreground
        } else {
            p.sidebar_foreground
        })
        .child(chevron(&path, expanded, target, p, app))
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER))
                .with_rems(rems(control::ICON))
                .text_color(p.primary),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::MEDIUM)
                .child(entry.name),
        )
        .on_click(move |event: &ClickEvent, window, cx| {
            let path = select_path.clone();
            let is_double_click = event.click_count() >= 2;
            select_app.update(cx, |this, cx| {
                this.git_select_dir(target, path, window, cx);
                // 双击落在目录行上视为「选好了」：克隆弹窗把目录树收起，
                // 项目弹窗的树是主内容，保持展开。
                if is_double_click {
                    this.git_close_dir_tree(target, cx);
                }
            });
        });
    if is_selected {
        // 选中态用列表激活色：只靠文字颜色区分，浅主题下几乎看不出来。
        band = band.bg(p.list_active);
    }
    band.into_any_element()
}

/// 行内新建目录：一个输入框 + 确认 / 取消，位置就在它所属那一层的末尾。
///
/// 为什么不靠回车提交：焦点在单行输入框里时，回车与 Esc 都被输入框吃掉，
/// 容器与遮罩层收不到这两个键（详见 dialog 容器迁移的实测记录）。所以确认与
/// 取消各给一个可见按钮，键位口径等弹窗整体迁到 Dialog 容器时再统一。
fn draft_row(
    browser: &GitDirBrowser,
    target: GitHost,
    depth: usize,
    p: &Palette,
    app: &Entity<AppShell>,
) -> AnyElement {
    let Some(draft) = browser.draft.as_ref() else {
        return div().into_any_element();
    };
    let input = draft.input.clone();
    let ok_app = app.clone();
    let cancel_app = app.clone();
    h_flex()
        .id("git-tree-draft")
        .pl(px(4.0 + depth as f32 * INDENT_STEP))
        .pr_2()
        .py_1()
        .gap_1()
        .items_center()
        .rounded(p.radius.sm)
        .bg(p.list_active)
        .text_color(p.sidebar_accent_foreground)
        .child(div().flex_none().size(rems(size::TREE_CHEVRON)))
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER))
                .with_rems(rems(control::ICON))
                .text_color(p.sidebar_accent_foreground),
        )
        .child(div().flex_1().min_w_0().child(Input::new(&input).w_full()))
        .child(
            IconButton::new(
                "git-draft-ok",
                UiIcon::CHECK,
                rust_i18n::t!("git.name.confirm").to_string(),
            )
            .size(IconButtonSize::RowAction)
            .render()
            .on_click(move |_, window, cx| {
                ok_app.update(cx, |this, cx| {
                    this.git_commit_dir_draft(target, window, cx);
                });
            }),
        )
        .child(
            IconButton::new(
                "git-draft-cancel",
                UiIcon::CLOSE,
                rust_i18n::t!("git.project.cancel").to_string(),
            )
            .size(IconButtonSize::RowAction)
            .render()
            .on_click(move |_, _, cx| {
                cancel_app.update(cx, |this, cx| {
                    this.git_cancel_dir_draft(target, cx);
                });
            }),
        )
        .into_any_element()
}

/// 说明行：缩进跟它所属的父级一致，不带图标、不可点击。
///
/// 高度与条目行同档：说明行夹在条目之间，两种行高不一致会把整列的网格错开。
fn indented_note(text: String, depth: usize, p: &Palette) -> AnyElement {
    h_flex()
        .h(rems(ROW_HEIGHT))
        .pl(px(4.0 + depth as f32 * INDENT_STEP))
        .pr_2()
        .items_center()
        .text_size(rems(typography::CAPTION))
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}

/// 行首三角：点击不冒泡到行身，避免「展开顺带选中」。
fn chevron(
    path: &Path,
    expanded: bool,
    host: GitHost,
    p: &Palette,
    app: &Entity<AppShell>,
) -> AnyElement {
    let path = path.to_path_buf();
    let click_app = app.clone();
    div()
        .id(ElementId::Name(
            format!("git-tree-chevron-{}", path.display()).into(),
        ))
        .flex_none()
        .size(rems(size::TREE_CHEVRON))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(
            AppIconElement::new(if expanded {
                AppIcon::Ui(UiIcon::CHEVRON_DOWN)
            } else {
                AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
            })
            .with_rems(rems(size::TREE_CHEVRON))
            .text_color(p.muted_foreground),
        )
        .on_click(move |_: &ClickEvent, _, cx| {
            cx.stop_propagation();
            let path = path.clone();
            click_app.update(cx, |this, cx| {
                this.git_toggle_tree_dir(host, path, cx);
            });
        })
        .into_any_element()
}

/// 一句独立说明（空态）：与条目同字号，不带图标、不可点击。
fn plain_note(text: String, p: &Palette) -> AnyElement {
    indented_note(text, 0, p)
}
