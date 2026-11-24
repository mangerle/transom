//! RDP CLIPRDR 虚拟通道后端实现与本地系统剪贴板桥接。
//!
//! 设计原理（Why）：根据 [MS-RDPECLIP] 协议规范，远端 Windows 系统与本地客户端之间的
//! 剪贴板同步基于延迟渲染机制（Delayed Rendering）。当一方执行复制时，仅广播可用格式清单
//! （FormatList）；另一方按需发起数据提取请求（FormatDataRequest），再由数据持有端回传实际载荷。
//! 本模块实现 `ironrdp_cliprdr::backend::CliprdrBackend` 特型，与主数据流异步解耦。

use crate::clipboard::ClipboardEvent;
use crate::clipboard::text::{decode_ansi_payload, decode_text_payload};
use crate::session::channel::RdpOutput;
use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_core::impl_as_any;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// 剪贴板跨线程同步状态上下文。
#[derive(Debug, Default)]
pub struct CliprdrSyncContext {
    /// 远端宣告可用且待拉取的具体文本格式（优先 `CF_UNICODETEXT`，回退 `CF_TEXT`）。
    pub pending_paste_format: Option<ClipboardFormatId>,
    /// 最近一次发起拉取时请求的格式（用于准确选择 UTF-16LE 或 ANSI 解码器）。
    pub inflight_paste_format: Option<ClipboardFormatId>,
    /// 远端是否正在请求本地提供指定格式的剪贴板数据。
    pub pending_data_request: Option<ClipboardFormatId>,
    /// 本地最新待共享给远端的纯文本数据。
    pub local_text: Option<String>,
    /// 最近一次同步自远端的文本（用于防回环震荡）。
    pub last_remote_text: Option<String>,
}

/// Transom 原生剪贴板通道后端。
#[derive(Debug)]
pub struct TransomCliprdrBackend {
    context: Arc<Mutex<CliprdrSyncContext>>,
    output_tx: mpsc::Sender<RdpOutput>,
}

impl_as_any!(TransomCliprdrBackend);

impl TransomCliprdrBackend {
    /// 构造新的剪贴板通道后端实例。
    pub fn new(
        context: Arc<Mutex<CliprdrSyncContext>>,
        output_tx: mpsc::Sender<RdpOutput>,
    ) -> Self {
        Self { context, output_tx }
    }
}

impl CliprdrBackend for TransomCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        log::info!("RDP CLIPRDR 剪贴板虚拟通道初始化就绪");
    }

    fn on_request_format_list(&mut self) {
        log::debug!("收到远端剪贴板格式清单请求指令");
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        log::debug!("CLIPRDR 协商完成能力集合: {capabilities:?}");
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        let preferred = if available_formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT)
        {
            Some(ClipboardFormatId::CF_UNICODETEXT)
        } else if available_formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_TEXT)
        {
            Some(ClipboardFormatId::CF_TEXT)
        } else {
            None
        };
        if let Some(format) = preferred {
            self.context.lock().pending_paste_format = Some(format);
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.context.lock().pending_data_request = Some(request.format);
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let requested_format = self.context.lock().inflight_paste_format.take();
        if response.is_error() {
            log::debug!("远端返回剪贴板数据提取失败响应");
            return;
        }

        let decoded = match requested_format {
            Some(fmt) if fmt == ClipboardFormatId::CF_TEXT => decode_ansi_payload(response.data()),
            _ => decode_text_payload(response.data()),
        };

        if let Some(text) = decoded {
            let mut ctx = self.context.lock();
            if ctx.last_remote_text.as_deref() != Some(&text) {
                ctx.last_remote_text = Some(text.clone());
                let _ = self
                    .output_tx
                    .try_send(RdpOutput::Clipboard(ClipboardEvent::RemoteText(text)));
            }
        }
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}

    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}
