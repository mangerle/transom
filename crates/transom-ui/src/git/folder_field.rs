//! 分组字段：可输入组名，也可从既有分组里选（与会话表单的文件夹字段同一套控件）。
//!
//! 设计原理（Why）：
//! - 用 Popover + 可编辑输入框，而不是把分组平铺成一排按钮：分组数量会长，
//!   平铺会把弹窗挤成一整行按钮；而「新建一个组」本来就是打字，输入框比
//!   「先点新建再填名字」少一步；
//! - 输入的名字不在既有分组里就是新建分组（确认时才真的建），与会话表单里
//!   「输入新路径就建目录」同一口径，因此这里不再另设「新建分组」按钮；
//! - 清空输入 = 不归组，候选第一项固定是「不归组」；
//! - 确认时才建组：用户在输入框里打了一半字不该就在侧栏里多出一行空组。

use crate::app::AppShell;
use crate::git::form::{GitFolderField, GitHost};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::*;
use transom_core::GitFolder;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染分组字段：标签 + 可输入的选择框。
///
/// `fixed_width` 给定时把触发器卡在这档宽度：克隆弹窗要和上方的图标行左右对齐，
/// 不能因为输入框内容/弹层内容把字段撑得比图标行长；`None` 时保持自适应
/// （「打开本机仓库」的 2×2 网格用这一档）。
pub(crate) fn render_git_folder_field(
    field: &GitFolderField,
    host: GitHost,
    options: &[String],
    fixed_width: Option<f32>,
    p: &Palette,
    app: &Entity<AppShell>,
    cx: &App,
) -> AnyElement {
    let focus = field.name.read(cx).focus_handle(cx);
    let toggle_app = app.clone();
    let content_app = app.clone();
    let content_palette = p.clone();
    let content_host = host;
    let options = options.to_vec();
    let mut trigger_style = StyleRefinement::default().w_full();
    if let Some(width) = fixed_width {
        trigger_style = trigger_style.min_w(px(width)).max_w(px(width));
    }

    let popover = Popover::new("git-folder-popover")
        .anchor(Anchor::TopLeft)
        .offset(px(4.0))
        .open(field.open)
        .track_focus(&focus)
        .on_open_change(move |open, _window, cx| {
            let open = *open;
            toggle_app.update(cx, |this, cx| {
                this.git_folder_set_open(host, open, cx);
            });
        })
        .trigger_style(trigger_style)
        .trigger(
            Input::new(&field.name).w_full().suffix(
                AppIconElement::new(AppIcon::Ui(UiIcon::SORT_BY))
                    .xsmall()
                    .text_color(p.muted_foreground),
            ),
        )
        .content(move |_, _, _cx| {
            render_folder_options(&content_app, content_host, &options, &content_palette)
        });

    v_flex()
        .w_full()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.project.folder_label").to_string()),
        )
        .child(popover)
        .into_any_element()
}

/// 候选列表：「不归组」固定置顶，其余是全部既有分组。
fn render_folder_options(
    app: &Entity<AppShell>,
    host: GitHost,
    options: &[String],
    p: &Palette,
) -> AnyElement {
    let root_app = app.clone();
    let mut list = v_flex().min_w(px(240.0)).gap_0p5().child(folder_option_row(
        "git-folder-root",
        AppIcon::Ui(UiIcon::SHOW_AS_TREE),
        rust_i18n::t!("git.project.folder_root").to_string(),
        p,
        {
            let app = root_app.clone();
            move |window, cx| {
                app.update(cx, |this, cx| {
                    this.git_folder_pick(host, String::new(), window, cx);
                });
            }
        },
    ));
    if !options.is_empty() {
        list = list.child(div().h(px(1.0)).my_1().bg(p.surface.contrast));
    }
    for name in options {
        let pick_app = app.clone();
        let picked = name.clone();
        list = list.child(folder_option_row(
            format!("git-folder-{name}"),
            AppIcon::Ui(UiIcon::FOLDER),
            name.clone(),
            p,
            move |window, cx| {
                pick_app.update(cx, |this, cx| {
                    this.git_folder_pick(host, picked.clone(), window, cx);
                });
            },
        ));
    }
    if options.is_empty() {
        list = list.child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.folder.none_yet").to_string()),
        );
    }
    list.into_any_element()
}

/// 一行候选。
fn folder_option_row(
    id: impl Into<String>,
    icon: AppIcon,
    label: String,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(ElementId::Name(SharedString::from(id.into())))
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .cursor_pointer()
        .rounded(p.radius.sm)
        .hover(|style| style.bg(p.list_hover))
        .child(
            AppIconElement::new(icon)
                .xsmall()
                .text_color(p.muted_foreground),
        )
        .child(div().flex_1().min_w_0().truncate().text_xs().child(label))
        .on_click(move |_, window, cx| on_click(window, cx))
        .into_any_element()
}

impl AppShell {
    /// 按宿主取到分组字段（可变）。
    pub(crate) fn folder_field_mut(&mut self, host: GitHost) -> Option<&mut GitFolderField> {
        match host {
            GitHost::Modal => self
                .state
                .modals
                .git
                .project
                .as_mut()
                .map(|modal| &mut modal.folder),
            GitHost::Clone => self
                .state
                .modals
                .git
                .clone
                .as_mut()
                .map(|modal| &mut modal.folder),
        }
    }

    /// 展开或收起分组候选。
    pub(crate) fn git_folder_set_open(
        &mut self,
        host: GitHost,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.folder_field_mut(host) else {
            return;
        };
        if field.open != open {
            field.open = open;
            cx.notify();
        }
    }

    /// 选中一个分组（空串表示不归组）：写回输入框并收起候选。
    pub(crate) fn git_folder_pick(
        &mut self,
        host: GitHost,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.folder_field_mut(host) else {
            return;
        };
        field.open = false;
        field.name.update(cx, |state, cx| {
            state.set_value(name, window, cx);
        });
        cx.notify();
    }

    /// 分组候选：既有分组的名字。
    pub(crate) fn git_folder_names(&self) -> Vec<String> {
        self.state
            .git
            .folders
            .iter()
            .map(|folder| folder.name.clone())
            .collect()
    }

    /// 把输入框里的组名落成真实分组：空 → 不归组；已有同名 → 用它；
    /// 新名字 → 现在就建组（确认时才建，打字过程中不落盘）。
    pub(crate) fn git_folder_id_for_name(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Option<transom_core::GitFolderId> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(existing) = super::helpers::find_folder_by_name(&self.state.git.folders, name) {
            return Some(existing);
        }
        log::info!("按表单输入新建 Git 分组: {name}");
        self.state.git.folders.push(GitFolder::new(name));
        let created = self.state.git.folders.last().map(|folder| folder.id);
        self.persist_git_projects(cx);
        created
    }
}
