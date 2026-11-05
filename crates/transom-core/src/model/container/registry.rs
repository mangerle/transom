//! 容器镜像仓库（Registry）与远端检索/拉取数据模型。
//!
//! 设计原理（Why）：
//! - 将镜像中心（Search）与推送（Push）的目标仓库统一抽象为 `ContainerRegistry`，
//!   消除在不同交互界面重复配置仓库域名与认证凭据的冗余；
//! - 预置 Docker Hub 官方源作为开箱即用的默认搜索源，同时允许用户自由扩展
//!   私有 Harbor、阿里云 ACR、GitHub Packages (GHCR) 等企业级私有源；
//! - 镜像分层拉取进度抽象为精细化的 `ImageLayerProgress`，支持界面实时渲染
//!   Docker/Podman 每一个 Layer 的下载与解压双阶段百分比。

use super::ContainerRegistryAuth;
use serde::{Deserialize, Serialize};

/// 镜像仓库配置（如 Docker Hub、私有 Harbor、阿里云等）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerRegistry {
    /// 仓库唯一标识符。
    pub id: String,
    /// 仓库展示名称（如 "Docker Hub (官方)", "公司内部 Harbor"）。
    pub name: String,
    /// 仓库服务地址（如 "https://registry-1.docker.io", "https://harbor.internal.com"）。
    pub url: String,
    /// 可选认证用户名。
    #[serde(default)]
    pub username: String,
    /// 可选认证密码或 Token。
    #[serde(default)]
    pub password: String,
    /// 是否为默认选中的搜索源。
    #[serde(default)]
    pub is_default: bool,
}

impl ContainerRegistry {
    /// 构造官方 Docker Hub 默认仓库项。
    pub fn docker_hub() -> Self {
        Self {
            id: "docker-hub".to_string(),
            name: "Docker Hub (官方)".to_string(),
            url: "https://registry-1.docker.io".to_string(),
            username: String::new(),
            password: String::new(),
            is_default: true,
        }
    }

    /// 提取用于镜像 Tag 前缀的域名/主机名（例如从 "https://harbor.example.com" 提取 "harbor.example.com"）。
    pub fn domain(&self) -> &str {
        let trimmed = self
            .url
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        trimmed.split('/').next().unwrap_or(trimmed)
    }

    /// 是否为 Docker Hub；Hub 的搜索入口与私有 Registry 不同。
    pub fn is_docker_hub(&self) -> bool {
        let domain = self.domain().to_ascii_lowercase();
        matches!(
            domain.as_str(),
            "docker.io" | "index.docker.io" | "registry-1.docker.io" | "hub.docker.com"
        ) || self.id.eq_ignore_ascii_case("docker-hub")
    }

    /// 把仓库名与标签拼成可直接交给 Docker 的完整引用。
    ///
    /// Docker Hub 不需要域名前缀；私有仓库补 `domain/`；仓库名已包含域名时
    /// 不重复拼接，避免出现 `harbor/harbor/app`。
    pub fn qualify_reference(&self, repository: &str, tag: &str) -> String {
        let repo = repository.trim().trim_start_matches('/');
        if repo.is_empty() {
            return String::new();
        }
        let domain = self.domain();
        let base = if self.is_docker_hub() || repo.starts_with(&format!("{domain}/")) {
            repo.to_string()
        } else {
            format!("{domain}/{repo}")
        };
        let tag = tag.trim();
        if tag.is_empty() || base.contains('@') {
            base
        } else {
            format!("{base}:{tag}")
        }
    }

    /// 构造拉取/推送所需的认证凭据；未配置用户名时返回 `None`。
    pub fn auth(&self) -> Option<ContainerRegistryAuth> {
        if self.username.is_empty() {
            return None;
        }
        Some(ContainerRegistryAuth {
            username: self.username.clone(),
            password: self.password.clone(),
            server_address: Some(self.url.clone()),
            identity_token: None,
        })
    }
}

/// 远端镜像检索结果条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ImageSearchResult {
    /// 镜像名称（如 "nginx", "bitnami/redis"）。
    pub name: String,
    /// 镜像简要描述。
    #[serde(default)]
    pub description: String,
    /// 是否为 Docker 官方认证镜像。
    #[serde(default)]
    pub is_official: bool,
    /// 是否为自动构建镜像。
    #[serde(default)]
    pub is_automated: bool,
    /// 社区收藏/Star 计数。
    #[serde(default)]
    pub star_count: u64,
}

/// 镜像拉取单层进度。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ImageLayerProgress {
    /// 分层标识（短 SHA，如 "2d4e3a1f"）。
    pub id: String,
    /// 当前状态文本（如 "Downloading", "Extracting", "Pull complete"）。
    pub status: String,
    /// 当前已传输字节数。
    #[serde(default)]
    pub current: u64,
    /// 该层总字节数（若未知则为 0）。
    #[serde(default)]
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_hub_has_valid_defaults() {
        let hub = ContainerRegistry::docker_hub();
        assert_eq!(hub.id, "docker-hub");
        assert!(hub.is_default);
        assert_eq!(hub.domain(), "registry-1.docker.io");
    }

    #[test]
    fn registry_domain_extracts_host() {
        let reg = ContainerRegistry {
            id: "harbor".into(),
            name: "Harbor".into(),
            url: "https://harbor.internal.company.com/v2".into(),
            username: "".into(),
            password: "".into(),
            is_default: false,
        };
        assert_eq!(reg.domain(), "harbor.internal.company.com");
    }

    #[test]
    fn docker_hub_reference_has_no_domain_prefix() {
        let hub = ContainerRegistry::docker_hub();
        assert!(hub.is_docker_hub());
        assert_eq!(hub.qualify_reference("nginx", "latest"), "nginx:latest");
        assert_eq!(
            hub.qualify_reference("bitnami/redis", "7.2"),
            "bitnami/redis:7.2"
        );
    }

    #[test]
    fn private_registry_reference_prefixes_domain_once() {
        let registry = ContainerRegistry {
            id: "harbor".into(),
            name: "Harbor".into(),
            url: "https://harbor.example.com".into(),
            username: "user".into(),
            password: "secret".into(),
            is_default: false,
        };
        assert!(!registry.is_docker_hub());
        assert_eq!(
            registry.qualify_reference("library/nginx", "1.27"),
            "harbor.example.com/library/nginx:1.27"
        );
        assert_eq!(
            registry.qualify_reference("harbor.example.com/library/nginx", "1.27"),
            "harbor.example.com/library/nginx:1.27"
        );
        assert!(registry.auth().is_some());
    }
}
