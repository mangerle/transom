//! 引擎连接服务：把“保存的引擎配置”解析成可用的 [`Engine`]。
//!
//! 设计原理（Why）：`Auto` 端点需要按平台发现，显式端点只需探测；把这段
//! 逻辑集中在一个服务里，控制器只关心“连接成功/失败”，表单也能复用同一
//! 条路径做“测试连接”。

use crate::engine::{self, Engine};
use crate::error::ContainerError;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use transom_core::{ContainerEngineProfile, ContainerRequestId};

/// 容器接入服务（随控制器单例）。
#[derive(Default)]
pub struct ContainerService {
    /// 进行中的可取消操作：关闭页签/工作区时置位，长任务轮询检查。
    cancels: Mutex<HashMap<ContainerRequestId, Arc<AtomicBool>>>,
}

impl ContainerService {
    pub fn new() -> Self {
        Self::default()
    }

    /// 解析并探测一个保存的引擎连接。
    ///
    /// # Errors
    /// `Auto` 发现不到该种类引擎返回 [`ContainerError::NoEngine`]；显式端点
    /// 不可达或探测失败时返回对应的连接/协议错误。
    pub async fn connect(
        &self,
        profile: &ContainerEngineProfile,
    ) -> Result<Engine, ContainerError> {
        if profile.engine.endpoint.is_auto() {
            let (engines, failures) = engine::discover_all().await;
            engines
                .into_iter()
                .find(|engine| engine.info.engine.kind == profile.engine.kind)
                .ok_or_else(|| ContainerError::NoEngine {
                    detail: if failures.is_empty() {
                        "没有找到候选端点".to_string()
                    } else {
                        failures.join("；")
                    },
                })
        } else {
            engine::probe_explicit(profile.engine.kind, &profile.engine.endpoint).await
        }
    }

    /// 登记一次可取消操作，返回取消标志。
    pub fn register_cancel(&self, request_id: ContainerRequestId) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut cancels) = self.cancels.lock() {
            cancels.insert(request_id, Arc::clone(&flag));
        }
        flag
    }

    /// 置位取消标志；请求不存在时返回 `false`。
    pub fn cancel(&self, request_id: ContainerRequestId) -> bool {
        match self.cancels.lock() {
            Ok(cancels) => cancels.get(&request_id).is_some_and(|flag| {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }),
            Err(_) => false,
        }
    }

    /// 操作结束后移除登记，避免取消表无限增长。
    pub fn finish(&self, request_id: ContainerRequestId) {
        if let Ok(mut cancels) = self.cancels.lock() {
            cancels.remove(&request_id);
        }
    }

    /// 当前是否已被取消。
    pub fn is_cancelled(&self, request_id: ContainerRequestId) -> bool {
        self.cancels
            .lock()
            .ok()
            .and_then(|cancels| {
                cancels
                    .get(&request_id)
                    .map(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::ContainerService;
    use transom_core::ContainerRequestId;

    #[test]
    fn cancel_flags_are_registered_and_cleaned_up() {
        let service = ContainerService::new();
        let request = ContainerRequestId::new();
        let flag = service.register_cancel(request);
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
        assert!(service.cancel(request));
        assert!(service.is_cancelled(request));
        service.finish(request);
        assert!(!service.is_cancelled(request));
        assert!(!service.cancel(request));
    }
}
