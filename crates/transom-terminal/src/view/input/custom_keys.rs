//! custom_keys 键鼠/输入处理。

use gpui_kit::*;

/// 把设置里的映射解析成运行期查表结构；坏条目跳过并留日志。
pub(crate) fn build_custom_keys(
    bindings: &[transom_core::TerminalKeyBinding],
    os: OperatingSystem,
) -> Vec<(KeybindingKeystroke, Vec<u8>)> {
    let mut custom = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let keystroke = match Keystroke::parse_with_os(&binding.keys, os) {
            Ok(keystroke) => keystroke,
            Err(error) => {
                log::warn!("自定义按键映射无法解析，已忽略: {error}");
                continue;
            }
        };
        let bytes = match binding.decode_sequence() {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!(
                    "自定义按键映射的转义序列非法，已忽略，按键: {}，原因: {error}",
                    binding.keys
                );
                continue;
            }
        };
        custom.push((KeybindingKeystroke::from_keystroke(keystroke), bytes));
    }
    custom
}

pub(super) fn is_control_or_function_key(key: &str) -> bool {
    matches!(
        key,
        "enter"
            | "escape"
            | "backspace"
            | "tab"
            | "up"
            | "down"
            | "right"
            | "left"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "insert"
            | "delete"
            | "f1"
            | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
    )
}
