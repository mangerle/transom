//! 界面外观设置：明暗模式、UI 主题、界面密度、字体族覆盖与分割线。

use serde::{Deserialize, Serialize};

/// 界面外观的明暗模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiThemeMode {
    /// 跟随系统明暗设置；依赖桌面 portal 上报，极简窗口管理器下可能恒为浅色。
    #[default]
    System,
    /// 固定浅色。
    Light,
    /// 固定深色。
    Dark,
    /// 反序列化兜底：设置文件属外部输入，未知值按跟随系统处理，避免一个非法
    /// 字段让整份设置回退默认。
    #[serde(other)]
    Unknown,
}

impl UiThemeMode {
    /// 归一化兜底值：写盘前洗成合法状态，保证持久化文件永远可回读。
    pub(crate) fn normalized(self) -> Self {
        match self {
            UiThemeMode::Unknown => UiThemeMode::System,
            mode => mode,
        }
    }
}

/// 界面密度档位。
///
/// 为什么只有两个数值：gpui-kit 的根插件每帧把窗口 rem 基准设为全局主题字号，
/// 间距、文字与图标全部随动，因此密度只需定 sans 与 mono 两个字号。mono 与
/// sans 的差距随档位保持在 2 至 3px，标准档即 gpui 默认值（16/13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiDensity {
    /// 紧凑：全局 13px、等宽 11px。
    Compact,
    /// 偏紧：全局 14px、等宽 12px。
    Snug,
    /// 适中：全局 15px、等宽 12px。
    Medium,
    /// 标准：全局 16px、等宽 13px，与 gpui 默认一致。
    #[default]
    Standard,
    /// 宽松：全局 18px、等宽 15px。
    Roomy,
    /// 反序列化兜底：未知值按标准档处理。
    #[serde(other)]
    Unknown,
}

impl UiDensity {
    /// 全部可选档位，供设置页按固定顺序枚举。
    pub const ALL: [UiDensity; 5] = [
        UiDensity::Compact,
        UiDensity::Snug,
        UiDensity::Medium,
        UiDensity::Standard,
        UiDensity::Roomy,
    ];

    /// 界面全局字号（rem 基准），单位 px。
    pub fn font_size(self) -> f32 {
        match self {
            UiDensity::Compact => 13.0,
            UiDensity::Snug => 14.0,
            UiDensity::Medium => 15.0,
            UiDensity::Standard | UiDensity::Unknown => 16.0,
            UiDensity::Roomy => 18.0,
        }
    }

    /// 界面等宽字号（SFTP 预览、指纹展示等场景），单位 px。
    pub fn mono_font_size(self) -> f32 {
        match self {
            UiDensity::Compact => 11.0,
            UiDensity::Snug | UiDensity::Medium => 12.0,
            UiDensity::Standard | UiDensity::Unknown => 13.0,
            UiDensity::Roomy => 15.0,
        }
    }

    /// 设置页展示用的本地化键。
    pub fn label_key(self) -> &'static str {
        match self {
            UiDensity::Compact => "settings.density_compact",
            UiDensity::Snug => "settings.density_snug",
            UiDensity::Medium => "settings.density_medium",
            UiDensity::Standard | UiDensity::Unknown => "settings.density_standard",
            UiDensity::Roomy => "settings.density_roomy",
        }
    }

    /// 归一化兜底值：写盘前洗成合法状态，保证持久化文件永远可回读。
    pub(crate) fn normalized(self) -> Self {
        match self {
            UiDensity::Unknown => UiDensity::Standard,
            density => density,
        }
    }
}

/// 分割线颜色的语义来源。
///
/// 为什么用语义枚举而不是绝对色值：分割线要跨深浅主题、窗口透明折算与主题
/// 热切换保持一致观感。语义色每帧从当前主题解析，换主题自动跟随；`Theme`
/// 档还额外尊重各区域原有取色（普通边界取 border，侧栏与 SFTP 边界取各自
/// 的专用边界色），因此默认档不改变任何现有观感。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DividerColor {
    /// 跟随主题：各区域沿用自身原有边界色。
    #[default]
    Theme,
    /// 所有分割线统一使用主题边框色。
    Border,
    /// 所有分割线统一使用弱前景色，比边框更醒目。
    Muted,
    /// 所有分割线统一使用前景色，对比最强。
    Foreground,
    /// 所有分割线统一使用主题强调色。
    Primary,
    /// 反序列化兜底：设置文件属外部输入，未知值按跟随主题处理，避免一个
    /// 非法字段让整份设置回退默认。
    #[serde(other)]
    Unknown,
}

impl DividerColor {
    /// 设置页展示用的本地化键。
    pub fn label_key(self) -> &'static str {
        match self {
            DividerColor::Theme => "settings.divider_color_theme",
            DividerColor::Border => "settings.divider_color_border",
            DividerColor::Muted => "settings.divider_color_muted",
            DividerColor::Foreground => "settings.divider_color_foreground",
            DividerColor::Primary => "settings.divider_color_primary",
            DividerColor::Unknown => "settings.divider_color_theme",
        }
    }

    /// 归一化兜底值：写盘前洗成合法状态，保证持久化文件永远可回读。
    pub(crate) fn normalized(self) -> Self {
        match self {
            DividerColor::Unknown => DividerColor::Theme,
            color => color,
        }
    }
}

/// 分割线粗细档位。
///
/// 为什么只开三档而不用自由像素：分割线是低强度结构件，超过 3px 就会抢走
/// 内容区的注意力；三档已覆盖"潜行、标准、明显"三种诉求，也避免静态线因
/// 过宽挤占布局（边框参与排版，每加 1px 都会推动相邻内容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DividerWidth {
    /// 发丝线：1px，与现有观感一致。
    #[default]
    Hairline,
    /// 中等：2px。
    Medium,
    /// 明显：3px。
    Thick,
    /// 反序列化兜底：未知值按发丝线处理。
    #[serde(other)]
    Unknown,
}

impl DividerWidth {
    /// 全部可选档位（不含兜底变体），按像素从细到粗排列。
    ///
    /// 设置页已改用 1/2/3 步进器，不再按此枚举出下拉；保留它给测试钉住
    /// "档位顺序 ↔ 像素宽度"的对应关系。
    pub const ALL: [DividerWidth; 3] = [
        DividerWidth::Hairline,
        DividerWidth::Medium,
        DividerWidth::Thick,
    ];

    /// 分割线占用的像素宽度。
    pub fn pixels(self) -> f32 {
        match self {
            DividerWidth::Hairline | DividerWidth::Unknown => 1.0,
            DividerWidth::Medium => 2.0,
            DividerWidth::Thick => 3.0,
        }
    }

    /// 归一化兜底值：写盘前洗成合法状态，保证持久化文件永远可回读。
    pub(crate) fn normalized(self) -> Self {
        match self {
            DividerWidth::Unknown => DividerWidth::Hairline,
            width => width,
        }
    }
}

/// 分割线外观偏好：颜色语义与粗细。
///
/// 覆盖范围是可拖拽分割条与区域结构性分隔线；列表行分隔、控件内部边框与
/// 拖拽落点指示线有各自的语义，不受这里支配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DividerSettings {
    /// 颜色语义；默认跟随主题。
    pub color: DividerColor,
    /// 粗细档位；默认发丝线。
    pub width: DividerWidth,
}

impl DividerSettings {
    /// 写盘前归一化兜底值，保证持久化状态合法。
    pub fn normalize(&mut self) {
        self.color = self.color.normalized();
        self.width = self.width.normalized();
    }
}

/// 界面外观偏好：只描述外壳皮肤（明暗、主题、密度、分割线），不触碰终端
/// ANSI 配色与会话渲染 profile——那两部分由 `TerminalProfile` 承担，两套
/// 体系互不覆盖。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// 明暗模式；`System` 跟随操作系统。
    pub mode: UiThemeMode,
    /// 用户选择的 UI 主题名；空串表示用内置默认。明暗切换时若该主题的明暗
    /// 属性与生效模式不符，运行期回退到对应明暗的默认主题，但选择值保留，
    /// 切回匹配的明暗后自动恢复用户所选主题。
    pub ui_theme: String,
    /// 界面密度档位（驱动全局字号与 rem 基准）。
    pub density: UiDensity,
    /// 界面字体族覆盖；`None` 时跟随主题自带或系统默认字体。
    ///
    /// 与字号的分工：字号归密度档（整体缩放），字体族是个体偏好——中文用户
    /// 换更纱黑体/思源黑体的诉求只有这一条入口能表达。
    pub ui_font_family: Option<String>,
    /// 分割线颜色与粗细；默认跟随主题、1px，旧设置缺少该字段时零观感差异。
    pub divider: DividerSettings,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            mode: UiThemeMode::System,
            ui_theme: String::new(),
            density: UiDensity::Standard,
            ui_font_family: None,
            divider: DividerSettings::default(),
        }
    }
}

impl AppearanceSettings {
    /// 写盘前归一化兜底值，保证持久化状态合法。
    pub fn normalize(&mut self) {
        self.mode = self.mode.normalized();
        self.density = self.density.normalized();
        self.divider.normalize();
    }
}
