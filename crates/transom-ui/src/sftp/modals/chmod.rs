//! SFTP 权限修改（Chmod）弹窗视图实体。

use super::common::*;
use gpui_kit::component::input::InputState;
use transom_core::parse_octal_permissions;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 打开权限修改弹窗所需的一次性参数包。
///
/// 设计原理（Why）：参数原本平铺了六个字段，调用点容易把路径、权限与输入实体
/// 顺序写错；参数对象让字段名在构造处可见，也为后续弹窗字段扩展保留稳定入口。
pub(crate) struct SftpChmodModalParams {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) file_name: String,
    pub(crate) remote_path: String,
    pub(crate) permissions: u32,
    pub(crate) octal_input: Entity<InputState>,
}

/// SFTP 权限修改弹窗事件。
pub(crate) enum SftpChmodModalEvent {
    Apply {
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        remote_path: String,
        permissions: u32,
    },
    Cancel,
}

/// SFTP 权限修改弹窗实体视图。
pub(crate) struct SftpChmodModalView {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) file_name: String,
    pub(crate) remote_path: String,
    pub(crate) permissions: u32,
    pub(crate) octal_input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

impl EventEmitter<SftpChmodModalEvent> for SftpChmodModalView {}

impl Render for SftpChmodModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl SftpChmodModalView {
    pub(crate) fn new(
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        file_name: String,
        remote_path: String,
        permissions: u32,
        octal_input: Entity<InputState>,
    ) -> Self {
        Self {
            tab_id,
            connection_id,
            file_name,
            remote_path,
            permissions,
            octal_input,
            error: None,
        }
    }

    pub(crate) fn toggle_bit(
        &mut self,
        mask: u32,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if checked {
            self.permissions |= mask;
        } else {
            self.permissions &= !mask;
        }
        self.sync_octal_input(window, cx);
    }

    pub(crate) fn set_preset(&mut self, mode: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.permissions = mode & 0o777;
        self.sync_octal_input(window, cx);
    }

    fn sync_octal_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        let octal_str = format_octal_permissions(self.permissions);
        self.octal_input.update(cx, |state, cx| {
            state.set_value(octal_str, window, cx);
        });
        cx.notify();
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input_text = self.octal_input.read(cx).value().trim().to_string();
        let target_perms = if input_text.is_empty() {
            self.permissions & 0o777
        } else {
            match parse_octal_permissions(&input_text) {
                Some(p) => p,
                None => {
                    self.error = Some(rust_i18n::t!("sftp.chmod.octal_invalid").to_string());
                    self.octal_input
                        .update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                    return;
                }
            }
        };
        cx.emit(SftpChmodModalEvent::Apply {
            tab_id: self.tab_id,
            connection_id: self.connection_id,
            remote_path: self.remote_path.clone(),
            permissions: target_perms,
        });
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpChmodModalEvent::Cancel);
    }

    fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let perms = self.permissions & 0o777;
        let grid = chmod_grid(perms, cx, p);

        let content = v_flex()
            .gap_3()
            .child(grid)
            .child(chmod_octal_row(perms, &self.octal_input, cx, p))
            .when_some(self.error.clone(), |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-sftp-chmod",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
            ))
            .child(
                Button::new("save-sftp-chmod")
                    .primary()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small())
                    .label(rust_i18n::t!("sftp.chmod.apply").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    })),
            );

        crate::widgets::dialog_body(
            "sftp-chmod-modal",
            rust_i18n::t!("sftp.chmod.title", name = self.file_name.as_str()).to_string(),
            Some(
                rust_i18n::t!("sftp.chmod.remote_path", path = self.remote_path.as_str())
                    .to_string(),
            ),
            content,
            Some(footer),
        )
    }
}

/// 八进制输入、等价的 rwx 文案与四个常用预设。
fn chmod_octal_row(
    perms: u32,
    octal_input: &Entity<InputState>,
    cx: &mut Context<SftpChmodModalView>,
    p: &Palette,
) -> AnyElement {
    let rwx_str = format_rwx_permissions(perms);
    let presets = [
        (0o755, "755"),
        (0o644, "644"),
        (0o700, "700"),
        (0o600, "600"),
    ];
    let mut preset_btns = h_flex().gap_1p5().items_center();
    for (mode_val, label_str) in presets {
        preset_btns = preset_btns.child(
            Button::new(format!("chmod-preset-{label_str}"))
                .ghost()
                .xsmall()
                .label(label_str)
                .selected(perms == mode_val)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_preset(mode_val, window, cx);
                })),
        );
    }

    h_flex()
        .gap_3()
        .items_center()
        .justify_between()
        .pt_1()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("sftp.chmod.octal_label").to_string()),
                )
                .child(div().w(px(90.0)).child(Input::new(octal_input).w_full()))
                .child(
                    div()
                        .text_xs()
                        .px_2()
                        .py_1()
                        .rounded(p.radius.sm)
                        .bg(p.muted)
                        .text_color(p.muted_foreground)
                        .child(rwx_str),
                ),
        )
        .child(preset_btns)
        .into_any_element()
}

/// 三组权限复选行：属主/属组/其他，各含读/写/执行。
fn chmod_grid(perms: u32, cx: &mut Context<SftpChmodModalView>, p: &Palette) -> AnyElement {
    let groups = [
        (
            rust_i18n::t!("sftp.chmod.group_owner").to_string(),
            (0o400, "chmod-ur"),
            (0o200, "chmod-uw"),
            (0o100, "chmod-ux"),
        ),
        (
            rust_i18n::t!("sftp.chmod.group_group").to_string(),
            (0o040, "chmod-gr"),
            (0o020, "chmod-gw"),
            (0o010, "chmod-gx"),
        ),
        (
            rust_i18n::t!("sftp.chmod.group_others").to_string(),
            (0o004, "chmod-or"),
            (0o002, "chmod-ow"),
            (0o001, "chmod-ox"),
        ),
    ];

    let mut grid_rows: Vec<AnyElement> = Vec::with_capacity(3);
    for (group_title, (r_mask, r_id), (w_mask, w_id), (x_mask, x_id)) in groups {
        grid_rows.push(
            h_flex()
                .py_2()
                .px_3()
                .rounded(p.radius.md)
                .border_1()
                .border_color(p.border)
                .bg(p.surface.pressed)
                .items_center()
                .justify_between()
                .child(
                    div()
                        .w(px(140.0))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(group_title),
                )
                .child(
                    h_flex()
                        .gap_4()
                        .items_center()
                        .child(
                            Checkbox::new(r_id)
                                .label(rust_i18n::t!("sftp.chmod.read").to_string())
                                .checked(perms & r_mask != 0)
                                .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                                    this.toggle_bit(r_mask, *checked, window, cx);
                                })),
                        )
                        .child(
                            Checkbox::new(w_id)
                                .label(rust_i18n::t!("sftp.chmod.write").to_string())
                                .checked(perms & w_mask != 0)
                                .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                                    this.toggle_bit(w_mask, *checked, window, cx);
                                })),
                        )
                        .child(
                            Checkbox::new(x_id)
                                .label(rust_i18n::t!("sftp.chmod.execute").to_string())
                                .checked(perms & x_mask != 0)
                                .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                                    this.toggle_bit(x_mask, *checked, window, cx);
                                })),
                        ),
                )
                .into_any_element(),
        );
    }

    v_flex().gap_2().children(grid_rows).into_any_element()
}
