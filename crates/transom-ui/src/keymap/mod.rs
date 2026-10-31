//! 应用快捷键规格、用户覆盖解析与冲突检测。
//!
//! 默认键位使用 `$app` 占位符：macOS 展开为 `secondary`（Command），
//! 其他平台展开为 `secondary-shift`（Ctrl+Shift），避免占用终端常用的
//! Ctrl+字母控制码。用户覆盖以稳定的动作 ID 为键持久化，升级后新增动作
//! 仍能自动获得默认键位。

use gpui_kit::*;
use std::collections::HashMap;
use std::sync::Arc;
use transom_core::{ShortcutSettings, TerminalKeyBinding};
use transom_terminal::{TERMINAL_CONTEXT, TerminalCopy, TerminalPaste};

/// 快捷键改键捕获：等待用户按下新组合键。
pub(crate) struct ShortcutCapture {
    pub(crate) id: &'static str,
    pub(crate) error: Option<String>,
    pub(crate) focus: FocusHandle,
}

/// 快捷键域的聚合状态：生效键位过滤器、已应用键位串与改键捕获。
pub(crate) struct ShortcutRecordingState {
    pub(crate) filter: Arc<parking_lot::RwLock<Vec<KeybindingKeystroke>>>,
    /// 上一次生效的 `(键位, 上下文)`，改键时用 `NoAction` 逐条抑制旧绑定。
    pub(crate) applied_keys: Vec<(String, &'static str)>,
    pub(crate) capture: Option<ShortcutCapture>,
}

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
        ToggleCopyMode,
        ToggleBlockView,
        PrevPrompt,
        NextPrompt,
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
/// 工作区专属上下文，且要求没有遮罩浮层。
///
/// `!Overlay` 如今只对仍走自绘遮罩的浮层有效（命令面板）：它的输入框焦点在
/// `AppShell` 子树里，工作区上下文仍在按键路径上，靠这条排除才不会误触发。
/// 已迁入组件库弹窗栈的模态不需要它——容器把焦点接进窗口根的弹窗层，
/// 工作区上下文根本不在焦点路径上（实测见 `tests/dialog_key_probe.rs`
/// 的 `弹窗打开时工作区上下文的快捷键不再命中`）。
const WORKSPACE_ONLY: &str = "Workspace && !Overlay";
/// SFTP 面板内需要独占按键的场景：排除文本输入框（`Input`），
/// 保证在路径/过滤框中按 Delete 时正常编辑文本而不是触发删除文件。
const SFTP_ACTION_CONTEXT: &str = "SftpPanel && !Input";

/// 可被用户覆盖的快捷键动作描述。
pub(crate) struct ShortcutSpec {
    pub(crate) id: &'static str,
    pub(crate) label_key: &'static str,
    pub(crate) defaults: &'static [&'static str],
    /// 该动作生效的按键上下文。
    ///
    /// 为什么必须逐条声明而不是统一挂在工作区上下文上：上下文的宽窄直接决定
    /// 两件事。一是改键时旧键位的 `NoAction` 抑制必须注册在同一个上下文里，
    /// 否则旧绑定不会被覆盖；二是终端的保留键名单只能收录"焦点在终端时也可能
    /// 命中"的上下文（工作区与终端），把 SFTP/编辑器面板的专属键也收进去，
    /// 会让终端白丢 Ctrl+F/Ctrl+S 这些 Shell 与 vim 赖以工作的控制码。
    pub(crate) context: &'static str,
    bind: fn(&str, &'static str) -> KeyBinding,
    /// 动作实例工厂：命令面板据此派发真实 Action，并由组件自动关联键位提示。
    ///
    /// 为什么与 `bind` 分开：`KeyBinding::new` 要求具体的 Action 值，而面板需要
    /// 装箱的 `Box<dyn Action>`；两处各生成一次，避免面板维护一份平行动作清单。
    pub(crate) action: fn() -> Box<dyn Action>,
}

macro_rules! bind_fn {
    ($bind:ident, $make:ident, $action:ident) => {
        fn $bind(key: &str, context: &'static str) -> KeyBinding {
            KeyBinding::new(key, $action, Some(context))
        }
        fn $make() -> Box<dyn Action> {
            Box::new($action)
        }
    };
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

static SPECS: &[ShortcutSpec] = &[
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
];

/// 可自定义的动作列表，顺序同时决定同键冲突时的绑定优先级。
pub(crate) fn specs() -> &'static [ShortcutSpec] {
    SPECS
}

pub(crate) fn spec_by_id(id: &str) -> Option<&'static ShortcutSpec> {
    SPECS.iter().find(|spec| spec.id == id)
}

/// 生成全部应用键位：用户可改键的动作（含面板专属上下文）+ 固定的 SFTP 键位
/// + 弹窗栈内多行输入框的提交键位。
pub(crate) fn bindings(custom: &ShortcutSettings) -> Vec<KeyBinding> {
    let mut bindings = custom_bindings(custom);
    bindings.extend(crate::dialog_host::dialog_bindings());
    bindings.extend([
        // 显示/隐藏隐藏项与删除键暂不进可改键表：`secondary-h` 用裸 Ctrl 而非
        // `$app`，后者在非 macOS 展开成 Ctrl+Shift，会与 `$app-s`（SFTP 面板开关）
        // 撞车；`delete` 只在非输入框的行上生效，改键收益低于误配风险。
        KeyBinding::new("ctrl-h", ToggleSftpHidden, Some(SFTP_CONTEXT)),
        KeyBinding::new("secondary-h", ToggleSftpHidden, Some(SFTP_CONTEXT)),
        KeyBinding::new("delete", DeleteSftpSelection, Some(SFTP_ACTION_CONTEXT)),
    ]);
    bindings
}

fn custom_bindings(custom: &ShortcutSettings) -> Vec<KeyBinding> {
    let mut bindings = Vec::with_capacity(SPECS.len());
    for spec in SPECS {
        for key in effective_keys(spec, custom) {
            if is_valid_key(&key) {
                bindings.push((spec.bind)(&key, spec.context));
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

/// 所有可改键动作当前生效的 `(键位, 上下文)`，供改键时抑制旧绑定。
///
/// 上下文必须随键位一起带出：抑制绑定只有注册在同一个上下文里才盖得住旧绑定，
/// 否则旧键位会继续触发改键前注册的动作。
pub(crate) fn effective_key_strings(custom: &ShortcutSettings) -> Vec<(String, &'static str)> {
    let mut keys = Vec::new();
    for spec in SPECS {
        keys.extend(
            effective_keys(spec, custom)
                .into_iter()
                .map(|key| (key, spec.context)),
        );
    }
    keys
}

/// 为旧键位生成 `NoAction` 抑制绑定。
///
/// GPUI 的键位表只支持追加；在重新绑定前用 `NoAction` 覆盖旧键位，
/// 随后再追加新绑定，即可在不重建组件默认键位的前提下完成运行时改键。
pub(crate) fn suppression_bindings(entries: &[(String, &'static str)]) -> Vec<KeyBinding> {
    entries
        .iter()
        .filter(|(key, _)| is_valid_key(key))
        .map(|(key, context)| KeyBinding::new(key, NoAction {}, Some(*context)))
        .collect()
}

/// 解析并校验设置页里的自定义终端按键映射。
///
/// 校验分三层：核心层负责 `键 = 序列` 语法与转义字节；这里补上 gpui 键位
/// 语法、重复绑定与"无修饰键单字符"拦截。最后一层是刻意加的：把 `a` 绑成
/// 某个序列会直接吃掉正常输入，几乎一定是误配置。
pub(crate) fn parse_terminal_key_bindings(text: &str) -> Result<Vec<TerminalKeyBinding>, String> {
    let bindings = TerminalKeyBinding::parse_multiline(text)?;
    let mut seen: HashMap<KeybindingKeystroke, String> = HashMap::with_capacity(bindings.len());
    for binding in &bindings {
        let keystroke = Keystroke::parse(&binding.keys)
            .map_err(|_| rust_i18n::t!("keymap.parse_failed", keys = binding.keys).to_string())?;
        let parsed = KeybindingKeystroke::from_keystroke(keystroke);
        let modifiers = parsed.modifiers();
        let has_modifier = modifiers.control
            || modifiers.alt
            || modifiers.shift
            || modifiers.platform
            || modifiers.function;
        let key = parsed.key();
        if !has_modifier && (key.chars().count() == 1 || key == "space") {
            return Err(rust_i18n::t!("keymap.missing_modifier", keys = binding.keys).to_string());
        }
        if let Some(previous) = seen.insert(parsed, binding.keys.clone()) {
            return Err(rust_i18n::t!(
                "keymap.duplicate",
                keys = binding.keys,
                previous = previous
            )
            .to_string());
        }
    }
    Ok(bindings)
}

/// 提取需要从终端输入中截留的按键：只收录在终端聚焦时也可能命中的上下文。
///
/// 设计原理（Why）：这份名单交给终端视图判断"按键归属"——命中即表示应用会消费
/// 这次按键，视图必须让路，否则一次按键会既执行应用动作、又被编码成控制码发给
/// Shell（Ctrl+Shift+C 会编成 0x03 打断前台进程）。
/// 反之，SFTP 面板与编辑面板的专属键在终端聚焦时根本匹配不到，绝不能收进来：
/// 收了就是白丢 Ctrl+F、Ctrl+S、Ctrl+G 这些 Shell 与 vim 赖以工作的控制码。
/// 代价与局限：判断依据是上下文字符串相等，新增上下文时要同步这里。
pub(crate) fn reserved_keystrokes(custom: &ShortcutSettings) -> Vec<KeybindingKeystroke> {
    let mut reserved = Vec::new();
    for spec in SPECS {
        if !shares_focus_with_terminal(spec.context) {
            continue;
        }
        for key in effective_keys(spec, custom) {
            if !is_valid_key(&key) {
                continue;
            }
            let binding = (spec.bind)(&key, spec.context);
            reserved.extend(binding.keystrokes().iter().cloned());
        }
    }
    reserved
}

/// 该上下文的按键是否会与终端抢同一次焦点：工作区与终端自身会，面板专属上下文不会。
fn shares_focus_with_terminal(context: &'static str) -> bool {
    context == WORKSPACE_ONLY || context == TERMINAL_CONTEXT
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
mod tests;
