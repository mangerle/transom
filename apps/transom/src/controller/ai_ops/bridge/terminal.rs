//! 终端上下文反向请求桥接。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::oneshot;
use transom_ai::{TerminalContextProvider, ToolError};
use transom_core::{AiAppEvent, AiTerminalContextKind, AppEvent, TabId};
use uuid::Uuid;

/// 通过事件总线向 GPUI 终端实体反向请求 OSC 133 命令块或屏幕内容。
pub(in crate::controller) struct UiTerminalBridge {
    pub(in crate::controller) tab_id: TabId,
    pub(in crate::controller) event_tx: flume::Sender<AppEvent>,
    pub(in crate::controller) pending: Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>>,
}

#[async_trait]
impl TerminalContextProvider for UiTerminalBridge {
    async fn recent_blocks(&self, limit: usize) -> Result<String, ToolError> {
        self.request(AiTerminalContextKind::RecentBlocks { limit })
            .await
    }

    async fn read_screen_tail(&self, lines: usize) -> Result<String, ToolError> {
        self.request(AiTerminalContextKind::ScreenTail { lines })
            .await
    }
}

impl UiTerminalBridge {
    async fn request(&self, kind: AiTerminalContextKind) -> Result<String, ToolError> {
        let request_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(request_id.clone(), tx);

        let evt = AppEvent::Ai(Box::new(AiAppEvent::TerminalContextRequested {
            request_id: request_id.clone(),
            tab_id: self.tab_id,
            kind,
        }));
        if self.event_tx.send_async(evt).await.is_err() {
            self.pending.lock().remove(&request_id);
            return Err(ToolError::ExecutionFailed {
                tool: "terminal_context",
                reason: "界面事件通道已关闭".to_owned(),
            });
        }

        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(Some(content))) => Ok(content),
            Ok(Ok(None)) => Ok("当前终端暂无可用记录。".to_owned()),
            Ok(Err(_)) | Err(_) => {
                self.pending.lock().remove(&request_id);
                Err(ToolError::ExecutionTimeout { timeout_secs: 5 })
            }
        }
    }
}
