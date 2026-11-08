//! 系统默认程序打开器。
//!
//! 设计原理（Why）：终端超链接、仓库主页、审计目录和容器端口链接都需要
//! 交给系统默认程序；各平台命令不同，但业务层不应重复判断 `xdg-open`、
//! `open` 与 Windows ShellExecute。这里统一委托给成熟的 `open` crate，并
//! 使用 detached 模式避免阻塞 UI 线程，同时由实现负责子进程回收。

use std::ffi::OsStr;
use std::io;

/// 用系统默认程序打开 URL、文件或目录。
///
/// 调用方必须先完成 URL 协议白名单或本地路径校验；本函数只负责交给系统。
///
/// # Errors
/// 系统没有可用的默认程序启动器或 ShellExecute 拒绝请求时返回 IO 错误。
pub fn open_with_default_app(target: impl AsRef<OsStr>) -> io::Result<()> {
    open::that_detached(target)
}
