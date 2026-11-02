//! 文件页的操作输入条与路径辅助。

use super::*;

pub(crate) fn render_rename_bar(
    tab_id: TabId,
    rename_from: Option<&str>,
    rename_input: Option<&Entity<InputState>>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let from = rename_from?.to_string();
    let input = rename_input?.clone();
    Some(
        h_flex()
            .gap_2()
            .p_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.4))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.files.rename").to_string()),
            )
            .child(Input::new(&input).flex_1())
            .child(
                Button::new("container-file-rename-confirm")
                    .primary()
                    .xsmall()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.confirm").to_string())
                    .on_click(cx.listener({
                        let input = input.clone();
                        let from = from.clone();
                        move |this, _, window, cx| {
                            let value = input.read(cx).value().trim().to_string();
                            if value.is_empty() || value.contains('/') {
                                this.show_toast(
                                    rust_i18n::t!("container.files.rename_invalid").to_string(),
                                    false,
                                    window,
                                    cx,
                                );
                                return;
                            }
                            let parent = parent_path(&from);
                            let to = if parent == "/" {
                                format!("/{value}")
                            } else {
                                format!("{parent}/{value}")
                            };
                            this.rename_container_path(tab_id, from.clone(), to);
                            if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                                workspace.files.rename_input = None;
                            }
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new("container-file-rename-cancel")
                    .ghost()
                    .xsmall()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                            workspace.files.rename_from = None;
                        }
                        cx.notify();
                    })),
            )
            .into_any_element(),
    )
}

/// 「新建目录」条：在当前目录下建子目录，上传与整理都要先有目录。
pub(crate) fn render_new_dir_bar(
    tab_id: TabId,
    input: Option<&Entity<InputState>>,
    _p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(input) = input.cloned() else {
        return div().into_any_element();
    };
    h_flex()
        .gap_1()
        .items_center()
        .child(Input::new(&input).flex_1().min_w_0())
        .child(
            Button::new("container-file-mkdir-confirm")
                .primary()
                .xsmall()
                .icon(IconName::Plus)
                .label(rust_i18n::t!("container.create").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.submit_container_mkdir(tab_id, window, cx);
                })),
        )
        .child(
            Button::new("container-file-mkdir-cancel")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .label(rust_i18n::t!("common.cancel").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_container_mkdir(tab_id, window, cx);
                })),
        )
        .into_any_element()
}

/// tar 头里的权限位转成常见的 `rwxr-xr-x` 形式，只保留读写执行九位。
pub(crate) fn mode_label(mode: u32) -> String {
    const BITS: [(char, u32); 9] = [
        ('r', 0o400),
        ('w', 0o200),
        ('x', 0o100),
        ('r', 0o040),
        ('w', 0o020),
        ('x', 0o010),
        ('r', 0o004),
        ('w', 0o002),
        ('x', 0o001),
    ];
    let mut text = String::with_capacity(9);
    for (glyph, bit) in BITS {
        text.push(if mode & bit != 0 { glyph } else { '-' });
    }
    text
}

pub(crate) fn parent_path(path: &str) -> String {
    if path == "/" || path.is_empty() {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}
