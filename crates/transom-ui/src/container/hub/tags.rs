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

const fn preset(
    tag: &'static str,
    desc: &'static str,
    arch: &'static str,
    recommended: bool,
) -> ImageTagPreset {
    ImageTagPreset {
        tag,
        desc,
        arch,
        recommended,
    }
}

const DEFAULT_PRESETS: &[ImageTagPreset] = &[
    preset("latest", "默认官方主标签", "多架构兼容", true),
    preset("stable", "稳定发行分支", "多架构兼容", false),
    preset("alpine", "轻量化 Alpine 基础构建", "多架构兼容", false),
];

const TAG_PRESET_TABLE: &[(&[&str], &[ImageTagPreset])] = &[
    (
        &["mysql"],
        &[
            preset("latest", "官方默认最新版", "amd64 / arm64", false),
            preset("8.4", "最新长期支持版 (8.4 LTS)", "amd64 / arm64", true),
            preset("8.0", "主流成熟生产版 (8.0系列)", "amd64 / arm64", true),
            preset("5.7", "历史兼容稳定版 (5.7系列)", "amd64", false),
        ],
    ),
    (
        &["redis"],
        &[
            preset("latest", "官方最新稳定版", "amd64 / arm64", false),
            preset("7.4", "最新高可用稳定版", "amd64 / arm64", true),
            preset("7.2", "主流稳定分支", "amd64 / arm64", true),
            preset("6.2", "长期维护分支 (6.2系列)", "amd64 / arm64", false),
            preset("alpine", "极简轻量版 (约30MB)", "amd64 / arm64", true),
        ],
    ),
    (
        &["nginx"],
        &[
            preset("latest", "官方默认主线版", "amd64 / arm64", false),
            preset("alpine", "极简轻量构建版", "amd64 / arm64", true),
            preset("1.27", "1.27 主线开发稳定系列", "amd64 / arm64", true),
            preset("1.26", "1.26 官方正式稳定版", "amd64 / arm64", true),
        ],
    ),
    (
        &["postgres", "postgresql"],
        &[
            preset("latest", "官方默认最新版", "amd64 / arm64", false),
            preset("16", "PostgreSQL 16 主流生产版", "amd64 / arm64", true),
            preset("15", "PostgreSQL 15 稳定生产版", "amd64 / arm64", true),
            preset("14", "PostgreSQL 14 稳定维护版", "amd64 / arm64", false),
            preset("alpine", "轻量 Alpine 构建", "amd64 / arm64", true),
        ],
    ),
    (
        &["node"],
        &[
            preset("latest", "Node 当前活跃开发版", "amd64 / arm64", false),
            preset("22", "Node 22 (最新 LTS)", "amd64 / arm64", true),
            preset("20", "Node 20 (长期支持 LTS - Iron)", "amd64 / arm64", true),
            preset("18", "Node 18 (LTS - Hydrogen)", "amd64 / arm64", false),
            preset("alpine", "轻量化 Linux 运行基座", "amd64 / arm64", true),
        ],
    ),
    (
        &["python"],
        &[
            preset("latest", "官方最新默认构建", "amd64 / arm64", false),
            preset("3.12", "Python 3.12 现代主流版", "amd64 / arm64", true),
            preset("3.11", "Python 3.11 性能成熟版", "amd64 / arm64", true),
            preset("3.10", "Python 3.10 长期稳定版", "amd64 / arm64", false),
            preset(
                "slim",
                "Debian 裁剪版 (体积小兼容佳)",
                "amd64 / arm64",
                true,
            ),
            preset("alpine", "极简 Musl 基础构建", "amd64 / arm64", false),
        ],
    ),
    (
        &["alpine"],
        &[
            preset("latest", "最新稳定正式版", "amd64 / arm64 / armv7", true),
            preset("3.20", "3.20 稳定系列构建", "amd64 / arm64", true),
            preset("3.19", "3.19 维护系列构建", "amd64 / arm64", false),
            preset("edge", "Rolling Edge 开发分支", "多架构支持", false),
        ],
    ),
    (
        &["ubuntu"],
        &[
            preset("latest", "当前默认 Ubuntu 发行版", "amd64 / arm64", false),
            preset(
                "24.04",
                "Noble Numbat (最新 24.04 LTS)",
                "amd64 / arm64",
                true,
            ),
            preset(
                "22.04",
                "Jammy Jellyfish (主流 22.04 LTS)",
                "amd64 / arm64",
                true,
            ),
            preset(
                "20.04",
                "Focal Fossa (经典 20.04 LTS)",
                "amd64 / arm64",
                false,
            ),
        ],
    ),
    (
        &["golang"],
        &[
            preset("latest", "官方最新 Go 工具链", "amd64 / arm64", false),
            preset("1.23", "Go 1.23 特性成熟版", "amd64 / arm64", true),
            preset("1.22", "Go 1.22 长期稳定版", "amd64 / arm64", true),
            preset("alpine", "极简轻量编译基底", "amd64 / arm64", true),
        ],
    ),
];

/// 获取指定镜像的常用版本推荐列表。
pub(crate) fn get_image_tag_presets(image_name: &str) -> Vec<ImageTagPreset> {
    // 提取纯镜像名（忽略命名空间，如 bitnami/mysql 同样匹配 mysql 规则）
    let pure_name = image_name
        .split('/')
        .next_back()
        .unwrap_or(image_name)
        .to_lowercase();

    TAG_PRESET_TABLE
        .iter()
        .find(|(names, _)| names.contains(&pure_name.as_str()))
        .map(|(_, presets)| *presets)
        .unwrap_or(DEFAULT_PRESETS)
        .to_vec()
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
