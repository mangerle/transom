//! 钥匙串凭据回填：并行读取多个槽位并写回输入框。
//!
//! 设计原理（Why）：编辑会话与编辑凭据模板都需要"打开表单时把已存口令
//! 填回输入框"，两处此前逐字复制同一段后台读取与窗口回写逻辑。钥匙串
//! 访问走 D-Bus 可能挂起数秒，必须在后台执行；回写必须回到 UI 线程。

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{SecretKey, SecretKind, SecretOwner, SecretStore};

/// 并行读取 `fields` 对应的钥匙串槽位，并把非空值回填到输入框。
///
/// 读取失败或槽位不存在时保持输入框现状，不阻断表单打开；任务随窗口
/// 生命周期结束，不保留额外句柄。
pub(crate) fn prefill_secret_fields<E: 'static>(
    owner: SecretOwner,
    fields: Vec<(SecretKind, Entity<InputState>)>,
    secret_store: Arc<dyn SecretStore>,
    window: &mut Window,
    cx: &mut Context<E>,
) {
    if fields.is_empty() {
        return;
    }
    let executor = cx.background_executor().clone();
    let read_fields = fields.clone();
    window
        .spawn(cx, async move |cx| {
            let secrets = executor
                .spawn(async move {
                    read_fields
                        .iter()
                        .map(|(kind, _)| {
                            secret_store
                                .get(&SecretKey::new(owner, *kind))
                                .ok()
                                .flatten()
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = cx.update(|window, cx| {
                for ((_, field), secret) in fields.into_iter().zip(secrets) {
                    if let Some(secret) = secret {
                        field.update(cx, |state, cx| {
                            state.set_value(secret.as_str(), window, cx);
                        });
                    }
                }
            });
        })
        .detach();
}
