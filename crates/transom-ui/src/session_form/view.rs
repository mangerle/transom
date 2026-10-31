//! 会话表单弹窗主体外壳渲染与文件选择逻辑。

use super::appearance::default_icon_for;
use super::serial::render_serial_fields;
use super::ssh::render_ssh_fields;
use super::telnet::render_telnet_fields;
use super::types::ConnectionChoice;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::BlockViewOverride;

impl AppShell {
    pub(crate) fn render_session_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.modals.form.as_ref() else {
            return div().into_any_element();
        };
        let title = if form.base.is_some() {
            rust_i18n::t!("form.edit_title").to_string()
        } else {
            rust_i18n::t!("form.new_title").to_string()
        };

        let basic_card = Self::render_session_basic_card(form, app, p, cx);
        let connection_type_card = Self::render_session_connection_type_card(form, app, cx, p);

        let mut content = v_flex()
            .id(ElementId::Name("session-form-body".into()))
            .max_h(px(460.0))
            .overflow_y_scroll()
            .gap_3()
            .child(basic_card)
            .child(connection_type_card);

        match form.connection {
            ConnectionChoice::Ssh => {
                content = content.child(render_ssh_fields(form, app, cx, p));
            }
            ConnectionChoice::Mosh => {
                content = content
                    .child(render_ssh_fields(form, app, cx, p))
                    .child(Self::render_mosh_specific_card(form, p));
            }
            ConnectionChoice::Serial => {
                content = content.child(render_serial_fields(form, app, cx, p));
            }
            ConnectionChoice::Telnet => {
                content = content.child(render_telnet_fields(form, app, cx, p));
            }
        }

        content = content.child(Self::render_session_terminal_attributes_card(
            form, app, p, cx,
        ));

        let editing = form.base.is_some();
        let confirm_delete = form.confirm_delete;
        let mut footer = h_flex().gap_2().items_center().justify_end().w_full();
        if editing {
            if confirm_delete {
                footer = footer.child(
                    Button::new("confirm-delete")
                        .danger()
                        .small()
                        .icon(IconName::Trash)
                        .label(rust_i18n::t!("common.confirm_delete").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, _, cx| this.delete_session_form(cx),
                        )),
                );
            } else {
                footer = footer.child(
                    Button::new("delete-session")
                        .ghost()
                        .small()
                        .icon(IconName::Trash)
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(form) = this.modals.form.as_mut() {
                                    form.confirm_delete = true;
                                }
                                cx.notify();
                            },
                        )),
                );
            }
        }
        footer = footer
            .child(
                Button::new("cancel-session")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.modals.form = None;
                            cx.notify();
                        },
                    )),
            )
            .child(
                Button::new("save-session")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.save_session_form(cx),
                    )),
            );

        let mut footer_block = v_flex().gap_2().items_end();
        if let Some(error) = &form.error {
            footer_block =
                footer_block.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer_block = footer_block.child(footer);

        Self::dialog_body(
            "session-form-modal",
            title,
            None,
            content,
            Some(footer_block),
        )
    }

    /// 用系统文件选择器挑选私钥，起始目录 `~/.ssh`。
    pub(crate) fn pick_private_key_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.modals.form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_key").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.key_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }

    /// 选择 CA 签发的用户证书文件（OpenSSH 惯例是 `私钥名-cert.pub`）。
    pub(crate) fn pick_certificate_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.modals.form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_cert").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .add_filter(rust_i18n::t!("form.cert_filter").to_string(), &["pub"])
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.cert_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }

    /// 会话基本信息卡片（名称、所属文件夹、外观图标与颜色）。
    fn render_session_basic_card(
        form: &super::types::SessionForm,
        app: &Entity<AppShell>,
        p: &Palette,
        cx: &App,
    ) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(Self::form_field(
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
    fn render_folder_field(
        form: &super::types::SessionForm,
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
                    if let Some(form) = this.modals.form.as_mut()
                        && form.folder_open != open
                    {
                        form.folder_open = open;
                        cx.notify();
                    }
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
        if let Some(form) = self.modals.form.as_mut() {
            form.folder_open = false;
        }
        cx.notify();
    }

    /// 会话连接类型切换卡片（SSH、串口、Telnet）。
    fn render_session_connection_type_card(
        form: &super::types::SessionForm,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.connection_type").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("conn-ssh")
                            .small()
                            .label("SSH")
                            .selected(form.connection == ConnectionChoice::Ssh)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.connection = ConnectionChoice::Ssh;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        Button::new("conn-serial")
                            .small()
                            .label(rust_i18n::t!("form.conn_serial").to_string())
                            .selected(form.connection == ConnectionChoice::Serial)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, window, cx| {
                                    let needs_ports = if let Some(form) = this.modals.form.as_mut()
                                    {
                                        form.connection = ConnectionChoice::Serial;
                                        form.serial_ports.is_empty()
                                    } else {
                                        false
                                    };
                                    if needs_ports {
                                        this.refresh_serial_ports(window, cx);
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        Button::new("conn-telnet")
                            .small()
                            .label("Telnet")
                            .selected(form.connection == ConnectionChoice::Telnet)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.connection = ConnectionChoice::Telnet;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        Button::new("conn-mosh")
                            .small()
                            .label("Mosh")
                            .selected(form.connection == ConnectionChoice::Mosh)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.connection = ConnectionChoice::Mosh;
                                    }
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    /// 终端通用属性卡片（启动命令、语义块模式）。
    fn render_session_terminal_attributes_card(
        form: &super::types::SessionForm,
        app: &Entity<AppShell>,
        p: &Palette,
        _cx: &App,
    ) -> AnyElement {
        let block_choice = form.block_view;

        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.startup_commands").to_string()),
                    )
                    .child(Textarea::new(&form.startup_commands).h(px(88.0)).w_full())
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.startup_hint").to_string()),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.block_view").to_string()),
                    )
                    .child(
                        h_flex().gap_2().children(
                            [
                                (
                                    "block-follow-global",
                                    "form.block_view_follow",
                                    BlockViewOverride::FollowGlobal,
                                ),
                                (
                                    "block-enabled",
                                    "form.block_view_enabled",
                                    BlockViewOverride::Enabled,
                                ),
                                (
                                    "block-disabled",
                                    "form.block_view_disabled",
                                    BlockViewOverride::Disabled,
                                ),
                            ]
                            .into_iter()
                            .map(|(id, key, value)| {
                                Button::new(id)
                                    .small()
                                    .label(rust_i18n::t!(key).to_string())
                                    .selected(block_choice == value)
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        move |this, _: &ClickEvent, _, cx| {
                                            if let Some(form) = this.modals.form.as_mut() {
                                                form.block_view = value;
                                            }
                                            cx.notify();
                                        },
                                    ))
                            }),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.block_view_hint").to_string()),
                    ),
            )
            .into_any_element()
    }

    /// Mosh 协议专属设置卡片（UDP 监听端口、mosh-server 远端路径）。
    fn render_mosh_specific_card(form: &super::types::SessionForm, p: &Palette) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.mosh.group").to_string()),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(Self::form_field(
                        rust_i18n::t!("form.mosh.server_path").to_string(),
                        form.mosh_server_path.clone(),
                        p,
                    )))
                    .child(div().flex_1().child(Self::form_field(
                        rust_i18n::t!("form.mosh.server_port").to_string(),
                        form.mosh_server_port.clone(),
                        p,
                    ))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.mosh.hint").to_string()),
            )
            .into_any_element()
    }
}
