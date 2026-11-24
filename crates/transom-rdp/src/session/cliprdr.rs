//! 会话循环中的 CLIPRDR 剪贴板协议驱动辅助模块。
//!
//! 设计原理（Why）：将剪贴板静态虚拟通道的异步消息驱动（延迟渲染粘贴、格式数据应答、
//! 本地复制通告）从主事件循环 `loop_runner` 中剥离，降低主循环复杂度并保持高内聚。

use crate::clipboard::CliprdrSyncContext;
use crate::clipboard::text::encode_text_payload;
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataResponse};
use ironrdp_cliprdr::{Client, Cliprdr};
use ironrdp_core::IntoOwned;
use ironrdp_session::ActiveStage;
use ironrdp_tokio::FramedWrite;
use parking_lot::Mutex;
use std::sync::Arc;

/// 驱动 CLIPRDR 剪贴板事件流转（拉取远端数据或应答远端数据请求）。
pub(crate) async fn drive_cliprdr_events<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    context: &Arc<Mutex<CliprdrSyncContext>>,
) {
    let (need_paste, pending_format) = {
        let mut ctx = context.lock();
        let paste = ctx.pending_paste_text;
        ctx.pending_paste_text = false;
        let format = ctx.pending_data_request.take();
        (paste, format)
    };

    if need_paste
        && let Some(cliprdr) = active_stage.get_svc_processor_mut::<Cliprdr<Client>>()
        && let Ok(messages) = cliprdr.initiate_paste(ClipboardFormatId::CF_UNICODETEXT)
        && let Ok(bytes) = active_stage.process_svc_processor_messages::<Cliprdr<Client>>(messages)
        && let Err(e) = writer.write_all(&bytes).await
    {
        log::debug!("写入 CLIPRDR 粘贴请求失败: {e}");
    }

    if let Some(_format) = pending_format {
        let local_text = context.lock().local_text.clone();
        let response = if let Some(text) = local_text {
            FormatDataResponse::new_data(encode_text_payload(&text)).into_owned()
        } else {
            FormatDataResponse::new_error().into_owned()
        };

        if let Some(cliprdr) = active_stage.get_svc_processor_mut::<Cliprdr<Client>>()
            && let Ok(messages) = cliprdr.submit_format_data(response)
            && let Ok(bytes) =
                active_stage.process_svc_processor_messages::<Cliprdr<Client>>(messages)
            && let Err(e) = writer.write_all(&bytes).await
        {
            log::debug!("写入 CLIPRDR 格式数据响应失败: {e}");
        }
    }
}

/// 处理本地剪贴板更新并向远端通告。
pub(crate) async fn handle_local_clipboard_copy<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    context: &Arc<Mutex<CliprdrSyncContext>>,
    text: String,
) {
    context.lock().local_text = Some(text);
    let formats = [ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<Cliprdr<Client>>()
        && let Ok(messages) = cliprdr.initiate_copy(&formats)
        && let Ok(bytes) = active_stage.process_svc_processor_messages::<Cliprdr<Client>>(messages)
        && let Err(e) = writer.write_all(&bytes).await
    {
        log::debug!("写入 CLIPRDR 本地复制通告失败: {e}");
    }
}
