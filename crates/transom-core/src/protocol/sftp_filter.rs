//! SFTP 目录传输的排除规则：把用户输入的通配符编译为可复用的匹配器。

use crate::{AppError, ErrorCode};
use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

/// 默认排除的常见垃圾目录，首次使用时写入过滤规则输入框。
pub const DEFAULT_TRANSFER_EXCLUDES: &str = ".git/\nnode_modules/";

/// 面向用户的传输排除规则，按行保存原始通配符文本。
///
/// 设计原理（Why）：
/// - 实现初衷：过滤规则需要跨设置持久化与在后台传输任务间传递，因此只保存
///   简洁的字符串列表，编译后的匹配器在真正扫描目录时才构建；
/// - 核心优势：规则文本可直接展示与编辑，序列化稳定，新增规则语法无需迁移数据结构；
/// - 代价与局限：每次目录传输都会重新编译规则，规则数量通常很少，开销可忽略。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferFilter {
    #[serde(default)]
    pub patterns: Vec<String>,
}

impl TransferFilter {
    pub fn from_text(text: &str) -> Self {
        let patterns = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect();
        Self { patterns }
    }

    pub fn to_text(&self) -> String {
        self.patterns.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// 编译排除规则。
    ///
    /// 对 `xxx/**` 形式额外生成一条 `xxx/` 目录规则，使匹配到的目录本身也能
    /// 被剪枝，避免必须深入其子项才发现全部被排除。
    ///
    /// # Errors
    /// 任意一行不是合法通配符时返回 `ErrorCode::Configuration`，
    /// 错误详情包含具体规则文本，便于用户在设置弹窗内定位修正。
    pub fn matcher(&self) -> Result<TransferMatcher, AppError> {
        let mut rules = Vec::with_capacity(self.patterns.len());
        for raw in &self.patterns {
            rules.push(CompiledRule::parse(raw)?);
            if let Some(prefix) = raw.trim().strip_suffix("/**")
                && !prefix.trim_end_matches('/').is_empty()
            {
                rules.push(CompiledRule::parse_path_dir(prefix)?);
            }
        }
        Ok(TransferMatcher { rules })
    }
}

/// 编译后的排除规则集合。
///
/// 匹配语义（Why）：
/// - 不含 `/` 的规则按“任意层级的名称”匹配，例如 `node_modules/` 可命中
///   `a/b/node_modules`，符合用户排除垃圾目录的直觉；
/// - 含 `/` 的规则按“相对传输根目录的路径”匹配，例如 `build/**` 只影响根下
///   `build` 子树，避免误伤其他同名目录；
/// - 末尾带 `/` 的规则只匹配目录；命中目录时调用方会直接剪枝，不再继续遍历
///   其子项，从而保证大目录（如 `.git`）不会被无谓扫描。
pub struct TransferMatcher {
    rules: Vec<CompiledRule>,
}

impl TransferMatcher {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// 判断相对路径是否应被排除；`relative` 使用 `/` 分隔且不含开头 `/`。
    pub fn matches(&self, relative: &str, is_dir: bool) -> bool {
        if self.rules.is_empty() || relative.is_empty() {
            return false;
        }
        self.rules.iter().any(|rule| rule.matches(relative, is_dir))
    }
}

/// 校验过滤规则文本是否可以编译，供 UI 保存前提示错误。
///
/// # Errors
/// 规则非法时返回 `ErrorCode::Configuration`，详情为中文可读原因。
pub fn validate_transfer_filter(text: &str) -> Result<(), AppError> {
    TransferFilter::from_text(text).matcher().map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleScope {
    Basename,
    RelativePath,
}

struct CompiledRule {
    glob: GlobMatcher,
    scope: RuleScope,
    dir_only: bool,
}

impl CompiledRule {
    fn parse(raw: &str) -> Result<Self, AppError> {
        let trimmed = raw.trim();
        let dir_only = trimmed.ends_with('/');
        let normalized = normalize_pattern(trimmed.trim_end_matches('/'), raw)?;
        let scope = if normalized.contains('/') {
            RuleScope::RelativePath
        } else {
            RuleScope::Basename
        };
        Ok(Self {
            glob: compile_glob(&normalized, raw)?,
            scope,
            dir_only,
        })
    }

    /// `xxx/**` 的前缀目录规则必须始终按相对路径匹配，不能退化成任意层级同名目录。
    fn parse_path_dir(raw: &str) -> Result<Self, AppError> {
        let normalized = normalize_pattern(raw.trim_end_matches('/'), raw)?;
        Ok(Self {
            glob: compile_glob(&normalized, raw)?,
            scope: RuleScope::RelativePath,
            dir_only: true,
        })
    }

    fn matches(&self, relative: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        match self.scope {
            RuleScope::Basename => relative
                .rsplit('/')
                .next()
                .is_some_and(|name| self.glob.is_match(name)),
            RuleScope::RelativePath => self.glob.is_match(relative),
        }
    }
}

fn normalize_pattern(pattern: &str, raw: &str) -> Result<String, AppError> {
    let normalized = pattern
        .strip_prefix("./")
        .unwrap_or(pattern)
        .trim_start_matches('/');
    if normalized.is_empty() || normalized == "." || normalized == ".." {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            format!("排除规则无效: {raw}"),
        ));
    }
    Ok(normalized.to_string())
}

fn compile_glob(pattern: &str, raw: &str) -> Result<GlobMatcher, AppError> {
    Glob::new(pattern)
        .map(|glob| glob.compile_matcher())
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("排除规则无法解析: {raw}（{error}）"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_TRANSFER_EXCLUDES, TransferFilter, validate_transfer_filter};

    fn matcher(text: &str) -> super::TransferMatcher {
        TransferFilter::from_text(text)
            .matcher()
            .expect("规则应合法")
    }

    #[test]
    fn basename_directory_rules_match_any_depth() {
        let rules = matcher(DEFAULT_TRANSFER_EXCLUDES);
        assert!(rules.matches("a/.git", true));
        assert!(rules.matches("deep/node_modules", true));
        assert!(rules.matches("node_modules", true));
        assert!(!rules.matches("a/.git", false));
        assert!(!rules.matches("a/node_modules_backup", true));
        assert!(!rules.matches("src/main.rs", false));
    }

    #[test]
    fn wildcard_basename_rules_match_files_at_any_depth() {
        let rules = matcher("*.tmp\n# 注释行\n\n*.log");
        assert!(rules.matches("a/b/cache.tmp", false));
        assert!(rules.matches("server.log", false));
        assert!(!rules.matches("a/cache.tmp/keep.txt", false));
        assert!(!rules.matches("cache.tmpl", false));
    }

    #[test]
    fn path_rules_are_root_relative() {
        let rules = matcher("build/**\n/target/\nsub/cache");
        assert!(rules.matches("build/output/app.bin", false));
        assert!(rules.matches("build", true));
        assert!(rules.matches("target", true));
        assert!(rules.matches("sub/cache", false));
        assert!(!rules.matches("src/build", true));
        assert!(!rules.matches("deep/sub/cache", false));
    }

    #[test]
    fn invalid_patterns_are_rejected_with_readable_error() {
        assert!(validate_transfer_filter("ok/\n[broken").is_err());
        assert!(validate_transfer_filter("/").is_err());
        assert!(validate_transfer_filter("..").is_err());
        assert!(validate_transfer_filter(DEFAULT_TRANSFER_EXCLUDES).is_ok());
    }

    #[test]
    fn text_round_trip_ignores_comments_and_blank_lines() {
        let filter = TransferFilter::from_text("  .git/  \n\n# 注释\nnode_modules/\n");
        assert_eq!(filter.patterns, vec![".git/", "node_modules/"]);
        assert_eq!(filter.to_text(), ".git/\nnode_modules/");
    }
}
