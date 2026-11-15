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

use crate::git::form::GitFolderField;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 分组字段渲染参数。
pub(crate) struct GitFolderFieldProps<'a, V> {
    pub options: &'a [String],
    pub fixed_width: Option<f32>,
    pub palette: &'a Palette,
    pub entity: &'a Entity<V>,
    pub access: fn(&mut V) -> &mut GitFolderField,
}

/// 渲染分组字段：标签 + 可输入的选择框。
///
/// `fixed_width` 给定时把触发器卡在这档宽度：克隆弹窗要和上方的图标行左右对齐，
/// 不能因为输入框内容/弹层内容把字段撑得比图标行长；`None` 时保持自适应
/// （「打开本机仓库」的 2×2 网格用这一档）。
pub(crate) fn render_git_folder_field<V: 'static>(
    field: &GitFolderField,
    props: GitFolderFieldProps<'_, V>,
    cx: &App,
) -> AnyElement {
    let focus = field.name.read(cx).focus_handle(cx);
    let p = props.palette;
    let toggle_entity = props.entity.clone();
    let content_entity = props.entity.clone();
    let content_palette = p.clone();
    let options = props.options.to_vec();
    let access = props.access;
    let mut trigger_style = StyleRefinement::default().w_full();
    if let Some(width) = props.fixed_width {
        trigger_style = trigger_style.min_w(px(width)).max_w(px(width));
    }

    let popover = Popover::new("git-folder-popover")
        .anchor(Anchor::TopLeft)
        .offset(px(4.0))
        .open(field.open)
        .track_focus(&focus)
        .on_open_change(move |open, _window, cx| {
            let open = *open;
            toggle_entity.update(cx, |this, cx| {
                let field = access(this);
                if field.open != open {
                    field.open = open;
                    cx.notify();
                }
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
            render_folder_options(&content_entity, access, &options, &content_palette)
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
fn render_folder_options<V: 'static>(
    entity: &Entity<V>,
    access: fn(&mut V) -> &mut GitFolderField,
    options: &[String],
    p: &Palette,
) -> AnyElement {
    let root_entity = entity.clone();
    let mut list = v_flex().min_w(px(240.0)).gap_0p5().child(folder_option_row(
        "git-folder-root",
        AppIcon::Ui(UiIcon::SHOW_AS_TREE),
        rust_i18n::t!("git.project.folder_root").to_string(),
        p,
        move |window, cx| {
            root_entity.update(cx, |this, cx| {
                access(this).pick(String::new(), window, cx);
            });
        },
    ));
    if !options.is_empty() {
        list = list.child(div().h(px(1.0)).my_1().bg(p.surface.contrast));
    }
    for name in options {
        let pick_entity = entity.clone();
        let picked = name.clone();
        list = list.child(folder_option_row(
            format!("git-folder-{name}"),
            AppIcon::Ui(UiIcon::FOLDER),
            name.clone(),
            p,
            move |window, cx| {
                pick_entity.update(cx, |this, cx| {
                    access(this).pick(picked.clone(), window, cx);
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
