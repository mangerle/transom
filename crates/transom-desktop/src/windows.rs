//! Windows 桌面注册：`ssh://` 默认应用候选。
//!
//! 设计原理（Why）：Windows 11 的系统默认终端由 Console Handoff COM 协议
//! 管理，只写注册表模拟会破坏控制台启动链；因此这里不伪造默认终端能力。
//! 可以安全实现的是每用户 URL Scheme 候选登记：写入专用 ProgID、Capabilities
//! 和 RegisteredApplications，用户可在“默认应用”中自行选择 Transom。

use std::path::Path;
use transom_core::{AppError, ErrorCode};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

const PROG_ID: &str = "Transom.ssh";
const APP_REGISTRATION_NAME: &str = "Transom";
const APP_REGISTRATION_PATH: &str = r"Software\Transom\Capabilities";
const PROG_ID_PATH: &str = r"Software\Classes\Transom.ssh";
const REGISTERED_APPLICATIONS: &str = r"Software\RegisteredApplications";

/// Windows 默认终端需要完整 Console Handoff COM 实现，当前明确拒绝伪注册。
pub fn apply_default_terminal(_enabled: bool) -> Result<(), AppError> {
    Err(unsupported_default_terminal())
}

/// 注册系统默认终端；Windows 上返回明确的不支持错误。
pub fn register_default_terminal(_exe: &Path) -> Result<(), AppError> {
    Err(unsupported_default_terminal())
}

/// 撤销系统默认终端；Windows 上返回明确的不支持错误。
pub fn unregister_default_terminal() -> Result<(), AppError> {
    Err(unsupported_default_terminal())
}

/// Windows 没有可由本应用安全查询的通用默认终端状态。
pub fn default_terminal_registered(_exe: &Path) -> bool {
    false
}

/// 注册当前可执行文件为 `ssh://` 默认应用候选，不修改用户当前默认选择。
pub fn ensure_uri_handler(exe: &Path) -> Result<(), AppError> {
    let expected_command = handler_command(exe);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let current = hkcu
        .open_subkey_with_flags(r"Software\Classes\Transom.ssh\shell\open\command", KEY_READ)
        .ok()
        .and_then(|key| key.get_value::<String, _>("").ok());
    if current.as_deref() == Some(expected_command.as_str()) {
        return Ok(());
    }
    register_uri_handler(exe)
}

fn register_uri_handler(exe: &Path) -> Result<(), AppError> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (prog_id, _) = hkcu
        .create_subkey(PROG_ID_PATH)
        .map_err(|error| registry_error(PROG_ID_PATH, error))?;
    prog_id
        .set_value("", &"URL:Transom SSH")
        .map_err(|error| registry_error(PROG_ID_PATH, error))?;
    prog_id
        .set_value("URL Protocol", &"")
        .map_err(|error| registry_error(PROG_ID_PATH, error))?;
    let (icon, _) = hkcu
        .create_subkey(format!(r"{PROG_ID_PATH}\DefaultIcon"))
        .map_err(|error| registry_error(PROG_ID_PATH, error))?;
    icon.set_value("", &format!("\"{}\",0", exe.display()))
        .map_err(|error| registry_error(PROG_ID_PATH, error))?;
    let command_path = format!(r"{PROG_ID_PATH}\shell\open\command");
    let (command, _) = hkcu
        .create_subkey(&command_path)
        .map_err(|error| registry_error(&command_path, error))?;
    command
        .set_value("", &handler_command(exe))
        .map_err(|error| registry_error(&command_path, error))?;

    let (capabilities, _) = hkcu
        .create_subkey(APP_REGISTRATION_PATH)
        .map_err(|error| registry_error(APP_REGISTRATION_PATH, error))?;
    capabilities
        .set_value("ApplicationName", &"Transom")
        .map_err(|error| registry_error(APP_REGISTRATION_PATH, error))?;
    capabilities
        .set_value("ApplicationDescription", &"Transom SSH 终端工作台")
        .map_err(|error| registry_error(APP_REGISTRATION_PATH, error))?;
    capabilities
        .set_value("ApplicationIcon", &format!("\"{}\",0", exe.display()))
        .map_err(|error| registry_error(APP_REGISTRATION_PATH, error))?;
    let associations_path = format!(r"{APP_REGISTRATION_PATH}\URLAssociations");
    let (associations, _) = hkcu
        .create_subkey(&associations_path)
        .map_err(|error| registry_error(&associations_path, error))?;
    associations
        .set_value("ssh", &PROG_ID)
        .map_err(|error| registry_error(&associations_path, error))?;
    let (registered, _) = hkcu
        .create_subkey(REGISTERED_APPLICATIONS)
        .map_err(|error| registry_error(REGISTERED_APPLICATIONS, error))?;
    registered
        .set_value(APP_REGISTRATION_NAME, &APP_REGISTRATION_PATH)
        .map_err(|error| registry_error(REGISTERED_APPLICATIONS, error))
}

fn handler_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

fn unsupported_default_terminal() -> AppError {
    AppError::with_detail(
        ErrorCode::Configuration,
        "Windows 默认终端需要完整 Console Handoff COM 支持；Transom 不伪造注册，已提供 ssh:// 默认应用候选",
    )
}

fn registry_error(path: &str, error: std::io::Error) -> AppError {
    AppError::with_detail(
        ErrorCode::Io,
        format!("写入 Windows 桌面注册失败，注册表路径: {path}，原因: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::handler_command;
    use std::path::Path;

    #[test]
    fn handler_command_preserves_executable_path_argument() {
        assert_eq!(
            handler_command(Path::new(r"C:\Program Files\Transom\transom.exe")),
            r#""C:\Program Files\Transom\transom.exe" "%1""#
        );
    }
}
