//! 当前用户私有的 Windows 命名管道创建与连接。
//!
//! 设计原理（Why）：单实例深链需要跨进程传输 URL，Windows 命名管道是首选的
//! 本机 IPC；但默认管道安全描述符可能允许同一机器上的其它用户连接。这里复用
//! 私有文件 DACL 的安全描述符，把服务端管道限定为当前用户、SYSTEM 与管理员，
//! 并启用 `PIPE_REJECT_REMOTE_CLIENTS`，同时关闭远程客户端面。
//!
//! 代价与局限：服务端句柄必须以 overlapped 模式创建，调用方需转换为异步
//! 命名管道对象；本模块只负责安全创建与文件句柄封装，不绑定具体异步运行时。

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::time::Duration;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
};
use windows::core::PCWSTR;

use crate::private_fs::windows::PrivateSecurity;

/// 单个客户端连接允许等待服务端的最长时间。
const PIPE_TIMEOUT_MS: u32 = 2_000;
/// 管道读写缓冲区：深链请求上限 16 KiB，留出一倍余量。
const PIPE_BUFFER_BYTES: u32 = 32 * 1024;
/// 当前连接与其后继监听实例同时存在，避免两次请求之间出现 NotFound 窗口。
const PIPE_MAX_INSTANCES: u32 = 2;
/// 提权代理的可执行文件启动与连接上限；用户可在 UAC 提示上停留更久。
pub const BROKER_PIPE_ACCEPT_TIMEOUT: Duration = Duration::from_secs(120);

/// 创建私有命名管道服务端。
///
/// `first_instance` 为真时启用 `FILE_FLAG_FIRST_PIPE_INSTANCE`，用于原子地
/// 判定本进程是否为首个实例；后续轮次必须置为假。
pub fn create_private_server(name: &str, first_instance: bool) -> io::Result<File> {
    let security = PrivateSecurity::new(false)?;
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first_instance {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let name = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open_mode,
            mode,
            PIPE_MAX_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            PIPE_TIMEOUT_MS,
            Some(security.attributes_ptr()),
        )
    };
    if handle.is_invalid() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

/// 连接已存在的私有命名管道客户端。
///
/// 客户端使用同步文件句柄：调用方只做一次有界请求写入，随后立即关闭。
/// 服务端是否在线由连接错误反馈，调用方据此区分“已有实例”和“陈旧端点”。
pub fn connect_private_client(name: &str) -> io::Result<File> {
    let name = wide(name);
    let mut last_error = None;
    for attempt in 0..2 {
        match open_client(&name) {
            Ok(file) => return Ok(file),
            Err(error) if attempt == 0 => {
                last_error = Some(error);
                if !unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), PIPE_TIMEOUT_MS) }.as_bool() {
                    last_error = Some(io::Error::last_os_error());
                }
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("连接命名管道失败")))
}

/// 创建可阻塞读写的私有命名管道服务端。
///
/// 与深链使用的 overlapped 服务端不同，提权 PTY 代理需要把句柄直接包装成
/// `std::fs::File` 供终端阻塞读写线程使用，因此这里不能设置
/// `FILE_FLAG_OVERLAPPED`。
pub fn create_private_blocking_server(name: &str) -> io::Result<File> {
    let security = PrivateSecurity::new(false)?;
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let name = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            mode,
            1,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            PIPE_TIMEOUT_MS,
            Some(security.attributes_ptr()),
        )
    };
    if handle.is_invalid() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

/// 等待客户端连接阻塞命名管道，并在超时后取消等待。
pub fn accept_private_server(server: &File, timeout: Duration) -> io::Result<()> {
    let raw_handle = server.as_raw_handle() as isize;
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            if done_rx.recv_timeout(timeout).is_err() {
                // 安全性：只对调用方传入的有效命名管道句柄取消自身发起的 I/O。
                unsafe {
                    let _ = CancelIoEx(
                        windows::Win32::Foundation::HANDLE(raw_handle as *mut _),
                        None,
                    );
                }
            }
        });
        let result = unsafe { ConnectNamedPipe(server_handle(server), None) };
        let _ = done_tx.send(());
        if result.is_ok() {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(windows::Win32::Foundation::ERROR_PIPE_CONNECTED.0 as i32) {
            return Ok(());
        }
        if error.raw_os_error()
            == Some(windows::Win32::Foundation::ERROR_OPERATION_ABORTED.0 as i32)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "等待提权 PTY 代理连接超时",
            ));
        }
        Err(error)
    })
}

/// 读取已连接命名管道客户端的进程 id，用于校验提权代理身份。
pub fn private_server_client_process_id(server: &File) -> io::Result<u32> {
    let mut process_id = 0;
    // 安全性：句柄来自已连接的命名管道，输出指针指向栈上有效整数。
    unsafe { GetNamedPipeClientProcessId(server_handle(server), &mut process_id) }
        .map_err(io::Error::other)?;
    Ok(process_id)
}

fn server_handle(server: &File) -> windows::Win32::Foundation::HANDLE {
    windows::Win32::Foundation::HANDLE(server.as_raw_handle() as *mut _)
}

fn open_client(name: &[u16]) -> io::Result<File> {
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            windows::Win32::Foundation::GENERIC_READ.0
                | windows::Win32::Foundation::GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
    }
    .map_err(|error| io::Error::other(format!("连接命名管道失败: {error}")))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::wide;

    #[test]
    fn pipe_names_are_null_terminated_utf16() {
        assert_eq!(
            wide("\\\\.\\pipe\\transom"),
            [
                b'\\' as u16,
                b'\\' as u16,
                b'.' as u16,
                b'\\' as u16,
                b'p' as u16,
                b'i' as u16,
                b'p' as u16,
                b'e' as u16,
                b'\\' as u16,
                b't' as u16,
                b'r' as u16,
                b'a' as u16,
                b'n' as u16,
                b's' as u16,
                b'o' as u16,
                b'm' as u16,
                0,
            ]
        );
    }
}
