//! `--mcp-stdio` 子命令桥接：将标准输入输出 JSON-RPC 行透明转发至运行中的 Transom 本地 MCP Socket。

use anyhow::Context as _;
use transom_store::AiStore;

/// 若命令行包含 `--mcp-stdio`，则进入 MCP stdio 转发模式并返回 `Ok(true)`。
pub(crate) fn run_if_requested() -> anyhow::Result<bool> {
    if !std::env::args().any(|arg| arg == "--mcp-stdio") {
        return Ok(false);
    }

    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::net::UnixStream;

        let socket_path = AiStore::new().root().join("mcp.sock");
        let mut sock_read = UnixStream::connect(&socket_path).with_context(|| {
            format!(
                "无法连接运行中的 Transom MCP 服务（{}），请先启动 Transom 主程序",
                socket_path.display()
            )
        })?;
        let mut sock_write = sock_read
            .try_clone()
            .context("克隆本地 MCP Socket 写端失败")?;

        let up = std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let _ = std::io::copy(&mut stdin, &mut sock_write);
            let _ = sock_write.flush();
            let _ = sock_write.shutdown(std::net::Shutdown::Write);
        });
        let mut stdout = std::io::stdout().lock();
        let _ = std::io::copy(&mut sock_read, &mut stdout);
        let _ = stdout.flush();
        let _ = up.join();
        Ok(true)
    }

    #[cfg(not(unix))]
    {
        anyhow::bail!("当前平台暂不支持 `--mcp-stdio` 本地 Socket 桥接");
    }
}
