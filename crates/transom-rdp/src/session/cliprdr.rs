//! 会话循环中的 CLIPRDR 剪贴板协议驱动辅助模块。
//!
//! 设计原理（Why）：将剪贴板静态虚拟通道的异步消息驱动（延迟渲染粘贴、格式数据应答、
//! 本地复制通告）从主事件循环 `loop_runner` 中剥离，并严格按照对端协商/请求的格式 ID
//! （`CF_UNICODETEXT` / `CF_TEXT`）编解码，杜绝格式错配导致的粘贴乱码。

use crate::clipboard::TransomCliprdrBackend;
use crate::clipboard::text::{encode_ansi_payload, encode_text_payload};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataResponse};
use ironrdp_cliprdr::{Client, Cliprdr};
use ironrdp_core::IntoOwned;
use ironrdp_session::ActiveStage;
use ironrdp_tokio::FramedWrite;

/// 驱动 CLIPRDR 剪贴板事件流转（拉取远端数据或应答远端数据请求）。
pub(crate) async fn drive_cliprdr_events<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
) {
    let (paste_format, pending_requests, local_text) = {
        let Some(cliprdr) = active_stage.get_svc_processor_mut::<Cliprdr<Client>>() else {
            return;
        };
        let Some(backend) = cliprdr.downcast_backend_mut::<TransomCliprdrBackend>() else {
            return;
        };
        let ctx = &mut backend.context;
        let paste = ctx.pending_paste_format.take();
        if let Some(fmt) = paste {
            if ctx.inflight_paste_formats.len() >= 16 {
                ctx.inflight_paste_formats.pop_front();
            }
            ctx.inflight_paste_formats.push_back(fmt);
        }
        let requests: Vec<ClipboardFormatId> = ctx.pending_data_requests.drain(..).collect();
        let text = if requests.is_empty() {
            None
        } else {
            ctx.local_text.clone()
        };
        (paste, requests, text)
    };

    if let Some(format_id) = paste_format
        && let Some(cliprdr) = active_stage.get_svc_processor_mut::<Cliprdr<Client>>()
        && let Ok(messages) = cliprdr.initiate_paste(format_id)
        && let Ok(bytes) = active_stage.process_svc_processor_messages::<Cliprdr<Client>>(messages)
        && let Err(e) = writer.write_all(&bytes).await
    {
        log::debug!("写入 CLIPRDR 粘贴请求失败: {e}");
    }

    for requested_format in pending_requests {
        let response = build_format_data_response(requested_format, local_text.as_deref());

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

/// 处理本地剪贴板更新并向远端通告（同时声明 Unicode 与 ANSI 文本格式以兼容各类远端应用）。
pub(crate) async fn handle_local_clipboard_copy<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    text: String,
) {
    let formats = [
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(ClipboardFormatId::CF_TEXT),
    ];
    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<Cliprdr<Client>>() {
        if let Some(backend) = cliprdr.downcast_backend_mut::<TransomCliprdrBackend>() {
            backend.context.local_text = Some(text);
        }
        if let Ok(messages) = cliprdr.initiate_copy(&formats)
            && let Ok(bytes) =
                active_stage.process_svc_processor_messages::<Cliprdr<Client>>(messages)
            && let Err(e) = writer.write_all(&bytes).await
        {
            log::debug!("写入 CLIPRDR 本地复制通告失败: {e}");
        }
    }
}

fn build_format_data_response(
    format: ClipboardFormatId,
    local_text: Option<&str>,
) -> FormatDataResponse<'static> {
    let Some(text) = local_text else {
        return FormatDataResponse::new_error().into_owned();
    };
    if format == ClipboardFormatId::CF_UNICODETEXT {
        FormatDataResponse::new_data(encode_text_payload(text)).into_owned()
    } else if format == ClipboardFormatId::CF_TEXT {
        FormatDataResponse::new_data(encode_ansi_payload(text)).into_owned()
    } else {
        FormatDataResponse::new_error().into_owned()
    }
}
