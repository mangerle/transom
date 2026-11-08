//! 会话表单基本信息卡片与文件夹选择。

use super::*;

impl AppShell {
    pub(super) fn render_session_basic_card(
        form: &crate::session_form::types::SessionForm,
        app: &Entity<AppShell>,
        p: &Palette,
        cx: &App,
    ) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("form.name").to_string(),
                        form.name.clone(),
                        p,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .child(Self::render_folder_field(form, p, app, cx)),
                    ),
            )
            .child(Self::render_session_appearance(
                default_icon_for(form.connection),
                form.icon,
                form.color,
                p,
                app,
                cx,
            ))
            .into_any_element()
    }

    /// 文件夹字段：可输入，也可从既有目录里选一个，不做搜索/过滤。
    ///
    /// 文件夹本质是自由路径，候选只是既有目录的快捷方式，所以没有拆成
    /// "只读选择器 + 隐藏输入"：输入框始终可编辑，Popover 只负责把候选
    /// 挂在下方。展开时把焦点交给输入框自己的 FocusHandle，避免弹层抢走
    /// 焦点打断键入；候选保持完整清单，开始键入路径时收起弹层（见
    /// install_session_form），不把输入当成过滤词。
    pub(super) fn render_folder_field(
        form: &crate::session_form::types::SessionForm,
        p: &Palette,
        app: &Entity<AppShell>,
        cx: &App,
    ) -> AnyElement {
        let options = form.folder_options.clone();
        let focus = form.folder_path.read(cx).focus_handle(cx);
        let toggle_app = app.clone();
        let content_app = app.clone();
        let content_palette = p.clone();

        let popover = Popover::new("session-form-folder-popover")
            .anchor(Anchor::TopLeft)
            .offset(px(4.0))
            .open(form.folder_open)
            .track_focus(&focus)
            .on_open_change(move |open, _window, cx| {
                let open = *open;
                toggle_app.update(cx, |this, cx| {
                    this.update_session_form(cx, |form| form.folder_open = open);
                });
            })
            .trigger_style(StyleRefinement::default().w_full())
            .trigger(
                Input::new(&form.folder_path).w_full().suffix(
                    Icon::new(IconName::ChevronsUpDown)
                        .xsmall()
                        .text_color(p.muted_foreground),
                ),
            )
            .content(move |_, _, _cx| {
                Self::render_folder_options(&content_app, &options, &content_palette)
            });

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.folder").to_string()),
            )
            .child(popover)
            .into_any_element()
    }

    /// 文件夹候选列表：根层级固定置顶，其余是全部既有路径。
    fn render_folder_options(
        app: &Entity<AppShell>,
        options: &[String],
        p: &Palette,
    ) -> AnyElement {
        let root_app = app.clone();
        let mut list = v_flex()
            .min_w(px(240.0))
            .gap_0p5()
            .child(Self::folder_option_row(
                "session-folder-root".to_string(),
                IconName::FolderTree,
                rust_i18n::t!("folder.root_level").to_string(),
                p,
                move |window, cx| {
                    root_app.update(cx, |this, cx| {
                        this.clear_form_folder(window, cx);
                        this.close_form_folder_menu(cx);
                    });
                },
            ));
        if !options.is_empty() {
            list = list.child(div().h(px(1.0)).my_1().bg(p.foreground.opacity(0.08)));
        }
        for path in options {
            let pick_app = app.clone();
            let picked = path.clone();
            list = list.child(Self::folder_option_row(
                format!("session-folder-{path}"),
                IconName::Folder,
                path.clone(),
                p,
                move |window, cx| {
                    pick_app.update(cx, |this, cx| {
                        this.select_form_folder(&picked, window, cx);
                        this.close_form_folder_menu(cx);
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
                    .child(rust_i18n::t!("form.folder_empty").to_string()),
            );
        }
        div()
            .max_h(px(240.0))
            .overflow_y_scrollbar()
            .child(list)
            .into_any_element()
    }

    /// 文件夹下拉里的一行；点击后的语义（回填或清空）由调用方决定。
    fn folder_option_row(
        id: String,
        icon: IconName,
        label: String,
        p: &Palette,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> AnyElement {
        h_flex()
            .id(ElementId::Name(id.into()))
            .gap_2()
            .items_center()
            .px_2()
            .py_1p5()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|style| style.bg(p.list_hover))
            .child(Icon::new(icon).xsmall().text_color(p.muted_foreground))
            .child(div().text_sm().text_color(p.foreground).child(label))
            .on_click(move |_, window, cx| on_click(window, cx))
            .into_any_element()
    }

    /// 关闭文件夹候选下拉；选中候选或清空后调用，避免弹层继续驻留。
    fn close_form_folder_menu(&mut self, cx: &mut Context<Self>) {
        self.update_session_form(cx, |form| form.folder_open = false);
    }
}
