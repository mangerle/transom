//! 镜像仓库版本标签预设与推导引擎。
//!
//! 设计原理（Why）：
//! - Docker daemon 底层 /images/search 仅返回仓库概要而不提供版本标签；
//! - 内置主流官方镜像的高频版本预设，提供零延迟的秒级推荐体验；
//! - 支持从搜索输入（如 `mysql:8.0`）中智能解构镜像名与版本标签。

/// 单个版本标签的预设元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageTagPreset {
    /// 标签名称（如 `latest`, `8.4-lts`, `8.0`, `5.7`）。
    pub(crate) tag: &'static str,
    /// 版本特征说明（如 `官方默认最新版`, `长期支持 LTS`, `经典生产稳定版`）。
    pub(crate) desc: &'static str,
    /// 典型架构支持。
    pub(crate) arch: &'static str,
    /// 是否推荐作为生产首选。
    pub(crate) recommended: bool,
}

/// 解析用户在搜索框中的输入，支持智能解构镜像名与版本号（例如 `mysql:8.0` 拆分为 `("mysql", Some("8.0"))`）。
pub(crate) fn parse_search_term_and_tag(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim();
    if let Some((img, tag)) = trimmed.split_once(':') {
        let img = img.trim().to_string();
        let tag = tag.trim().to_string();
        if !img.is_empty() && !tag.is_empty() {
            return (img, Some(tag));
        }
    }
    (trimmed.to_string(), None)
}

/// 获取指定镜像的常用版本推荐列表。
pub(crate) fn get_image_tag_presets(image_name: &str) -> Vec<ImageTagPreset> {
    // 提取纯镜像名（忽略命名空间，如 bitnami/mysql 同样匹配 mysql 规则）
    let pure_name = image_name
        .split('/')
        .next_back()
        .unwrap_or(image_name)
        .to_lowercase();

    match pure_name.as_str() {
        "mysql" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "官方默认最新版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "8.4",
                desc: "最新长期支持版 (8.4 LTS)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "8.0",
                desc: "主流成熟生产版 (8.0系列)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "5.7",
                desc: "历史兼容稳定版 (5.7系列)",
                arch: "amd64",
                recommended: false,
            },
        ],
        "redis" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "官方最新稳定版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "7.4",
                desc: "最新高可用稳定版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "7.2",
                desc: "主流稳定分支",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "6.2",
                desc: "长期维护分支 (6.2系列)",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "极简轻量版 (约30MB)",
                arch: "amd64 / arm64",
                recommended: true,
            },
        ],
        "nginx" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "官方默认主线版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "极简轻量构建版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "1.27",
                desc: "1.27 主线开发稳定系列",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "1.26",
                desc: "1.26 官方正式稳定版",
                arch: "amd64 / arm64",
                recommended: true,
            },
        ],
        "postgres" | "postgresql" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "官方默认最新版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "16",
                desc: "PostgreSQL 16 主流生产版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "15",
                desc: "PostgreSQL 15 稳定生产版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "14",
                desc: "PostgreSQL 14 稳定维护版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "轻量 Alpine 构建",
                arch: "amd64 / arm64",
                recommended: true,
            },
        ],
        "node" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "Node 当前活跃开发版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "22",
                desc: "Node 22 (最新 LTS)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "20",
                desc: "Node 20 (长期支持 LTS - Iron)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "18",
                desc: "Node 18 (LTS - Hydrogen)",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "轻量化 Linux 运行基座",
                arch: "amd64 / arm64",
                recommended: true,
            },
        ],
        "python" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "官方最新默认构建",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "3.12",
                desc: "Python 3.12 现代主流版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "3.11",
                desc: "Python 3.11 性能成熟版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "3.10",
                desc: "Python 3.10 长期稳定版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "slim",
                desc: "Debian 裁剪版 (体积小兼容佳)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "极简 Musl 基础构建",
                arch: "amd64 / arm64",
                recommended: false,
            },
        ],
        "alpine" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "最新稳定正式版",
                arch: "amd64 / arm64 / armv7",
                recommended: true,
            },
            ImageTagPreset {
                tag: "3.20",
                desc: "3.20 稳定系列构建",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "3.19",
                desc: "3.19 维护系列构建",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "edge",
                desc: "Rolling Edge 开发分支",
                arch: "多架构支持",
                recommended: false,
            },
        ],
        "ubuntu" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "当前默认 Ubuntu 发行版",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "24.04",
                desc: "Noble Numbat (最新 24.04 LTS)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "22.04",
                desc: "Jammy Jellyfish (主流 22.04 LTS)",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "20.04",
                desc: "Focal Fossa (经典 20.04 LTS)",
                arch: "amd64 / arm64",
                recommended: false,
            },
        ],
        "golang" => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "官方最新 Go 工具链",
                arch: "amd64 / arm64",
                recommended: false,
            },
            ImageTagPreset {
                tag: "1.23",
                desc: "Go 1.23 特性成熟版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "1.22",
                desc: "Go 1.22 长期稳定版",
                arch: "amd64 / arm64",
                recommended: true,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "极简轻量编译基底",
                arch: "amd64 / arm64",
                recommended: true,
            },
        ],
        _ => vec![
            ImageTagPreset {
                tag: "latest",
                desc: "默认官方主标签",
                arch: "多架构兼容",
                recommended: true,
            },
            ImageTagPreset {
                tag: "stable",
                desc: "稳定发行分支",
                arch: "多架构兼容",
                recommended: false,
            },
            ImageTagPreset {
                tag: "alpine",
                desc: "轻量化 Alpine 基础构建",
                arch: "多架构兼容",
                recommended: false,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_search_term_and_tag() {
        assert_eq!(
            parse_search_term_and_tag("mysql:8.0"),
            ("mysql".to_string(), Some("8.0".to_string()))
        );
        assert_eq!(
            parse_search_term_and_tag("redis:alpine"),
            ("redis".to_string(), Some("alpine".to_string()))
        );
        assert_eq!(
            parse_search_term_and_tag("nginx"),
            ("nginx".to_string(), None)
        );
        assert_eq!(
            parse_search_term_and_tag("  bitnami/mysql:5.7  "),
            ("bitnami/mysql".to_string(), Some("5.7".to_string()))
        );
    }

    #[test]
    fn test_get_image_tag_presets() {
        let mysql_tags = get_image_tag_presets("mysql");
        assert!(mysql_tags.iter().any(|t| t.tag == "8.0"));
        assert!(mysql_tags.iter().any(|t| t.tag == "latest"));

        let bitnami_redis = get_image_tag_presets("bitnami/redis");
        assert!(bitnami_redis.iter().any(|t| t.tag == "alpine"));

        let unknown = get_image_tag_presets("my-custom-app");
        assert!(unknown.iter().any(|t| t.tag == "latest"));
    }
}
