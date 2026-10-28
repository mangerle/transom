//! 会话审计文件名的生成：清洗标签、加时间戳与标签 id，并守住防穿越底线。
//!
//! 设计原理（Why）：
//! - 文件名里的会话名与主机名都来自可编辑的配置文件（`~/.ssh/config` 导入、
//!   云端同步下来的目录都可能带 `../`、分隔符或控制字符），直接拼接等于把
//!   "往任意路径写文件"交给了配置文件，因此必须先过 `sanitize_audit_label`；
//! - 拼完还要再验一次"结果确实只是一个文件名"：清洗规则与文件系统语义之间
//!   不该靠推理保证，代码里当场证一次更便宜；
//! - 代价与局限：一个会话一个文件、按连接次数累积，不做保留策略（条数/容量
//!   上限属于另一件事），磁盘占用由 `cat` 大输出这类操作决定，用户可自行清理。

use crate::app::TerminalTab;
use chrono::Local;
use std::path::{Path, PathBuf};
use transom_core::{Connection, Session, TabId};
use transom_platform::paths;

/// 本会话的审计文件路径；未开启审计时返回 `None`。
pub(crate) fn audit_path_for(tab: &TerminalTab, enabled: bool) -> Option<PathBuf> {
    if !enabled {
        return None;
    }
    Some(build_audit_path(
        &tab.session,
        tab.id,
        &paths::audit_dir(),
        Local::now().format("%Y%m%d-%H%M%S").to_string(),
    ))
}

/// 在系统文件管理器里打开给定目录（审计落盘后用户要能一眼找到文件）。
pub(crate) fn reveal_in_file_manager(path: PathBuf) {
    open_with_default_app(path.as_os_str());
}

/// 用系统默认程序打开外部链接（"关于"页的项目主页）。
///
/// 与打开目录共用同一条 xdg-open/`open`/`explorer` 通路：三者都接受 URL，
/// 因此不必为链接再写一套进程派生与收割逻辑。调用方只传代码里的常量地址，
/// 不接受用户或远端输入，不存在被诱导执行任意参数的风险。
pub(crate) fn open_web_page(url: &str) {
    open_with_default_app(std::ffi::OsStr::new(url));
}

/// 交给系统默认程序处理一个目标，并在同一线程里收割子进程。
///
/// 子进程在同一线程里 `wait` 收割，避免遗留僵尸进程；失败只记日志，
/// 因为打不开文件管理器属于可降级的体验问题，不该冒泡成界面错误。
fn open_with_default_app(target: &std::ffi::OsStr) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = "xdg-open";

    let target = target.to_os_string();
    let spawned = std::thread::Builder::new()
        .name("transom-open-file-manager".into())
        .spawn(
            move || match std::process::Command::new(program).arg(&target).spawn() {
                Ok(mut child) => {
                    let _ = child.wait();
                }
                Err(error) => {
                    log::warn!("调用系统默认程序失败，原因: {error}");
                }
            },
        );
    if spawned.is_err() {
        log::warn!("启动系统默认程序线程失败，已忽略本次打开请求");
    }
}

/// 组装审计路径的纯函数（时间戳与根目录由调用方给出，便于单测）。
///
/// # Panics
///
/// 不 panic：清洗结果理论上总带时间戳与 id，因此永远非空。
fn build_audit_path(session: &Session, tab_id: TabId, dir: &Path, stamp: String) -> PathBuf {
    let label = paths::sanitize_audit_label(&session.name);
    let host = match &session.connection {
        Connection::Ssh(ssh) => ssh.host.clone(),
        Connection::Telnet(telnet) => telnet.host.clone(),
        Connection::Serial(serial) => serial.port.clone(),
        Connection::Local(_) => "local".to_string(),
        // 未来新增连接类型时不至于漏命名而误用上一个分支。
        _ => "session".to_string(),
    };
    let host = paths::sanitize_audit_label(&host);
    // uuid 的展示形式前 8 位就是纯十六进制，不需要再清洗；取短前缀只为让
    // 同一秒内重开的两个会话不相撞。
    let id = tab_id.to_string();
    let id = &id[..8.min(id.len())];
    let file_name = format!("{label}-{host}-{stamp}-{id}.log");
    // 当场验证清洗有效：万一规则与文件系统语义出现偏差，宁可退回中性名，
    // 也不能让一个带分隔符的字符串拼进审计目录。
    let candidate = Path::new(&file_name);
    let safe = candidate
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(&file_name));
    dir.join(if safe {
        file_name
    } else {
        format!("session-{stamp}-{id}.log")
    })
}

#[cfg(test)]
mod tests {
    use super::build_audit_path;
    use std::path::Path;
    use transom_core::{Connection, LocalConnection, Session, SessionId, TabId, ids::FolderId};

    fn ssh_session(name: &str, host: &str) -> Session {
        let mut session = Session::new(
            name,
            Connection::Ssh(transom_core::SshConnection::new(host, "deploy")),
        );
        session.folder_id = Some(FolderId::new());
        // 让测试里比较名字而不是身份，但 id 字段仍要是合法值。
        let _ = SessionId::new();
        session
    }

    fn path_of(session: &Session, dir: &str, stamp: &str) -> std::path::PathBuf {
        build_audit_path(session, TabId::new(), Path::new(dir), stamp.to_string())
    }

    /// 会话名里的路径分隔符与 `..` 必须被清洗掉，且结果始终落在审计目录内。
    #[test]
    fn traversal_in_names_is_neutralized() {
        let hostile = ssh_session("../../etc/passwd", "ok.example");
        let produced = path_of(&hostile, "/tmp/audit", "20261025-120000");
        let file_name = produced.file_name().unwrap().to_str().unwrap().to_string();
        assert!(produced.starts_with("/tmp/audit"));
        assert!(
            !file_name.contains('/') && !file_name.contains(".."),
            "清洗后仍带危险字符: {file_name}"
        );
        assert_eq!(produced.parent().unwrap(), Path::new("/tmp/audit"));
    }

    /// 中文与空格会话名很常见：清洗成下划线而不是拒绝，保证审计不因此缺失。
    #[test]
    fn non_ascii_names_still_produce_a_file() {
        let session = ssh_session("生产 数据库 主节点", "10.0.0.8");
        let produced = path_of(&session, "/tmp/audit", "20261025-120000");
        let file_name = produced.file_name().unwrap().to_str().unwrap();
        assert!(file_name.ends_with(".log"));
        assert!(file_name.contains("10.0.0.8"));
        // 主机名参与命名：多机器同名会话靠目录列表就能分清。
    }

    /// 同一秒内重开的两个会话不能写进同一个文件（会互相追加污染审计）。
    #[test]
    fn distinct_tabs_get_distinct_files() {
        let session = ssh_session("web", "h1");
        let dir = Path::new("/tmp/audit");
        let a = build_audit_path(&session, TabId::new(), dir, "20261025-120000".to_string());
        let b = build_audit_path(&session, TabId::new(), dir, "20261025-120000".to_string());
        assert_ne!(a, b);
    }

    /// 空会话名与本地会话也要有合法文件名（清洗回落到中性名）。
    #[test]
    fn empty_name_and_local_session_are_named() {
        let blank = ssh_session("", "h");
        let produced = path_of(&blank, "/tmp/audit", "20261025-120000");
        assert!(produced.file_name().unwrap().to_str().unwrap().len() > ".log".len());

        let local = Session::new("本地", Connection::Local(LocalConnection::default()));
        let produced = path_of(&local, "/tmp/audit", "20261025-120000");
        assert!(
            produced
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("local")
        );
    }
}
