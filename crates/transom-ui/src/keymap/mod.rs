//! 应用快捷键规格、用户覆盖解析与冲突检测。
//!
//! 默认键位使用 `$app` 占位符：macOS 展开为 `secondary`（Command），
//! 其他平台展开为 `secondary-shift`（Ctrl+Shift），避免占用终端常用的
//! Ctrl+字母控制码。用户覆盖以稳定的动作 ID 为键持久化，升级后新增动作
//! 仍能自动获得默认键位。

use gpui_kit::*;
use std::collections::HashMap;
use std::rc::Rc;
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
        GoToEditorLine,
        DiffPrevChange,
        DiffNextChange,
        DiffFindInView
    ]
);

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const OVERLAY_CONTEXT: &str = "Overlay";
pub const SFTP_CONTEXT: &str = "SftpPanel";
/// 内置编辑器悬浮面板的按键上下文。
pub const EDITOR_PANEL_CONTEXT: &str = "EditorPanel";
/// Git 工作区与差异视图的按键上下文。
pub const GIT_WORKSPACE_CONTEXT: &str = "GitWorkspace";
pub const DIFF_VIEW_CONTEXT: &str = "GitDiffView";
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
    bind: fn(&str, &'static str, &App) -> Option<KeyBinding>,
    /// 动作实例工厂：命令面板据此派发真实 Action，并由组件自动关联键位提示。
    ///
    /// 为什么与 `bind` 分开：键位装载要求具体的 Action 值，而面板需要装箱的
    /// `Box<dyn Action>`；两处各生成一次，避免面板维护一份平行动作清单。
    pub(crate) action: fn() -> Box<dyn Action>,
}

mod specs;

use specs::SPECS;
#[cfg(test)]
use specs::load_key_binding;
pub(crate) use specs::{spec_by_id, specs};

/// 生成全部应用键位：用户可改键的动作（含面板专属上下文）+ 固定的 SFTP 键位
/// + 弹窗栈内多行输入框的提交键位。
pub(crate) fn bindings(custom: &ShortcutSettings, cx: &App) -> Vec<KeyBinding> {
    let mut bindings = custom_bindings(custom, cx);
    bindings.extend(crate::dialog_host::dialog_bindings());
    // 显示/隐藏隐藏项与删除键暂不进可改键表：`secondary-h` 用裸 Ctrl 而非
    // `$app`，后者在非 macOS 展开成 Ctrl+Shift，会与 `$app-s`（SFTP 面板开关）
    // 撞车；`delete` 只在非输入框的行上生效，改键收益低于误配风险。
    for (key, action, context) in [
        (
            "ctrl-h",
            Box::new(ToggleSftpHidden) as Box<dyn Action>,
            SFTP_CONTEXT,
        ),
        (
            "secondary-h",
            Box::new(ToggleSftpHidden) as Box<dyn Action>,
            SFTP_CONTEXT,
        ),
        (
            "delete",
            Box::new(DeleteSftpSelection) as Box<dyn Action>,
            SFTP_ACTION_CONTEXT,
        ),
        (
            "f7",
            Box::new(DiffNextChange) as Box<dyn Action>,
            GIT_WORKSPACE_CONTEXT,
        ),
        (
            "shift-f7",
            Box::new(DiffPrevChange) as Box<dyn Action>,
            GIT_WORKSPACE_CONTEXT,
        ),
        (
            "f7",
            Box::new(DiffNextChange) as Box<dyn Action>,
            DIFF_VIEW_CONTEXT,
        ),
        (
            "shift-f7",
            Box::new(DiffPrevChange) as Box<dyn Action>,
            DIFF_VIEW_CONTEXT,
        ),
        (
            "secondary-f",
            Box::new(DiffFindInView) as Box<dyn Action>,
            DIFF_VIEW_CONTEXT,
        ),
    ] {
        if let Some(binding) = load_boxed_key_binding(key, action, context, cx) {
            bindings.push(binding);
        }
    }
    bindings
}

fn load_boxed_key_binding(
    key: &str,
    action: Box<dyn Action>,
    context: &'static str,
    cx: &App,
) -> Option<KeyBinding> {
    let predicate = match KeyBindingContextPredicate::parse(context) {
        Ok(predicate) => predicate,
        Err(error) => {
            log::warn!("快捷键上下文无法解析，已忽略，上下文: {context}，原因: {error}");
            return None;
        }
    };
    match KeyBinding::load(key, action, Some(Rc::new(predicate)), false, None, cx) {
        Ok(binding) => Some(binding),
        Err(error) => {
            log::warn!("快捷键无法解析，已忽略，按键: {key}，上下文: {context}，原因: {error}");
            None
        }
    }
}

fn custom_bindings(custom: &ShortcutSettings, cx: &App) -> Vec<KeyBinding> {
    let mut bindings = Vec::with_capacity(SPECS.len());
    let os = cx.platform_info().os;
    for spec in SPECS {
        for key in effective_keys(spec, custom, os) {
            if let Some(binding) = (spec.bind)(&key, spec.context, cx) {
                bindings.push(binding);
            }
        }
    }
    bindings
}

/// 计算动作当前生效的键位集合；非法或空覆盖自动回退默认值。
pub(crate) fn effective_keys(
    spec: &ShortcutSpec,
    custom: &ShortcutSettings,
    os: OperatingSystem,
) -> Vec<String> {
    if let Some(raw_keys) = custom.bindings.get(spec.id) {
        let keys: Vec<String> = raw_keys
            .iter()
            .map(|key| expand_app_placeholder(key.trim()))
            .filter(|key| is_valid_key(key, os))
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
pub(crate) fn effective_key_strings(
    custom: &ShortcutSettings,
    os: OperatingSystem,
) -> Vec<(String, &'static str)> {
    let mut keys = Vec::new();
    for spec in SPECS {
        keys.extend(
            effective_keys(spec, custom, os)
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
pub(crate) fn suppression_bindings(
    entries: &[(String, &'static str)],
    cx: &App,
) -> Vec<KeyBinding> {
    entries
        .iter()
        .filter_map(|(key, context)| {
            load_boxed_key_binding(key, Box::new(NoAction {}), context, cx)
        })
        .collect()
}

/// 解析并校验设置页里的自定义终端按键映射。
///
/// 校验分三层：核心层负责 `键 = 序列` 语法与转义字节；这里补上 gpui 键位
/// 语法、重复绑定与"无修饰键单字符"拦截。最后一层是刻意加的：把 `a` 绑成
/// 某个序列会直接吃掉正常输入，几乎一定是误配置。
pub(crate) fn parse_terminal_key_bindings(
    text: &str,
    os: OperatingSystem,
) -> Result<Vec<TerminalKeyBinding>, String> {
    let bindings = TerminalKeyBinding::parse_multiline(text)?;
    let mut seen: HashMap<KeybindingKeystroke, String> = HashMap::with_capacity(bindings.len());
    for binding in &bindings {
        let keystroke = Keystroke::parse_with_os(&binding.keys, os)
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
pub(crate) fn reserved_keystrokes(
    custom: &ShortcutSettings,
    os: OperatingSystem,
) -> Vec<KeybindingKeystroke> {
    let mut reserved = Vec::new();
    for spec in SPECS {
        if !shares_focus_with_terminal(spec.context) {
            continue;
        }
        for key in effective_keys(spec, custom, os) {
            if let Ok(keystroke) = Keystroke::parse_with_os(&key, os) {
                reserved.push(KeybindingKeystroke::from_keystroke(keystroke));
            }
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
    os: OperatingSystem,
) -> Option<&'static ShortcutSpec> {
    let candidate = Keystroke::parse_with_os(candidate, os).ok()?;
    SPECS.iter().filter(|spec| spec.id != id).find(|spec| {
        effective_keys(spec, custom, os)
            .iter()
            .any(|key| Keystroke::parse_with_os(key, os).is_ok_and(|key| key == candidate))
    })
}

/// 列出当前配置中的所有键位冲突，供设置页提示。
pub(crate) fn conflict_pairs(
    custom: &ShortcutSettings,
    os: OperatingSystem,
) -> Vec<(&'static ShortcutSpec, &'static ShortcutSpec)> {
    let mut pairs = Vec::new();
    for (index, first) in SPECS.iter().enumerate() {
        for second in SPECS.iter().skip(index + 1) {
            let first_keys: Vec<Keystroke> = effective_keys(first, custom, os)
                .iter()
                .filter_map(|key| Keystroke::parse_with_os(key, os).ok())
                .collect();
            if effective_keys(second, custom, os).iter().any(|key| {
                Keystroke::parse_with_os(key, os).is_ok_and(|key| first_keys.contains(&key))
            }) {
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
pub(crate) fn format_captured(
    keystroke: &Keystroke,
    os: OperatingSystem,
) -> Result<String, CaptureIssue> {
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
    Ok(keystroke.unparse(os))
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .is_some_and(|number| number.parse::<u8>().is_ok())
}

fn is_valid_key(key: &str, os: OperatingSystem) -> bool {
    !key.is_empty()
        && key.split_whitespace().count() == 1
        && Keystroke::parse_with_os(key, os).is_ok()
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
