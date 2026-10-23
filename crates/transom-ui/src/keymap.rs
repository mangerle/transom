use gpui_kit::*;

actions!(
    transom_ui,
    [
        NewSession,
        NewLocalTerminal,
        QuickOpen,
        CloseTab,
        NextTab,
        PrevTab,
        Reconnect,
        ToggleSidebar,
        ToggleSftp,
        ToggleForward,
        FocusSftpFilter,
        ToggleSftpHidden,
        DeleteSftpSelection
    ]
);

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const OVERLAY_CONTEXT: &str = "Overlay";
pub const SFTP_CONTEXT: &str = "SftpPanel";
const WORKSPACE_ONLY: &str = "Workspace && !Overlay";
/// SFTP 面板内需要独占按键的场景：排除文本输入框（`Input`），
/// 保证在路径/过滤框中按 Delete 时正常编辑文本而不是触发删除文件。
const SFTP_ACTION_CONTEXT: &str = "SftpPanel && !Input";

pub fn bindings() -> Vec<KeyBinding> {
    #[cfg(target_os = "macos")]
    let prefix = "secondary";
    #[cfg(not(target_os = "macos"))]
    let prefix = "secondary-shift";

    vec![
        KeyBinding::new(&format!("{prefix}-n"), NewSession, Some(WORKSPACE_ONLY)),
        KeyBinding::new(
            &format!("{prefix}-t"),
            NewLocalTerminal,
            Some(WORKSPACE_ONLY),
        ),
        KeyBinding::new(&format!("{prefix}-k"), QuickOpen, Some(WORKSPACE_ONLY)),
        KeyBinding::new(&format!("{prefix}-w"), CloseTab, Some(WORKSPACE_ONLY)),
        KeyBinding::new(&format!("{prefix}-r"), Reconnect, Some(WORKSPACE_ONLY)),
        KeyBinding::new(&format!("{prefix}-b"), ToggleSidebar, Some(WORKSPACE_ONLY)),
        KeyBinding::new(&format!("{prefix}-s"), ToggleSftp, Some(WORKSPACE_ONLY)),
        KeyBinding::new(&format!("{prefix}-p"), ToggleForward, Some(WORKSPACE_ONLY)),
        KeyBinding::new("ctrl-tab", NextTab, Some(WORKSPACE_ONLY)),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some(WORKSPACE_ONLY)),
        KeyBinding::new("secondary-f", FocusSftpFilter, Some(SFTP_CONTEXT)),
        KeyBinding::new("ctrl-h", ToggleSftpHidden, Some(SFTP_CONTEXT)),
        KeyBinding::new("secondary-h", ToggleSftpHidden, Some(SFTP_CONTEXT)),
        KeyBinding::new("delete", DeleteSftpSelection, Some(SFTP_ACTION_CONTEXT)),
    ]
}

/// Keys reserved by the application. The terminal must not encode these as
/// control characters; it lets the workspace action handle them instead.
pub fn is_app_shortcut(keystroke: &Keystroke) -> bool {
    let key = keystroke.key.to_ascii_lowercase();
    let is_action = matches!(key.as_str(), "n" | "t" | "k" | "w" | "r" | "b" | "s" | "p");

    #[cfg(target_os = "macos")]
    {
        keystroke.modifiers.platform && is_action
    }
    #[cfg(not(target_os = "macos"))]
    {
        keystroke.modifiers.control && keystroke.modifiers.shift && is_action
    }
}
