//! .dockerignore 解析与匹配规则。

use globset::{Glob, GlobMatcher};
use std::path::Path;

/// 单条 `.dockerignore` 规则；按出现顺序应用，后面的规则覆盖前面的结果。
#[derive(Debug)]
struct IgnoreRule {
    /// 匹配条目本身的 glob。
    exact: Vec<GlobMatcher>,
    /// 匹配目录下后代的 glob，例如 `target/**`。
    descendants: Vec<GlobMatcher>,
    negated: bool,
    directory_only: bool,
}
impl IgnoreRule {
    fn matches(&self, path: &str, is_dir: bool) -> bool {
        if self.directory_only && !is_dir {
            return self
                .descendants
                .iter()
                .any(|matcher| matcher.is_match(path));
        }
        self.exact
            .iter()
            .chain(self.descendants.iter())
            .any(|matcher| matcher.is_match(path))
    }
}
/// `.dockerignore` 过滤规则集合。
#[derive(Debug, Default)]
pub struct DockerIgnore {
    rules: Vec<IgnoreRule>,
}
impl DockerIgnore {
    /// 从构建上下文目录解析 `.dockerignore`。
    ///
    /// 语义与 Docker CLI 对齐：支持 `#` 注释、`!` 否定、`/` 根锚定与目录结尾斜杠；
    /// 不再隐式忽略 `.git`，需要排除时由用户的 `.dockerignore` 显式声明。
    pub fn from_context(context_dir: &Path) -> Self {
        let ignore_file = context_dir.join(".dockerignore");
        let mut rules = Vec::new();
        let Ok(content) = std::fs::read_to_string(ignore_file) else {
            return Self { rules };
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (negated, pattern) = match trimmed.strip_prefix('!') {
                Some(pattern) => (true, pattern.trim()),
                None => (false, trimmed),
            };
            if pattern.is_empty() {
                continue;
            }
            if let Some(rule) = compile_ignore_rule(pattern, negated) {
                rules.push(rule);
            } else {
                log::warn!("忽略无效的 .dockerignore 规则: {trimmed}");
            }
        }
        Self { rules }
    }

    /// 判定指定相对路径是否已被忽略；后面的规则优先。
    pub fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        let clean = rel_path
            .trim_start_matches('/')
            .trim_start_matches("./")
            .trim_end_matches('/');
        if clean.is_empty() {
            return false;
        }
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches(clean, is_dir) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}
/// 把一条 Docker 风格 pattern 编译成多个 glob；无斜杠规则同时匹配任意层级。
fn compile_ignore_rule(pattern: &str, negated: bool) -> Option<IgnoreRule> {
    let anchored = pattern.starts_with('/');
    let directory_only = pattern.ends_with('/');
    let clean = pattern
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim_start_matches("./");
    if clean.is_empty() {
        return None;
    }
    let mut exact = Vec::new();
    let mut descendants = Vec::new();
    let add = |candidate: &str, target: &mut Vec<GlobMatcher>| {
        if let Ok(glob) = Glob::new(candidate) {
            target.push(glob.compile_matcher());
        }
    };
    add(clean, &mut exact);
    if !anchored {
        add(&format!("**/{clean}"), &mut exact);
    }
    add(&format!("{clean}/**"), &mut descendants);
    if !anchored {
        add(&format!("**/{clean}/**"), &mut descendants);
    }
    (!exact.is_empty() || !descendants.is_empty()).then_some(IgnoreRule {
        exact,
        descendants,
        negated,
        directory_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_dockerignore_filtering() {
        let temp = tempdir().expect("创建临时目录失败");
        let root = temp.path();
        fs::write(
            root.join(".dockerignore"),
            "target/\n*.tmp\nignore_me.txt\n!keep.tmp\n/root-only.txt\n",
        )
        .expect("写入 dockerignore 失败");

        let ignore = DockerIgnore::from_context(root);
        // Docker 默认不会隐式忽略 .git，需要用户自己写进 .dockerignore。
        assert!(!ignore.is_ignored(".git", true));
        assert!(ignore.is_ignored("target", true));
        assert!(ignore.is_ignored("target/debug/app", false));
        assert!(!ignore.is_ignored("target", false));
        assert!(ignore.is_ignored("test.tmp", false));
        assert!(ignore.is_ignored("nested/test.tmp", false));
        assert!(!ignore.is_ignored("keep.tmp", false));
        assert!(ignore.is_ignored("ignore_me.txt", false));
        assert!(ignore.is_ignored("root-only.txt", false));
        assert!(!ignore.is_ignored("sub/root-only.txt", false));
        assert!(!ignore.is_ignored("src/main.rs", false));
        assert!(!ignore.is_ignored("Dockerfile", false));
    }
}
