//! Windows 提权本地 PTY 代理。
//!
//! Windows UAC 不能给现有进程内的子进程临时加管理员令牌。为了把管理员 Shell
//! 仍放在当前 Transom 标签中，应用通过 `runas` 拉起同一可执行文件的隐藏代理：
//! 代理在提升后的进程里创建 ConPTY 与 Shell，再通过私有命名管道把原始终端字节
//! 和 resize/close 控制交回当前标签。普通会话不经过此模块。

#[cfg(any(windows, test))]
use std::io::{self, Read, Write};

/// 内部代理参数；普通用户启动不会携带它。
const BROKER_ARG: &str = "--internal-pty-broker";
/// 单次控制帧与握手 JSON 上限。
#[cfg(any(windows, test))]
const MAX_FRAME_BYTES: usize = 64 * 1024;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) use windows::{open_elevated_pty, run_broker};

/// 在任何重量级初始化前识别内部代理模式。
///
/// 返回 `true` 表示当前进程已经作为 PTY 代理执行完毕，调用方应立即退出主函数。
pub(crate) fn run_if_requested() -> anyhow::Result<bool> {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else {
        return Ok(false);
    };
    if first != BROKER_ARG {
        return Ok(false);
    }
    let token = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("提权 PTY 代理缺少实例令牌"))?;
    #[cfg(windows)]
    {
        run_broker(&token)?;
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("提权 PTY 代理仅支持 Windows，收到令牌: {token}")
    }
}

#[cfg(any(windows, test))]
fn write_frame<T: serde::Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PTY 代理握手数据超过上限",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(any(windows, test))]
fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PTY 代理握手数据超过上限",
        ));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::{read_frame, write_frame};
    use std::io::Cursor;

    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Payload {
        shell: String,
        cols: u16,
    }

    #[test]
    fn broker_frames_round_trip() {
        let payload = Payload {
            shell: "powershell.exe".to_string(),
            cols: 120,
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &payload).expect("写入握手帧");
        let decoded: Payload = read_frame(&mut Cursor::new(bytes)).expect("读取握手帧");
        assert_eq!(decoded, payload);
    }
}
