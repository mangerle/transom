//! 设置分组的搜索索引：每组可被搜索命中的文案键。
//!
//! 设计原理（Why）：设置页每个分组整体是一个自绘元素（`SettingItem::render`），
//! 上游 `Settings` 组件的搜索只比对元素项显式声明的关键词——没声明时任何非空
//! 查询都会把整组过滤掉，界面上表现为"搜索什么都搜不到"。组内多数行由本项目
//! 自绘（开关行、主题卡、条目列表），没有可被上游读取的标题或描述，因此把
//! "这一组有哪些文案"固化成显式索引表：页面标题、分组标题与组内各行文案；键与
//! 分组渲染函数里用的 i18n 键同名，改文案时两侧一起改。
//!
//! 核心优势：搜索在任意界面语言下都能命中，因为关键词在渲染时按当前语言翻译，
//! 与用户看到的行标题完全一致；测试会校验每个键都能翻译出来，杜绝键名漂移
//! 变成"能搜索但搜索词是键名"的假命中。
//!
//! 代价与局限：过滤粒度是"分组"而不是"行"，命中组内任意一行时整组显示。要细到
//! 行，需要把每组拆成多个 setting item，改动面覆盖全部设置页，暂不做。

use gpui_kit::*;

/// 设置分组的搜索索引。
///
/// 用枚举而不是字符串键：新增分组时 [`GroupSearch::entry`] 的 `match` 必须补齐
/// 对应臂，漏写关键词会直接编译失败，而不是在运行期悄悄退化成搜索无结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GroupSearch {
    /// 外观页：界面主题组。
    UiTheme,
    /// 外观页：主题文档编辑器组。
    ThemeEditor,
    /// 外观页：界面字体组。
    UiFont,
    /// 外观页：窗口外观组。
    WindowAppearance,
    /// 外观页：分割线组。
    Divider,
    /// 终端页：终端字体组。
    TerminalFont,
    /// 终端页：本地终端默认 Shell 组。
    LocalShell,
    /// 终端页：光标与滚动组。
    Cursor,
    /// 终端页：终端配色组。
    TerminalTheme,
    /// 终端页：关键词高亮组。
    Highlight,
    /// 终端页：触发器组。
    Trigger,
    /// 终端页：语义块组。
    BlockView,
    /// 终端页：自定义按键映射组。
    KeyBindings,
    /// 文件页：内置编辑器组。
    Editor,
    /// 文件页：文件面板组。
    FilePanel,
    /// 文件页：传输组。
    Transfer,
    /// 文件页：ZMODEM 组。
    Zmodem,
    /// 文件页：会话输出审计组。
    Audit,
    /// 文件页：凭据模板组。
    Identities,
    /// 文件页：命令历史组。
    History,
    /// 鼠标与剪贴板页：鼠标组。
    Mouse,
    /// 鼠标与剪贴板页：选择组。
    Selection,
    /// 鼠标与剪贴板页：剪贴板组。
    Clipboard,
    /// 快捷键页（该页只有一个无标题分组，标题键仅用于搜索）。
    Shortcuts,
    /// 窗口与布局页：标签页组。
    Tabs,
    /// 窗口与布局页：窗口与会话组。
    Window,
    /// 窗口与布局页：活动栏组。
    ActivityBar,
    /// 窗口与布局页：状态栏组。
    StatusBar,
    /// 窗口与布局页：启动组。
    Startup,
    /// 通用页：语言组。
    Language,
    /// 通用页：通知与声音组。
    Notify,
    /// 通用页：出站代理组。
    Proxy,
    /// 通用页：桌面集成组。
    #[cfg(target_os = "linux")]
    DesktopIntegration,
    /// 通用页：数据备份组。
    Backup,
    /// 通用页：云同步组。
    Sync,
    /// 通用页：运行日志组。
    Logs,
    /// 监控页：板块默认折叠状态组。
    MonitorSections,
    /// 监控页：抽屉与采样设置组。
    MonitorBehavior,
    /// 关于页：应用信息组。
    About,
    /// 关于页：软件更新组。
    Update,
    /// 容器与镜像源页：镜像仓库源管理组。
    ContainerRegistries,
    /// Git 版本控制页：可执行程序与自动刷新组。
    GitExecutable,
    /// Git 版本控制页：提交与推送安全组。
    GitCommitPush,
    /// Git 版本控制页：差异对比组。
    GitDiff,
}

impl GroupSearch {
    /// 分组标题的文案键。
    ///
    /// 快捷键页的分组在界面上不带标题，这里返回页面标题键，仅用于搜索命中。
    pub(super) fn title_key(self) -> &'static str {
        self.entry().0
    }

    /// 分组所属页面的标题键。
    ///
    /// 也进搜索关键词：用户常按页面名找设置（"监控""外观"），只认分组标题时
    /// 这类查询会一无所获。
    pub(super) fn page_title_key(self) -> &'static str {
        use GroupSearch::*;
        match self {
            UiTheme | ThemeEditor | UiFont | WindowAppearance | Divider => {
                "settings.tab_appearance"
            }
            TerminalFont | LocalShell | Cursor | TerminalTheme | Highlight | Trigger
            | BlockView | KeyBindings => "settings.tab_terminal",
            Editor | FilePanel | Transfer | Zmodem | Audit | Identities | History => {
                "settings.tab_files"
            }
            Mouse | Selection | Clipboard => "settings.tab_mouse_clipboard",
            Shortcuts => "settings.tab_shortcuts",
            Tabs | Window | ActivityBar | StatusBar | Startup => "settings.tab_window_layout",
            Language | Notify | Proxy | Backup | Sync | Logs => "settings.tab_general",
            #[cfg(target_os = "linux")]
            DesktopIntegration => "settings.tab_general",
            MonitorSections | MonitorBehavior => "settings.tab_monitor",
            GitExecutable | GitCommitPush | GitDiff => "settings.tab_git",
            ContainerRegistries => "settings.tab_containers",
            About | Update => "settings.tab_about",
        }
    }

    /// 当前界面语言下的搜索关键词：页面标题、分组标题在前，组内各行文案在后。
    pub(super) fn keywords(self) -> Vec<SharedString> {
        let (title, rows) = self.entry();
        let mut keywords = Vec::with_capacity(rows.len() + 2);
        keywords.push(SharedString::from(
            rust_i18n::t!(self.page_title_key()).to_string(),
        ));
        keywords.push(SharedString::from(rust_i18n::t!(title).to_string()));
        for &row in rows {
            keywords.push(SharedString::from(rust_i18n::t!(row).to_string()));
        }
        keywords
    }
}

/// 分组标题键与组内行文案键的唯一来源。
///
/// 表本身在 [`table`] 模块里维护，单独成文件只为控制单文件长度；方法仍是
/// [`GroupSearch`] 的一部分，归属不因拆分而改变。
mod table;

#[cfg(test)]
mod tests {
    use super::GroupSearch;

    /// 覆盖全部变体：新增分组时若忘了加进这里，测试不会验证它，但 `entry` 的
    /// 穷尽匹配仍会强制补齐关键词，编译期兜底。
    const ALL: &[GroupSearch] = &[
        GroupSearch::UiTheme,
        GroupSearch::ThemeEditor,
        GroupSearch::UiFont,
        GroupSearch::WindowAppearance,
        GroupSearch::Divider,
        GroupSearch::TerminalFont,
        GroupSearch::Cursor,
        GroupSearch::TerminalTheme,
        GroupSearch::Highlight,
        GroupSearch::Trigger,
        GroupSearch::BlockView,
        GroupSearch::KeyBindings,
        GroupSearch::Editor,
        GroupSearch::FilePanel,
        GroupSearch::Transfer,
        GroupSearch::Zmodem,
        GroupSearch::Audit,
        GroupSearch::Identities,
        GroupSearch::History,
        GroupSearch::Mouse,
        GroupSearch::Selection,
        GroupSearch::Clipboard,
        GroupSearch::Shortcuts,
        GroupSearch::Tabs,
        GroupSearch::Window,
        GroupSearch::ActivityBar,
        GroupSearch::StatusBar,
        GroupSearch::Startup,
        GroupSearch::Language,
        GroupSearch::Notify,
        GroupSearch::Proxy,
        #[cfg(target_os = "linux")]
        GroupSearch::DesktopIntegration,
        GroupSearch::Backup,
        GroupSearch::Sync,
        GroupSearch::Logs,
        GroupSearch::MonitorSections,
        GroupSearch::MonitorBehavior,
        GroupSearch::About,
        GroupSearch::Update,
        GroupSearch::ContainerRegistries,
        GroupSearch::GitExecutable,
        GroupSearch::GitCommitPush,
        GroupSearch::GitDiff,
    ];

    /// 翻译结果仍是原键名，说明资源里没有这条文案（rust-i18n 缺键时原样返回键）。
    fn translated(key: &str) -> String {
        rust_i18n::t!(key).to_string()
    }

    #[test]
    fn every_group_keyword_has_translation() {
        for group in ALL {
            let (title, rows) = group.entry();
            let page_title = group.page_title_key();
            assert_ne!(
                translated(page_title),
                page_title,
                "页面标题键缺少文案：{page_title}（分组 {group:?}）"
            );
            assert_ne!(
                translated(title),
                title,
                "分组标题键缺少文案：{title}（分组 {group:?}）"
            );
            assert!(!rows.is_empty(), "分组 {group:?} 没有可搜索的行文案键");
            for row in rows {
                assert_ne!(
                    translated(row),
                    *row,
                    "行文案键缺少文案：{row}（分组 {group:?}）"
                );
            }
        }
    }

    #[test]
    fn keywords_lead_with_page_and_group_titles() {
        for group in ALL {
            let keywords = group.keywords();
            assert_eq!(keywords.len(), group.entry().1.len() + 2);
            assert_eq!(
                keywords[0],
                translated(group.page_title_key()),
                "关键词首项必须是页面标题（分组 {group:?}）"
            );
            assert_eq!(
                keywords[1],
                translated(group.title_key()),
                "关键词次项必须是分组标题（分组 {group:?}）"
            );
            for keyword in &keywords {
                assert!(
                    !keyword.trim().is_empty(),
                    "关键词不得为空（分组 {group:?}）"
                );
            }
        }
    }
}
