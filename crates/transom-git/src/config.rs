//! Git 子进程运行配置的显式快照。
//!
//! 设计原理（Why）：
//! - 可执行文件与提交作者是每次 git 调用都要用的参数。此前它们放在进程级
//!   静态里，测试之间会互相污染，界面也能绕过控制平面直接改；改为值对象后，
//!   配置沿 [`Repo`](crate::Repo) 与调用参数显式流动，谁在用哪份配置一眼可见；
//! - 本模块只做纯数据与回退判定，不读取环境变量与配置文件：控制平面持有
//!   唯一一份快照，这里负责把空白值折算成可直接执行的口径。
//!
//! 代价与局限：每次派发仓库任务要克隆一份配置（三个短字符串），相对一次
//! 子进程调用可以忽略；换来的是调用期间配置不会被其他写方改掉。

/// Git 子进程的运行配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitConfig {
    /// 可执行文件路径；空白表示使用 `PATH` 中的 `git`。
    pub program: String,
    /// 提交作者姓名；空白表示不注入 `GIT_AUTHOR_NAME` 与 `GIT_COMMITTER_NAME`。
    pub author_name: String,
    /// 提交作者邮箱；空白表示不注入 `GIT_AUTHOR_EMAIL` 与 `GIT_COMMITTER_EMAIL`。
    pub author_email: String,
}

impl GitConfig {
    /// 实际使用的可执行文件名：去空白后为空时回退到 `"git"`。
    ///
    /// 返回借用而不是 `String`：调用点只把它交给 `Command::new`，复制一份没有收益。
    pub fn program(&self) -> &str {
        let program = self.program.trim();
        if program.is_empty() { "git" } else { program }
    }
}

#[cfg(test)]
mod tests {
    use super::GitConfig;

    #[test]
    fn program_falls_back_to_git_when_blank() {
        assert_eq!(GitConfig::default().program(), "git");
        let blank = GitConfig {
            program: "   ".to_string(),
            ..GitConfig::default()
        };
        assert_eq!(blank.program(), "git", "空白配置必须回退到系统 git");
    }

    #[test]
    fn program_keeps_custom_path_and_trims_surrounding_space() {
        let config = GitConfig {
            program: " /usr/bin/git-custom ".to_string(),
            ..GitConfig::default()
        };
        assert_eq!(config.program(), "/usr/bin/git-custom");
    }

    #[test]
    fn default_author_fields_are_empty() {
        let config = GitConfig::default();
        assert!(config.author_name.is_empty());
        assert!(config.author_email.is_empty());
    }
}
