//! 会话表单基本信息卡片与文件夹选择。

use super::*;
use crate::session_form::SessionFormEvent;

impl SessionFormView {
    pub(super) fn render_session_basic_card(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
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
                    .child(div().flex_1().child(self.render_folder_field(form, p, cx))),
            )
            .child(self.render_session_appearance(form.icon, form.color, p, cx))
            .into_any_element()
    }

    /// 文件夹字段：可输入，也可从既有目录里选一个，不做搜索/过滤。
    ///
    /// 文件夹本质是自由路径，候选只是既有目录的快捷方式，所以没有拆成
    /// "只读选择器 + 隐藏输入"：输入框始终可编辑，Popover 只负责把候选
    /// 挂在下方。展开时把焦点交给输入框自己的 FocusHandle，避免弹层抢走
    /// 焦点打断键入；候选保持完整清单，开始键入路径时收起弹层（见
    /// `SessionFormView::install`），不把输入当成过滤词。
    pub(super) fn render_folder_field(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = form.folder_options.clone();
        let focus = form.folder_path.read(cx).focus_handle(cx);
        let content_entity = cx.entity();
        let content_palette = p.clone();

        let popover = Popover::new("session-form-folder-popover")
            .anchor(Anchor::TopLeft)
            .offset(px(4.0))
            .open(form.folder_open)
            .track_focus(&focus)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                this.set_folder_open(*open, cx);
            }))
            .trigger_style(StyleRefinement::default().w_full())
            .trigger(
                Input::new(&form.folder_path).w_full().suffix(
                    AppIconElement::new(AppIcon::Ui(UiIcon::SORT_BY))
                        .xsmall()
                        .text_color(p.muted_foreground),
                ),
            )
            .content(move |_, _, _cx| {
                Self::render_folder_options(&content_entity, &options, &content_palette)
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
    ///
    /// 候选项的点击回调拿不到 `Context<Self>`（Popover 内容闭包只有 `&mut App`），
    /// 因此捕获实体句柄后经 `update` 回到实体；目录默认值由外壳在
    /// [`SessionFormEvent::SelectFolder`] 处理时解析并注入。
    fn render_folder_options(
        entity: &Entity<SessionFormView>,
        options: &[String],
        p: &Palette,
    ) -> AnyElement {
        let root_entity = entity.clone();
        let mut list = v_flex()
            .min_w(px(240.0))
            .gap_0p5()
            .child(Self::folder_option_row(
                "session-folder-root".to_string(),
                AppIcon::Ui(UiIcon::SHOW_AS_TREE),
                rust_i18n::t!("folder.root_level").to_string(),
                p,
                move |_: &ClickEvent, window, cx| {
                    root_entity.update(cx, |this, cx| {
                        this.clear_folder(window, cx);
                        this.set_folder_open(false, cx);
                    });
                },
            ));
        if !options.is_empty() {
            list = list.child(div().h(px(1.0)).my_1().bg(p.surface.contrast));
        }
        for path in options {
            let pick_entity = entity.clone();
            let picked = path.clone();
            list = list.child(Self::folder_option_row(
                format!("session-folder-{path}"),
                AppIcon::Ui(UiIcon::FOLDER),
                path.clone(),
                p,
                move |_: &ClickEvent, _window, cx| {
                    let picked = picked.clone();
                    pick_entity.update(cx, |this, cx| {
                        this.set_folder_open(false, cx);
                        cx.emit(SessionFormEvent::SelectFolder { path: picked });
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
        icon: AppIcon,
        label: String,
        p: &Palette,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
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
            .child(
                AppIconElement::new(icon)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(div().text_sm().text_color(p.foreground).child(label))
            .on_click(on_click)
            .into_any_element()
    }
}
