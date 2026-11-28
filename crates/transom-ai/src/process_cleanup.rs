//! AI 运行时外部子进程的确定性终止与状态收割。

use std::io;
use std::time::Duration;

use tokio::process::Child;

/// 终止子进程并显式等待其退出，确保异常路径也回收进程控制块。
pub(crate) async fn terminate_child_and_reap(child: &mut Child, context: &str) {
    if let Err(err) = child.kill().await
        && err.kind() != io::ErrorKind::InvalidInput
    {
        log::warn!("终止外部进程失败（{context}）: {err}");
    }
    match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => log::warn!("等待外部进程回收失败（{context}）: {err}"),
        Err(_) => log::warn!("等待外部进程回收超时（2 秒，{context}）"),
    }
}
