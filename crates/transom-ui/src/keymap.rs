//! 应用快捷键规格、用户覆盖解析与冲突检测。
//!
//! 默认键位使用 `$app` 占位符：macOS 展开为 `secondary`（Command），
//! 其他平台展开为 `secondary-shift`（Ctrl+Shift），避免占用终端常用的
//! Ctrl+字母控制码。用户覆盖以稳定的动作 ID 为键持久化，升级后新增动作
//! 仍能自动获得默认键位。

use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{ShortcutSettings, TerminalKeyBinding};

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
        ToggleImmersive,
        ToggleSnippets,
        SearchBuffer,
        ToggleBlockView,
        SplitRight,
        SplitDown,
        ClosePane,
        ToggleSplitAxis,
        ZoomPane,
        FocusPaneLeft,
        FocusPaneRight,
        FocusPaneUp,
        FocusPaneDown,
        FocusSftpFilter,
        ToggleSftpHidden,
        DeleteSftpSelection,
        SaveEditorFile,
        GoToEditorLine
    ]
);

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const OVERLAY_CONTEXT: &str = "Overlay";
pub const SFTP_CONTEXT: &str = "SftpPanel";
/// 内置编辑器悬浮面板的按键上下文。
pub const EDITOR_PANEL_CONTEXT: &str = "EditorPanel";
const WORKSPACE_ONLY: &str = "Workspace && !Overlay";
/// SFTP 面板内需要独占按键的场景：排除文本输入框（`Input`），
/// 保证在路径/过滤框中按 Delete 时正常编辑文本而不是触发删除文件。
const SFTP_ACTION_CONTEXT: &str = "SftpPanel && !Input";

/// 可被用户覆盖的快捷键动作描述。
pub(crate) struct ShortcutSpec {
    pub(crate) id: &'static str,
    pub(crate) label_key: &'static str,
    pub(crate) defaults: &'static [&'static str],
    bind: fn(&str) -> KeyBinding,
}

macro_rules! bind_fn {
    ($name:ident, $action:ident) => {
        fn $name(key: &str) -> KeyBinding {
            KeyBinding::new(key, $action, Some(WORKSPACE_ONLY))
        }
    };
}

bind_fn!(bind_new_session, NewSession);
bind_fn!(bind_new_local, NewLocalTerminal);
bind_fn!(bind_quick_open, QuickOpen);
bind_fn!(bind_close_tab, CloseTab);
bind_fn!(bind_next_tab, NextTab);
bind_fn!(bind_prev_tab, PrevTab);
bind_fn!(bind_reconnect, Reconnect);
bind_fn!(bind_toggle_sidebar, ToggleSidebar);
bind_fn!(bind_toggle_sftp, ToggleSftp);
bind_fn!(bind_toggle_forward, ToggleForward);
bind_fn!(bind_toggle_immersive, ToggleImmersive);
bind_fn!(bind_search_buffer, SearchBuffer);
bind_fn!(bind_toggle_block_view, ToggleBlockView);
bind_fn!(bind_toggle_snippets, ToggleSnippets);
bind_fn!(bind_split_right, SplitRight);
bind_fn!(bind_split_down, SplitDown);
bind_fn!(bind_close_pane, ClosePane);
bind_fn!(bind_toggle_split_axis, ToggleSplitAxis);
bind_fn!(bind_zoom_pane, ZoomPane);
bind_fn!(bind_focus_pane_left, FocusPaneLeft);
bind_fn!(bind_focus_pane_right, FocusPaneRight);
bind_fn!(bind_focus_pane_up, FocusPaneUp);
bind_fn!(bind_focus_pane_down, FocusPaneDown);

static SPECS: &[ShortcutSpec] = &[
    ShortcutSpec {
        id: "new_session",
        label_key: "settings.shortcut_new_session",
        defaults: &["$app-n"],
        bind: bind_new_session,
    },
    ShortcutSpec {
        id: "new_local",
        label_key: "settings.shortcut_new_local",
        defaults: &["$app-t"],
        bind: bind_new_local,
    },
    ShortcutSpec {
        id: "quick_open",
        label_key: "settings.shortcut_quick_open",
        defaults: &["$app-k"],
        bind: bind_quick_open,
    },
    ShortcutSpec {
        id: "close_tab",
        label_key: "settings.shortcut_close_tab",
        defaults: &["$app-w"],
        bind: bind_close_tab,
    },
    ShortcutSpec {
        id: "next_tab",
        label_key: "settings.shortcut_next_tab",
        defaults: &["ctrl-tab"],
        bind: bind_next_tab,
    },
    ShortcutSpec {
        id: "prev_tab",
        label_key: "settings.shortcut_prev_tab",
        defaults: &["ctrl-shift-tab"],
        bind: bind_prev_tab,
    },
    ShortcutSpec {
        id: "reconnect",
        label_key: "settings.shortcut_reconnect",
        defaults: &["$app-r"],
        bind: bind_reconnect,
    },
    ShortcutSpec {
        id: "toggle_sidebar",
        label_key: "settings.shortcut_toggle_sidebar",
        defaults: &["$app-b"],
        bind: bind_toggle_sidebar,
    },
    ShortcutSpec {
        id: "toggle_sftp",
        label_key: "settings.shortcut_toggle_sftp",
        defaults: &["$app-s"],
        bind: bind_toggle_sftp,
    },
    ShortcutSpec {
        id: "toggle_forward",
        label_key: "settings.shortcut_toggle_forward",
        defaults: &["$app-p"],
        bind: bind_toggle_forward,
    },
    ShortcutSpec {
        id: "immersive",
        label_key: "settings.shortcut_immersive",
        defaults: &["f11", "alt-enter"],
        bind: bind_toggle_immersive,
    },
    ShortcutSpec {
        id: "toggle_snippets",
        label_key: "settings.shortcut_toggle_snippets",
        defaults: &["$app-j"],
        bind: bind_toggle_snippets,
    },
    // 终端缓冲区检索：默认 `$app-f`，与 SFTP 面板的 `secondary-f` 过滤框错开，
    // 且不会吞掉 shell 里 Ctrl+F 的光标前移；用户可在快捷键设置里改成别的键。
    ShortcutSpec {
        id: "terminal_search",
        label_key: "settings.shortcut_terminal_search",
        defaults: &["$app-f"],
        bind: bind_search_buffer,
    },
    // 语义块/经典连续流热切换：默认 `$app-shift-b`（Linux 为 Ctrl+Shift+B），
    // 与终端内 Ctrl+B（tmux 前缀）错开，不会误触复用器快捷键。
    ShortcutSpec {
        id: "toggle_block_view",
        label_key: "settings.shortcut_toggle_block_view",
        defaults: &["$app-shift-b"],
        bind: bind_toggle_block_view,
    },
    ShortcutSpec {
        id: "split_right",
        label_key: "settings.shortcut_split_right",
        defaults: &["$app-d"],
        bind: bind_split_right,
    },
    ShortcutSpec {
        id: "split_down",
        label_key: "settings.shortcut_split_down",
        defaults: &["$app-e"],
        bind: bind_split_down,
    },
    ShortcutSpec {
        id: "close_pane",
        label_key: "settings.shortcut_close_pane",
        defaults: &["$app-x"],
        bind: bind_close_pane,
    },
    // 窗格导航与布局动作默认带 Alt：单纯 `$app-方向` 会吞掉 shell 里
    // Ctrl+方向 的按词跳字，`$app-alt-*` 则不与终端透传序列冲突。
    ShortcutSpec {
        id: "toggle_split_axis",
        label_key: "settings.shortcut_toggle_split_axis",
        defaults: &["$app-alt-o"],
        bind: bind_toggle_split_axis,
    },
    ShortcutSpec {
        id: "zoom_pane",
        label_key: "settings.shortcut_zoom_pane",
        defaults: &["$app-enter"],
        bind: bind_zoom_pane,
    },
    ShortcutSpec {
        id: "focus_pane_left",
        label_key: "settings.shortcut_focus_pane_left",
        defaults: &["$app-alt-left"],
        bind: bind_focus_pane_left,
    },
    ShortcutSpec {
        id: "focus_pane_right",
        label_key: "settings.shortcut_focus_pane_right",
        defaults: &["$app-alt-right"],
        bind: bind_focus_pane_right,
    },
    ShortcutSpec {
        id: "focus_pane_up",
        label_key: "settings.shortcut_focus_pane_up",
        defaults: &["$app-alt-up"],
        bind: bind_focus_pane_up,
    },
    ShortcutSpec {
        id: "focus_pane_down",
        label_key: "settings.shortcut_focus_pane_down",
        defaults: &["$app-alt-down"],
        bind: bind_focus_pane_down,
    },
];

/// 可自定义的动作列表，顺序同时决定同键冲突时的绑定优先级。
pub(crate) fn specs() -> &'static [ShortcutSpec] {
    SPECS
}

pub(crate) fn spec_by_id(id: &str) -> Option<&'static ShortcutSpec> {
    SPECS.iter().find(|spec| spec.id == id)
}

/// 生成全部应用键位：用户可覆盖的 Workspace 动作 + 固定的 SFTP 动作。
pub(crate) fn bindings(custom: &ShortcutSettings) -> Vec<KeyBinding> {
    let mut bindings = workspace_bindings(custom);
    bindings.extend([
        KeyBinding::new("secondary-f", FocusSftpFilter, Some(SFTP_CONTEXT)),
        KeyBinding::new("ctrl-h", ToggleSftpHidden, Some(SFTP_CONTEXT)),
        KeyBinding::new("secondary-h", ToggleSftpHidden, Some(SFTP_CONTEXT)),
        KeyBinding::new("delete", DeleteSftpSelection, Some(SFTP_ACTION_CONTEXT)),
        // 保存限定在编辑面板上下文内：应用整体刻意不占用 Ctrl+字母，
        // 那些控制码属于终端；只有焦点在编辑器面板里时这条键位才生效。
        // 用 secondary 而非 $app，后者在非 macOS 展开成 Ctrl+Shift，与
        // 面板外已占用的 `$app-s`（显示/隐藏 SFTP 面板）撞车。
        KeyBinding::new("secondary-s", SaveEditorFile, Some(EDITOR_PANEL_CONTEXT)),
        // 跳转到行与保存同理，仅在编辑面板内生效；secondary-g 与既有
        // secondary-f/h/s 无冲突，与主流编辑器的 Ctrl+G 习惯一致。
        KeyBinding::new("secondary-g", GoToEditorLine, Some(EDITOR_PANEL_CONTEXT)),
    ]);
    bindings
}

fn workspace_bindings(custom: &ShortcutSettings) -> Vec<KeyBinding> {
    let mut bindings = Vec::with_capacity(SPECS.len());
    for spec in SPECS {
        for key in effective_keys(spec, custom) {
            if is_valid_key(&key) {
                bindings.push((spec.bind)(&key));
            }
        }
    }
    bindings
}

/// 计算动作当前生效的键位集合；非法或空覆盖自动回退默认值。
pub(crate) fn effective_keys(spec: &ShortcutSpec, custom: &ShortcutSettings) -> Vec<String> {
    if let Some(raw_keys) = custom.bindings.get(spec.id) {
        let keys: Vec<String> = raw_keys
            .iter()
            .map(|key| expand_app_placeholder(key.trim()))
            .filter(|key| is_valid_key(key))
            .collect();
        if !keys.is_empty() {
            return keys;
        }
    }
    spec.defaults
        .iter()
        .map(|key| expand_app_placeholder(key))
        .collect()
}

/// 所有 Workspace 动作当前生效的原始键位，用于后续覆盖时抑制旧绑定。
pub(crate) fn effective_key_strings(custom: &ShortcutSettings) -> Vec<String> {
    let mut keys = Vec::new();
    for spec in SPECS {
        keys.extend(effective_keys(spec, custom));
    }
    keys
}

/// 为旧键位生成 `NoAction` 抑制绑定。
///
/// GPUI 的键位表只支持追加；在重新绑定前用 `NoAction` 覆盖旧键位，
/// 随后再追加新绑定，即可在不重建组件默认键位的前提下完成运行时改键。
pub(crate) fn suppression_bindings(keys: &[String]) -> Vec<KeyBinding> {
    keys.iter()
        .filter(|key| is_valid_key(key))
        .map(|key| KeyBinding::new(key, NoAction {}, Some(WORKSPACE_ONLY)))
        .collect()
}

/// 提取需要从终端输入中截留的按键（首击即触发的工作区快捷键）。
/// 解析并校验设置页里的自定义终端按键映射。
///
/// 校验分三层：核心层负责 `键 = 序列` 语法与转义字节；这里补上 gpui 键位
/// 语法、重复绑定与"无修饰键单字符"拦截。最后一层是刻意加的：把 `a` 绑成
/// 某个序列会直接吃掉正常输入，几乎一定是误配置。
pub(crate) fn parse_terminal_key_bindings(text: &str) -> Result<Vec<TerminalKeyBinding>, String> {
    let bindings = TerminalKeyBinding::parse_multiline(text)?;
    let mut seen: HashMap<KeybindingKeystroke, String> = HashMap::with_capacity(bindings.len());
    for binding in &bindings {
        let keystroke = Keystroke::parse(&binding.keys).map_err(|_| {
            format!(
                "按键组合「{}」无法解析，请使用 ctrl-shift-left、alt-backspace、f1 这类写法",
                binding.keys
            )
        })?;
        let parsed = KeybindingKeystroke::from_keystroke(keystroke);
        let modifiers = parsed.modifiers();
        let has_modifier = modifiers.control
            || modifiers.alt
            || modifiers.shift
            || modifiers.platform
            || modifiers.function;
        let key = parsed.key();
        if !has_modifier && (key.chars().count() == 1 || key == "space") {
            return Err(format!(
                "按键组合「{}」没有修饰键，会覆盖正常字符输入，请加 ctrl/alt/shift",
                binding.keys
            ));
        }
        if let Some(previous) = seen.insert(parsed, binding.keys.clone()) {
            return Err(format!(
                "按键组合「{}」重复绑定（前一条为「{}」）",
                binding.keys, previous
            ));
        }
    }
    Ok(bindings)
}

pub(crate) fn reserved_keystrokes(custom: &ShortcutSettings) -> Vec<KeybindingKeystroke> {
    let mut reserved = Vec::new();
    for binding in workspace_bindings(custom) {
        reserved.extend(binding.keystrokes().iter().cloned());
    }
    reserved
}

/// 将当前生效键位格式化为界面标签，例如 `Ctrl+Shift+N / F11`。
pub(crate) fn pretty_keys(spec: &ShortcutSpec, custom: &ShortcutSettings) -> String {
    effective_keys(spec, custom)
        .iter()
        .map(|key| {
            Keystroke::parse(key)
                .map(|keystroke| keystroke.to_string())
                .unwrap_or_else(|_| key.clone())
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 检查候选键位是否与其他动作冲突，返回冲突动作。
pub(crate) fn conflict_with(
    id: &str,
    candidate: &str,
    custom: &ShortcutSettings,
) -> Option<&'static ShortcutSpec> {
    let candidate = Keystroke::parse(candidate).ok()?;
    SPECS.iter().filter(|spec| spec.id != id).find(|spec| {
        effective_keys(spec, custom)
            .iter()
            .any(|key| Keystroke::parse(key).is_ok_and(|key| key == candidate))
    })
}

/// 列出当前配置中的所有键位冲突，供设置页提示。
pub(crate) fn conflict_pairs(
    custom: &ShortcutSettings,
) -> Vec<(&'static ShortcutSpec, &'static ShortcutSpec)> {
    let mut pairs = Vec::new();
    for (index, first) in SPECS.iter().enumerate() {
        for second in SPECS.iter().skip(index + 1) {
            let first_keys: Vec<Keystroke> = effective_keys(first, custom)
                .iter()
                .filter_map(|key| Keystroke::parse(key).ok())
                .collect();
            if effective_keys(second, custom)
                .iter()
                .any(|key| Keystroke::parse(key).is_ok_and(|key| first_keys.contains(&key)))
            {
                pairs.push((first, second));
            }
        }
    }
    pairs
}

/// 按键录制失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureIssue {
    /// 只按下了修饰键，缺少主键。
    ModifierOnly,
    /// 缺少 Ctrl/Alt/Cmd/Fn 等修饰键，直接绑定会吞掉正常输入。
    NeedsModifier,
}

/// 将捕获的按键转换为 GPUI 可解析的字符串。
pub(crate) fn format_captured(keystroke: &Keystroke) -> Result<String, CaptureIssue> {
    let modifier_only = matches!(
        keystroke.key.as_str(),
        "control" | "shift" | "alt" | "platform" | "function"
    );
    if modifier_only {
        return Err(CaptureIssue::ModifierOnly);
    }
    let has_modifier = keystroke.modifiers.control
        || keystroke.modifiers.alt
        || keystroke.modifiers.platform
        || keystroke.modifiers.function;
    if !has_modifier && !is_function_key(&keystroke.key) {
        return Err(CaptureIssue::NeedsModifier);
    }
    Ok(keystroke.unparse())
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .is_some_and(|number| number.parse::<u8>().is_ok())
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && key.split_whitespace().count() == 1 && Keystroke::parse(key).is_ok()
}

fn app_prefix() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "secondary"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "secondary-shift"
    }
}

fn expand_app_placeholder(key: &str) -> String {
    key.replace("$app", app_prefix())
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureIssue, SPECS, ShortcutSettings, conflict_with, effective_key_strings,
        effective_keys, expand_app_placeholder, format_captured, spec_by_id, suppression_bindings,
    };
    use gpui_kit::Keystroke;

    fn custom(id: &str, keys: &[&str]) -> ShortcutSettings {
        let mut settings = ShortcutSettings::default();
        settings.bindings.insert(
            id.to_string(),
            keys.iter().map(|key| key.to_string()).collect(),
        );
        settings
    }

    #[test]
    fn defaults_expand_platform_prefix_and_include_alternatives() {
        let settings = ShortcutSettings::default();
        let new_session = spec_by_id("new_session").unwrap();
        let keys = effective_keys(new_session, &settings);
        assert_eq!(keys.len(), 1);
        assert!(keys[0].contains("n"));
        let immersive = spec_by_id("immersive").unwrap();
        assert_eq!(
            effective_keys(immersive, &settings),
            vec!["f11", "alt-enter"]
        );
    }

    #[test]
    fn invalid_custom_keys_fall_back_to_defaults() {
        let settings = custom("new_session", &["not-a-real-key!!"]);
        let spec = spec_by_id("new_session").unwrap();
        assert_eq!(
            effective_keys(spec, &settings),
            vec![expand_app_placeholder("$app-n")]
        );
    }

    #[test]
    fn duplicate_bindings_are_detected() {
        let settings = custom("new_session", &["ctrl-shift-q"]);
        let spec = spec_by_id("new_session").unwrap();
        let keys = effective_keys(spec, &settings);
        assert_eq!(keys, vec!["ctrl-shift-q"]);
        assert_eq!(
            conflict_with("quick_open", "ctrl-shift-q", &settings).map(|spec| spec.id),
            Some("new_session")
        );
    }

    #[test]
    fn suppression_covers_every_effective_key() {
        let settings = ShortcutSettings::default();
        let keys = effective_key_strings(&settings);
        assert_eq!(keys.len(), SPECS.len() + 1);
        assert_eq!(suppression_bindings(&keys).len(), keys.len());
    }

    #[test]
    fn capture_requires_non_shift_modifier_or_function_key() {
        let plain = Keystroke::parse("n").unwrap();
        assert_eq!(format_captured(&plain), Err(CaptureIssue::NeedsModifier));
        let shift = Keystroke::parse("shift-n").unwrap();
        assert_eq!(format_captured(&shift), Err(CaptureIssue::NeedsModifier));
        let ctrl = Keystroke::parse("ctrl-shift-n").unwrap();
        assert_eq!(format_captured(&ctrl).as_deref(), Ok("ctrl-shift-n"));
        let f11 = Keystroke::parse("f11").unwrap();
        assert_eq!(format_captured(&f11).as_deref(), Ok("f11"));
    }
}

#[cfg(test)]
mod terminal_binding_tests {
    use super::parse_terminal_key_bindings;

    #[test]
    fn accepts_modifier_and_function_keys() {
        let bindings = parse_terminal_key_bindings(
            "ctrl-shift-left = \\e[1;6D\nf1 = \\eOP\nalt-backspace = \\x7f",
        )
        .unwrap();
        assert_eq!(bindings.len(), 3);
    }

    #[test]
    fn rejects_plain_typing_override_and_duplicates() {
        let error = parse_terminal_key_bindings("a = \\e").unwrap_err();
        assert!(error.contains("正常字符输入"), "错误: {error}");
        let error = parse_terminal_key_bindings("space = \\e").unwrap_err();
        assert!(error.contains("正常字符输入"), "错误: {error}");
        let error = parse_terminal_key_bindings("ctrl-a = \\e\nctrl-a = \\x01").unwrap_err();
        assert!(error.contains("重复绑定"), "错误: {error}");
    }
}
