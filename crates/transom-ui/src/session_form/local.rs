//! 会话表单中的本地终端连接配置渲染。

use crate::palette::Palette;
use crate::session_form::{SessionForm, SessionFormView};
use crate::widgets::menu_item;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

pub(crate) fn render_local_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let mut fields = v_flex().gap_3();

    // 1. 工作目录（含浏览选择按钮）
    let working_dir_row = h_flex()
        .gap_2()
        .items_end()
        .child(div().flex_1().child(crate::widgets::form_field(
            rust_i18n::t!("form.local.working_dir").to_string(),
            form.local_working_dir.clone(),
            p,
        )))
        .child(
            Button::new("browse-local-working-dir")
                .ghost()
                .small()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
                .label(rust_i18n::t!("form.browse").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.pick_working_dir(window, cx);
                })),
        );
    fields = fields.child(working_dir_row);

    // 2. Shell 程序与系统探测候选下拉
    let shell_input = div().flex_1().child(crate::widgets::form_field(
        rust_i18n::t!("form.local.shell").to_string(),
        form.local_shell.clone(),
        p,
    ));

    let shell_row = if !form.available_shells.is_empty() {
        let shells = form.available_shells.clone();
        let shell_input_handle = form.local_shell.clone();
        let choose_btn = Button::new("choose-installed-shell")
            .ghost()
            .small()
            .child(AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL)).small())
            .label(rust_i18n::t!("form.local.choose_shell").to_string())
            .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _window, _cx| {
                let mut menu = menu.max_h(px(240.0)).scrollable(true);
                for shell in &shells {
                    let prog = shell.program.clone();
                    let label = shell.label.clone();
                    let input = shell_input_handle.clone();
                    menu = menu.item(menu_item(
                        format!("{label} ({prog})"),
                        AppIcon::Domain(DomainIcon::TERMINAL),
                        move |_, window, cx| {
                            input.update(cx, |state, cx| state.set_value(&prog, window, cx));
                        },
                    ));
                }
                menu
            });
        h_flex()
            .gap_2()
            .items_end()
            .child(shell_input)
            .child(choose_btn)
    } else {
        h_flex().items_end().child(shell_input)
    };
    fields = fields.child(shell_row);

    // 3. Shell 启动参数 与 启动执行命令
    fields = fields.child(
        h_flex()
            .gap_3()
            .items_end()
            .child(div().flex_1().child(crate::widgets::form_field(
                rust_i18n::t!("form.local.shell_args").to_string(),
                form.local_shell_args.clone(),
                p,
            )))
            .child(div().flex_1().child(crate::widgets::form_field(
                rust_i18n::t!("form.local.command").to_string(),
                form.local_command.clone(),
                p,
            ))),
    );

    // 4. Windows 管理员权限
    fields = fields.child(
        h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .child(
                v_flex()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("form.local.run_as_admin").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.local.run_as_admin_hint").to_string()),
                    ),
            )
            .child(
                Switch::new("toggle-local-run-as-admin")
                    .small()
                    .checked(form.local_run_as_admin)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.edit(cx, |form| form.local_run_as_admin = *checked);
                    })),
            ),
    );

    // 5. 环境变量
    fields = fields.child(
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.local.env").to_string()),
            )
            .child(Textarea::new(&form.local_env).h(px(88.0)).w_full())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.local.env_hint").to_string()),
            ),
    );

    fields.into_any_element()
}
