//! 可自定义快捷键动作规格表。

use super::*;

macro_rules! bind_fn {
    ($bind:ident, $make:ident, $action:ident) => {
        fn $bind(key: &str, context: &'static str, cx: &App) -> Option<KeyBinding> {
            load_key_binding(key, $action, context, cx)
        }
        fn $make() -> Box<dyn Action> {
            Box::new($action)
        }
    };
}

/// 从运行期字符串装载一条键位。
///
/// 为什么不用 `KeyBinding::new`：0.3.12 起该构造器只接受 `&'static str`，
/// 且不会解析 `secondary-` 别名。用户改键与平台展开都发生在运行期，必须走
/// `KeyBinding::load`；解析失败时记录中文告警并跳过该条，避免一条坏配置
/// 让整个快捷键系统失效。
pub(super) fn load_key_binding<A: Action>(
    key: &str,
    action: A,
    context: &'static str,
    cx: &App,
) -> Option<KeyBinding> {
    load_boxed_key_binding(key, Box::new(action), context, cx)
}

bind_fn!(bind_new_session, action_new_session, NewSession);
bind_fn!(bind_new_local, action_new_local, NewLocalTerminal);
bind_fn!(bind_quick_open, action_quick_open, QuickOpen);
bind_fn!(bind_close_tab, action_close_tab, CloseTab);
bind_fn!(bind_next_tab, action_next_tab, NextTab);
bind_fn!(bind_prev_tab, action_prev_tab, PrevTab);
bind_fn!(bind_reconnect, action_reconnect, Reconnect);
bind_fn!(bind_toggle_sidebar, action_toggle_sidebar, ToggleSidebar);
bind_fn!(bind_toggle_sftp, action_toggle_sftp, ToggleSftp);
bind_fn!(bind_toggle_forward, action_toggle_forward, ToggleForward);
bind_fn!(
    bind_toggle_immersive,
    action_toggle_immersive,
    ToggleImmersive
);
bind_fn!(bind_search_buffer, action_search_buffer, SearchBuffer);
bind_fn!(
    bind_toggle_copy_mode,
    action_toggle_copy_mode,
    ToggleCopyMode
);
bind_fn!(bind_terminal_copy, action_terminal_copy, TerminalCopy);
bind_fn!(bind_terminal_paste, action_terminal_paste, TerminalPaste);
bind_fn!(
    bind_toggle_block_view,
    action_toggle_block_view,
    ToggleBlockView
);
bind_fn!(bind_prev_prompt, action_prev_prompt, PrevPrompt);
bind_fn!(bind_next_prompt, action_next_prompt, NextPrompt);
bind_fn!(bind_toggle_snippets, action_toggle_snippets, ToggleSnippets);
bind_fn!(
    bind_inline_ai_assist,
    action_inline_ai_assist,
    InlineAiAssist
);
bind_fn!(bind_split_right, action_split_right, SplitRight);
bind_fn!(bind_split_down, action_split_down, SplitDown);
bind_fn!(bind_close_pane, action_close_pane, ClosePane);
bind_fn!(
    bind_toggle_split_axis,
    action_toggle_split_axis,
    ToggleSplitAxis
);
bind_fn!(bind_zoom_pane, action_zoom_pane, ZoomPane);
bind_fn!(bind_focus_pane_left, action_focus_pane_left, FocusPaneLeft);
bind_fn!(
    bind_focus_pane_right,
    action_focus_pane_right,
    FocusPaneRight
);
bind_fn!(bind_focus_pane_up, action_focus_pane_up, FocusPaneUp);
bind_fn!(bind_focus_pane_down, action_focus_pane_down, FocusPaneDown);
bind_fn!(
    bind_focus_sftp_filter,
    action_focus_sftp_filter,
    FocusSftpFilter
);
bind_fn!(
    bind_save_editor_file,
    action_save_editor_file,
    SaveEditorFile
);
bind_fn!(
    bind_go_to_editor_line,
    action_go_to_editor_line,
    GoToEditorLine
);

pub(super) static SPECS: &[ShortcutSpec] = &[
    ShortcutSpec {
        id: "new_session",
        label_key: "settings.shortcut_new_session",
        defaults: &["$app-n"],
        context: WORKSPACE_ONLY,
        bind: bind_new_session,
        action: action_new_session,
    },
    ShortcutSpec {
        id: "new_local",
        label_key: "settings.shortcut_new_local",
        defaults: &["$app-t"],
        context: WORKSPACE_ONLY,
        bind: bind_new_local,
        action: action_new_local,
    },
    ShortcutSpec {
        id: "quick_open",
        label_key: "settings.shortcut_quick_open",
        defaults: &["$app-k"],
        context: WORKSPACE_ONLY,
        bind: bind_quick_open,
        action: action_quick_open,
    },
    ShortcutSpec {
        id: "close_tab",
        label_key: "settings.shortcut_close_tab",
        defaults: &["$app-w"],
        context: WORKSPACE_ONLY,
        bind: bind_close_tab,
        action: action_close_tab,
    },
    ShortcutSpec {
        id: "next_tab",
        label_key: "settings.shortcut_next_tab",
        defaults: &["ctrl-tab"],
        context: WORKSPACE_ONLY,
        bind: bind_next_tab,
        action: action_next_tab,
    },
    ShortcutSpec {
        id: "prev_tab",
        label_key: "settings.shortcut_prev_tab",
        defaults: &["ctrl-shift-tab"],
        context: WORKSPACE_ONLY,
        bind: bind_prev_tab,
        action: action_prev_tab,
    },
    ShortcutSpec {
        id: "reconnect",
        label_key: "settings.shortcut_reconnect",
        defaults: &["$app-r"],
        context: WORKSPACE_ONLY,
        bind: bind_reconnect,
        action: action_reconnect,
    },
    ShortcutSpec {
        id: "toggle_sidebar",
        label_key: "settings.shortcut_toggle_sidebar",
        defaults: &["$app-b"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_sidebar,
        action: action_toggle_sidebar,
    },
    ShortcutSpec {
        id: "toggle_sftp",
        label_key: "settings.shortcut_toggle_sftp",
        defaults: &["$app-s"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_sftp,
        action: action_toggle_sftp,
    },
    ShortcutSpec {
        id: "toggle_forward",
        label_key: "settings.shortcut_toggle_forward",
        defaults: &["$app-p"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_forward,
        action: action_toggle_forward,
    },
    ShortcutSpec {
        id: "immersive",
        label_key: "settings.shortcut_immersive",
        defaults: &["f11", "alt-enter"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_immersive,
        action: action_toggle_immersive,
    },
    ShortcutSpec {
        id: "toggle_snippets",
        label_key: "settings.shortcut_toggle_snippets",
        defaults: &["$app-j"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_snippets,
        action: action_toggle_snippets,
    },
    // 终端缓冲区检索：默认 `$app-f`，与 SFTP 面板的 `secondary-f` 过滤框错开，
    // 且不会吞掉 shell 里 Ctrl+F 的光标前移；用户可在快捷键设置里改成别的键。
    ShortcutSpec {
        id: "terminal_search",
        label_key: "settings.shortcut_terminal_search",
        defaults: &["$app-f"],
        context: WORKSPACE_ONLY,
        bind: bind_search_buffer,
        action: action_search_buffer,
    },
    // 键盘复制模式：默认 Ctrl+Shift+Space。选它是因为主流终端（kitty、
    // Windows Terminal）都用这个组合进入选择模式，且不属于终端控制码；
    // 用户仍可在快捷键设置里改键。
    ShortcutSpec {
        id: "copy_mode",
        label_key: "settings.shortcut_copy_mode",
        defaults: &["ctrl-shift-space"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_copy_mode,
        action: action_toggle_copy_mode,
    },
    // 终端复制/粘贴：默认 `$app-c`、`$app-v`（macOS 为 Cmd+…，其他平台展开成
    // Ctrl+Shift+…），与既有的终端内硬编码组合键完全一致，只是从写死变成可改键。
    // 上下文取终端自己的 `Terminal` 而不是工作区：动作由焦点窗格的视图就地处理，
    // 也避免把 Cmd+C 这种全局组合注册到文本输入框上、抢掉输入框自己的复制。
    ShortcutSpec {
        id: "terminal_copy",
        label_key: "settings.shortcut_terminal_copy",
        defaults: &["$app-c"],
        context: TERMINAL_CONTEXT,
        bind: bind_terminal_copy,
        action: action_terminal_copy,
    },
    ShortcutSpec {
        id: "terminal_paste",
        label_key: "settings.shortcut_terminal_paste",
        defaults: &["$app-v"],
        context: TERMINAL_CONTEXT,
        bind: bind_terminal_paste,
        action: action_terminal_paste,
    },
    // 语义块/经典连续流热切换：默认 `$app-shift-b`（Linux 为 Ctrl+Shift+B），
    // 与终端内 Ctrl+B（tmux 前缀）错开，不会误触复用器快捷键。
    ShortcutSpec {
        id: "toggle_block_view",
        label_key: "settings.shortcut_toggle_block_view",
        defaults: &["$app-shift-b"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_block_view,
        action: action_toggle_block_view,
    },
    // 提示符跳跃默认落在 `$app-方向`：macOS 是 Cmd+方向，其他平台展开成
    // Ctrl+Shift+方向。刻意不占裸 Ctrl+方向——那在 bash/zsh 的 readline 里是
    // 历史搜索与翻页键，吞掉它等于废掉 Shell 自己的功能，而这个动作随时可改键。
    ShortcutSpec {
        id: "prev_prompt",
        label_key: "settings.shortcut_prev_prompt",
        defaults: &["$app-up"],
        context: WORKSPACE_ONLY,
        bind: bind_prev_prompt,
        action: action_prev_prompt,
    },
    ShortcutSpec {
        id: "next_prompt",
        label_key: "settings.shortcut_next_prompt",
        defaults: &["$app-down"],
        context: WORKSPACE_ONLY,
        bind: bind_next_prompt,
        action: action_next_prompt,
    },
    ShortcutSpec {
        id: "split_right",
        label_key: "settings.shortcut_split_right",
        defaults: &["$app-d"],
        context: WORKSPACE_ONLY,
        bind: bind_split_right,
        action: action_split_right,
    },
    ShortcutSpec {
        id: "split_down",
        label_key: "settings.shortcut_split_down",
        defaults: &["$app-e"],
        context: WORKSPACE_ONLY,
        bind: bind_split_down,
        action: action_split_down,
    },
    ShortcutSpec {
        id: "close_pane",
        label_key: "settings.shortcut_close_pane",
        defaults: &["$app-x"],
        context: WORKSPACE_ONLY,
        bind: bind_close_pane,
        action: action_close_pane,
    },
    // 窗格导航与布局动作默认带 Alt：单纯 `$app-方向` 会吞掉 shell 里
    // Ctrl+方向 的按词跳字，`$app-alt-*` 则不与终端透传序列冲突。
    ShortcutSpec {
        id: "toggle_split_axis",
        label_key: "settings.shortcut_toggle_split_axis",
        defaults: &["$app-alt-o"],
        context: WORKSPACE_ONLY,
        bind: bind_toggle_split_axis,
        action: action_toggle_split_axis,
    },
    ShortcutSpec {
        id: "zoom_pane",
        label_key: "settings.shortcut_zoom_pane",
        defaults: &["$app-enter"],
        context: WORKSPACE_ONLY,
        bind: bind_zoom_pane,
        action: action_zoom_pane,
    },
    ShortcutSpec {
        id: "focus_pane_left",
        label_key: "settings.shortcut_focus_pane_left",
        defaults: &["$app-alt-left"],
        context: WORKSPACE_ONLY,
        bind: bind_focus_pane_left,
        action: action_focus_pane_left,
    },
    ShortcutSpec {
        id: "focus_pane_right",
        label_key: "settings.shortcut_focus_pane_right",
        defaults: &["$app-alt-right"],
        context: WORKSPACE_ONLY,
        bind: bind_focus_pane_right,
        action: action_focus_pane_right,
    },
    ShortcutSpec {
        id: "focus_pane_up",
        label_key: "settings.shortcut_focus_pane_up",
        defaults: &["$app-alt-up"],
        context: WORKSPACE_ONLY,
        bind: bind_focus_pane_up,
        action: action_focus_pane_up,
    },
    ShortcutSpec {
        id: "focus_pane_down",
        label_key: "settings.shortcut_focus_pane_down",
        defaults: &["$app-alt-down"],
        context: WORKSPACE_ONLY,
        bind: bind_focus_pane_down,
        action: action_focus_pane_down,
    },
    // SFTP 面板的过滤框聚焦：`secondary-f` 用裸 Ctrl 而非 `$app`，后者在非 macOS
    // 展开成 Ctrl+Shift，会与 `$app-s`（显示/隐藏 SFTP 面板）撞车；它只在面板
    // 上下文里生效，因此不占用终端的 Ctrl+F。
    ShortcutSpec {
        id: "sftp_filter",
        label_key: "settings.shortcut_sftp_filter",
        defaults: &["secondary-f"],
        context: SFTP_CONTEXT,
        bind: bind_focus_sftp_filter,
        action: action_focus_sftp_filter,
    },
    // 保存与跳转到行只在编辑面板上下文内生效：应用整体刻意不占用 Ctrl+字母，
    // 那些控制码属于终端；只有焦点在编辑器面板里时这两条键位才生效。
    ShortcutSpec {
        id: "save_editor_file",
        label_key: "settings.shortcut_save_editor",
        defaults: &["secondary-s"],
        context: EDITOR_PANEL_CONTEXT,
        bind: bind_save_editor_file,
        action: action_save_editor_file,
    },
    // `secondary-g` 与面板内既有 secondary-f/h/s 无冲突，与主流编辑器的 Ctrl+G 一致。
    ShortcutSpec {
        id: "go_to_editor_line",
        label_key: "settings.shortcut_go_to_line",
        defaults: &["secondary-g"],
        context: EDITOR_PANEL_CONTEXT,
        bind: bind_go_to_editor_line,
        action: action_go_to_editor_line,
    },
    ShortcutSpec {
        id: "inline_ai_assist",
        label_key: "settings.shortcut_inline_ai_assist",
        defaults: &["$app-i"],
        context: WORKSPACE_ONLY,
        bind: bind_inline_ai_assist,
        action: action_inline_ai_assist,
    },
];

/// 可自定义的动作列表，顺序同时决定同键冲突时的绑定优先级。
pub(crate) fn specs() -> &'static [ShortcutSpec] {
    SPECS
}

pub(crate) fn spec_by_id(id: &str) -> Option<&'static ShortcutSpec> {
    SPECS.iter().find(|spec| spec.id == id)
}
