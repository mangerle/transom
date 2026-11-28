//! AI 命令审批授权的作用域与持久内存仓库。
//!
//! 设计原理（Why）：
//! - “本会话允许同类命令”必须同时绑定对话、主机与保护级别，不能使用进程级全局前缀集合；
//! - 授权判断保持纯内存和同步，避免审批热路径引入异步锁或 I/O；
//! - 仓库只保存已由审批门规范化的安全命令前缀，不负责解析原始命令，避免职责混杂。

use std::collections::{HashMap, HashSet};

pub use transom_core::HostProtection;

/// 一条审批授权的作用域。
///
/// 命名空间用于区分普通对话、外部 MCP 会话和子代理；主机标识与保护级别
/// 用于阻止授权在主机或保护级别变化后继续生效。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ApprovalScope {
    namespace: String,
    host_id: Option<String>,
    protection: HostProtection,
}

impl ApprovalScope {
    /// 构造指定对话/运行命名空间、主机与保护级别的授权作用域。
    pub fn new(
        namespace: impl Into<String>,
        host_id: Option<String>,
        protection: HostProtection,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            host_id: host_id.filter(|host| !host.trim().is_empty()),
            protection,
        }
    }

    /// 构造不绑定主机的全局问答作用域。
    pub fn global(namespace: impl Into<String>) -> Self {
        Self::new(namespace, None, HostProtection::Normal)
    }

    /// 派生一个隔离的子作用域，供子代理避免继承父级临时授权。
    pub fn child(&self, child_id: &str) -> Self {
        Self {
            namespace: format!("{}::{child_id}", self.namespace),
            host_id: self.host_id.clone(),
            protection: self.protection,
        }
    }

    /// 派生同一命名空间下绑定到实际目标主机的授权作用域。
    ///
    /// 跨主机工具在策略通过后必须使用解析后的目标主机，而不是当前活动标签页；
    /// 否则授权可能被错误地复用到另一台主机。
    pub fn with_target(&self, host_id: Option<String>, protection: HostProtection) -> Self {
        Self::new(self.namespace.clone(), host_id, protection)
    }

    /// 授权所属命名空间。
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// 授权绑定的主机标识。
    pub fn host_id(&self) -> Option<&str> {
        self.host_id.as_deref()
    }

    /// 授权绑定的主机保护级别。
    pub fn protection(&self) -> HostProtection {
        self.protection
    }
}

/// 按作用域保存的命令前缀授权仓库。
#[derive(Clone, Debug, Default)]
pub struct ApprovalGrantStore {
    grants: HashMap<ApprovalScope, HashSet<String>>,
}

impl ApprovalGrantStore {
    /// 构造空授权仓库。
    pub fn new() -> Self {
        Self::default()
    }

    /// 为指定作用域登记安全命令前缀；空前缀会被忽略。
    pub fn grant_prefix(&mut self, scope: &ApprovalScope, prefix: &str) -> bool {
        let trimmed = prefix.trim();
        if trimmed.is_empty() {
            return false;
        }
        self.grants
            .entry(scope.clone())
            .or_default()
            .insert(trimmed.to_owned())
    }

    /// 判断规范化命令是否被当前作用域中的授权覆盖。
    pub fn permits(&self, scope: &ApprovalScope, normalized_command: &str) -> bool {
        self.grants.get(scope).is_some_and(|prefixes| {
            prefixes.iter().any(|prefix| {
                normalized_command == prefix
                    || normalized_command
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with(char::is_whitespace))
            })
        })
    }

    /// 清除一个对话或运行命名空间下的全部授权。
    pub fn remove_namespace(&mut self, namespace: &str) -> usize {
        let before = self.grants.len();
        self.grants.retain(|scope, _| {
            let current = scope.namespace();
            current != namespace
                && !current
                    .strip_prefix(namespace)
                    .is_some_and(|suffix| suffix.starts_with("::"))
        });
        before.saturating_sub(self.grants.len())
    }

    /// 清除全部授权，供停机或全局策略变更使用。
    pub fn clear(&mut self) {
        self.grants.clear();
    }

    /// 当前授权条目覆盖的作用域数量。
    pub fn scope_count(&self) -> usize {
        self.grants.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalGrantStore, ApprovalScope, HostProtection};

    fn scope(namespace: &str, host: &str, protection: HostProtection) -> ApprovalScope {
        ApprovalScope::new(namespace, Some(host.to_owned()), protection)
    }

    #[test]
    fn grants_are_isolated_by_namespace_host_and_protection() {
        let mut grants = ApprovalGrantStore::new();
        let normal = scope("chat-1", "host-1", HostProtection::Normal);
        assert!(grants.grant_prefix(&normal, "systemctl restart"));
        assert!(grants.permits(&normal, "systemctl restart nginx"));

        let other_chat = scope("chat-2", "host-1", HostProtection::Normal);
        let other_host = scope("chat-1", "host-2", HostProtection::Normal);
        let production = scope("chat-1", "host-1", HostProtection::Production);
        assert!(!grants.permits(&other_chat, "systemctl restart nginx"));
        assert!(!grants.permits(&other_host, "systemctl restart nginx"));
        assert!(!grants.permits(&production, "systemctl restart nginx"));
    }

    #[test]
    fn removing_namespace_does_not_leave_orphan_grants() {
        let mut grants = ApprovalGrantStore::new();
        grants.grant_prefix(
            &scope("chat-1", "host-1", HostProtection::Normal),
            "docker ps",
        );
        grants.grant_prefix(
            &scope("chat-2", "host-1", HostProtection::Normal),
            "docker ps",
        );
        grants.grant_prefix(
            &scope("chat-1", "host-1", HostProtection::Normal).child("subagent-1"),
            "docker ps",
        );
        assert_eq!(grants.remove_namespace("chat-1"), 2);
        assert_eq!(grants.scope_count(), 1);
    }
}
