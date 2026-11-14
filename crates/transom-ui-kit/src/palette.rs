//! 主题调色板、通用弹窗容器与错误文本本地化辅助工具。

use gpui_kit::base::{ColorTokens, RadiusTokens};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use transom_core::{DividerWidth};

/// 应用级表面语义令牌。
///
/// 为什么集中定义：页面直接对基础颜色乘透明度会让同一视觉层级散落在不同文件里，
/// 主题无法分别调整弱表面、悬浮、按压和选中态。这里把高频表面层级收敛为
/// 稳定语义，页面只表达“这是什么状态”，不再重复决定透明度数值。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTokens {
    /// 最弱的表面提亮，用于卡片内部的细微分区。
    pub ghost: Hsla,
    /// 弱表面，用于次级卡片、预览块和轻量分组。
    pub faint: Hsla,
    /// 次级表面，用于强调底色、标签底和弱选中区域。
    pub subtle: Hsla,
    /// 指针悬浮时的控件表面。
    pub hover: Hsla,
    /// 已激活但仍可继续交互的表面，强于 hover、弱于 pressed。
    pub active: Hsla,
    /// 按压或拖拽中的控件表面。
    pub pressed: Hsla,
    /// 列表当前项与选中项表面。
    pub selected: Hsla,
    /// 控件与切换开关的选中态底色（中性深灰，区分亮色与暗色主题）。
    pub control_selected: Hsla,
    /// 浮层与弹窗表面。
    pub floating: Hsla,
    /// 以文字色叠加的轻量强调表面，用于极弱分割与交互反馈。
    pub contrast: Hsla,
}

/// 应用级边界语义令牌。
///
/// 边界强度必须与表面层级成套使用；页面只选择语义强度，避免同一个
/// 0.3 透明度在不同主题和不同背景下失去可辨识度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderTokens {
    /// 最弱边界，用于内部分割与辅助轮廓。
    pub faint: Hsla,
    /// 弱边界，用于卡片、预览和轻量容器。
    pub subtle: Hsla,
    /// 中等边界，用于普通分组与交互区域。
    pub muted: Hsla,
    /// 较强边界，用于需要明确分隔的结构。
    pub medium: Hsla,
    /// 强边界，用于强调区域与临时编辑边界。
    pub strong: Hsla,
}

/// 应用级语义色调令牌。
///
/// 这些底色来自主题的语义色，但不再由调用点临时选择透明度，因此主题切换时
/// 危险、警告、成功和主强调的弱背景仍保持一致层级。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneTokens {
    /// 主强调色的弱背景。
    pub primary_subtle: Hsla,
    /// 危险语义的弱背景。
    pub danger_subtle: Hsla,
    /// 警告语义的弱背景。
    pub warning_subtle: Hsla,
    /// 成功语义的弱背景。
    pub success_subtle: Hsla,
}

/// 应用级文字语义令牌。
///
/// 基础文字色由主题提供；这里只定义在弱化信息中反复出现的层级，避免页面用
/// 任意透明度表达禁用、辅助和次级文字。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextTokens {
    /// 次级辅助文字，比 muted_foreground 更弱一档。
    pub subtle: Hsla,
    /// 弱说明文字与低优先级标签。
    pub muted: Hsla,
    /// 禁用状态的文字与图标。
    pub disabled: Hsla,
}

impl SurfaceTokens {
    /// 从主题语义色派生表面层级。
    ///
    /// 透明度只允许在本模块集中定义：主题只需提供 muted、foreground、surface
    /// 等基础色，应用级界面即可获得一致的弱表面和交互表面。
    fn from_theme(colors: &ColorTokens, selected: Hsla) -> Self {
        let is_dark = colors.background.l < 0.5;
        let control_selected = if is_dark {
            colors.foreground.opacity(0.24)
        } else {
            colors.foreground.opacity(0.20)
        };
        Self {
            ghost: colors.muted.opacity(0.04),
            faint: colors.muted.opacity(0.08),
            subtle: colors.muted.opacity(0.12),
            hover: colors.muted.opacity(0.30),
            active: colors.muted.opacity(0.35),
            pressed: colors.muted.opacity(0.40),
            selected,
            control_selected,
            floating: colors.surface,
            contrast: colors.foreground.opacity(0.08),
        }
    }
}

impl BorderTokens {
    /// 从主题边界色派生边界强度层级。
    fn from_color(border: Hsla) -> Self {
        Self {
            faint: border.opacity(0.12),
            subtle: border.opacity(0.15),
            muted: border.opacity(0.30),
            medium: border.opacity(0.40),
            strong: border.opacity(0.50),
        }
    }
}

impl ToneTokens {
    /// 从主题语义色派生弱色背景。
    fn from_theme(colors: &ColorTokens, warning: Hsla, success: Hsla) -> Self {
        Self {
            primary_subtle: colors.primary.opacity(0.12),
            danger_subtle: colors.destructive.opacity(0.12),
            warning_subtle: warning.opacity(0.12),
            success_subtle: success.opacity(0.12),
        }
    }
}

impl TextTokens {
    /// 从主题次级文字色派生文字层级。
    fn from_color(muted_foreground: Hsla) -> Self {
        Self {
            subtle: muted_foreground.opacity(0.80),
            muted: muted_foreground.opacity(0.70),
            disabled: muted_foreground.opacity(0.40),
        }
    }
}

/// 全局 UI 调色板快照，用于各子视图渲染统一配色。
///
/// 为什么是值快照而不是直接读 `cx.theme()`：渲染函数签名只依赖 `&Palette`，
/// 不与 `cx` 借用纠缠；同时每帧从全局主题重建，主题热切换后下一帧自动生效。
/// 含 `SharedString` 字段因此只能 Clone，各渲染闭包持有 `Palette` 值时用
/// `p.clone()` 传递。
#[derive(Clone)]
pub struct Palette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    /// 表面层级语义令牌。
    pub surface: SurfaceTokens,
    /// 边界层级语义令牌。
    pub stroke: BorderTokens,
    /// 语义色调令牌。
    pub tone: ToneTokens,
    /// 文字层级语义令牌。
    pub text: TextTokens,
    pub sidebar: Hsla,
    pub sidebar_foreground: Hsla,
    pub sidebar_border: Hsla,
    pub sidebar_accent: Hsla,
    pub sidebar_accent_foreground: Hsla,
    /// 列表当前项的强调边框色，用于把"正在使用的会话"与普通悬浮态区分开。
    pub list_active_border: Hsla,
    pub tab_bar: Hsla,
    pub tab_active: Hsla,
    pub tab_foreground: Hsla,
    pub tab_active_foreground: Hsla,
    pub title_bar: Hsla,
    pub status_bar: Hsla,
    /// 次级表面背景色（徽标胶囊等）。
    pub secondary: Hsla,
    pub secondary_foreground: Hsla,
    pub secondary_hover: Hsla,
    /// 列表偶数行斑马纹背景色。
    pub list_even: Hsla,
    /// 列表行常态表面色（会话树行底）。
    pub list_background: Hsla,
    /// 列表表头表面色（会话树文件夹行底）。
    pub list_head: Hsla,
    /// 列表悬浮表面色。
    pub list_hover: Hsla,
    /// 列表激活表面色（选中与当前会话共用）。
    pub list_active: Hsla,
    /// 拖放目标提示底色。
    pub drop_target: Hsla,
    /// 主题强调色（设置页主题卡预览色点等场景）。
    pub primary: Hsla,
    /// 警示语义色（设置页明暗反差提示等场景）。
    pub warning: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    /// 模态遮罩底色：自带透明度，随主题明暗变化。
    pub overlay: Hsla,
    pub popover: Hsla,
    pub popover_foreground: Hsla,
    /// 编辑器画布底色：取自高亮主题的 `editor.background`，缺省回退到界面背景。
    ///
    /// 编辑面板的标题栏/状态栏用 `popover` 做外框，正文用这个色做画布，两者
    /// 只差一档明度，用户看到的是"编辑器 + 一点外框"，而不是"弹窗里嵌输入框"。
    pub editor_background: Hsla,
    /// 界面等宽字体族（SFTP 预览、指纹展示等逐字符对齐场景）。
    pub mono_font_family: SharedString,
    /// 界面等宽字号，随密度档位缩放。
    pub mono_font_size: Pixels,
    /// 圆角档位（由主题的 radius/radius_lg 派生），容器级圆角统一从这里取值，
    /// 主题即可控制全局圆角；装饰性圆点仍用 rounded_full。
    ///
    /// 浮层容器（对话框、浮窗、命令面板等）固定取 `none` 保持直角：这些面板
    /// 尺寸大且贴近底窗边缘，圆角会在四角切出底色缺口，故不随主题变化。
    pub radius: RadiusTokens,
    /// 分割线覆盖色；`None` 表示各区域沿用自身原有的边界色。
    pub divider_color: Option<Hsla>,
    /// 分割线粗细档位（静态线用边框实现，档位决定边框像素宽度）。
    pub divider_width: DividerWidth,
}

impl Palette {
    pub fn from(cx: &App) -> Self {
        let theme = cx.theme();
        let colors = theme.semantic_tokens().colors;
        let divider = crate::divider::settings(cx);
        let surface = SurfaceTokens::from_theme(&colors, theme.colors.list_active);
        let list_active = surface.control_selected;
        // 主题里已经按窗口透明度折算过表面色的 alpha，这里直接取用：
        // 自绘界面与 gpui-kit 组件必须共用同一份透明度，否则会出现割裂。
        Self {
            background: theme.background,
            foreground: theme.foreground,
            border: theme.border,
            muted: theme.muted,
            muted_foreground: theme.muted_foreground,
            surface,
            stroke: BorderTokens::from_color(colors.border),
            tone: ToneTokens::from_theme(&colors, theme.warning, theme.success),
            text: TextTokens::from_color(colors.muted_foreground),
            sidebar: theme.sidebar,
            sidebar_foreground: theme.sidebar_foreground,
            sidebar_border: theme.sidebar_border,
            sidebar_accent: theme.sidebar_accent,
            sidebar_accent_foreground: theme.sidebar_accent_foreground,
            list_active_border: theme.list_active_border,
            tab_bar: theme.tab_bar,
            tab_active: theme.tab_active,
            tab_foreground: theme.tab_foreground,
            tab_active_foreground: theme.tab_active_foreground,
            title_bar: theme.title_bar,
            status_bar: theme.status_bar,
            secondary: theme.secondary,
            secondary_foreground: theme.secondary_foreground,
            secondary_hover: theme.secondary_hover,
            list_even: theme.list_even,
            list_background: theme.colors.list,
            list_head: theme.colors.list_head,
            list_hover: theme.colors.list_hover,
            list_active,
            drop_target: theme.drop_target,
            primary: theme.primary,
            warning: theme.warning,
            danger: theme.danger,
            success: theme.success,
            overlay: theme.overlay,
            popover: theme.popover,
            popover_foreground: theme.popover_foreground,
            editor_background: theme
                .highlight_theme
                .style
                .editor_background
                .unwrap_or(theme.background),
            mono_font_family: theme.mono_font_family.clone(),
            mono_font_size: theme.mono_font_size,
            radius: theme.radius_tokens(),
            divider_color: crate::divider::resolve_color(cx, divider.color),
            divider_width: divider.width,
        }
    }

    /// 分割线颜色：设置了覆盖色时统一使用，否则回落到调用点的原有边界色
    /// （普通面板用 `border`，侧栏与 SFTP 面板用各自的专用边界色）。
    pub fn divider(&self, fallback: Hsla) -> Hsla {
        self.divider_color.unwrap_or(fallback)
    }

    /// 分割线像素厚度：由用户设置的分割线粗细档位决定。
    pub fn divider_thickness(&self) -> Pixels {
        px(self.divider_width.pixels())
    }

    /// 下划线激活标签底部强调线像素厚度：不低于 2px。
    pub fn divider_active_line_thickness(&self) -> Pixels {
        px(self.divider_width.pixels().max(2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::{BorderTokens, SurfaceTokens, TextTokens, ToneTokens};
    use gpui_kit::component::theme::Theme;

    #[test]
    fn semantic_tokens_derive_from_active_theme_colors() {
        let theme = Theme::default();
        let colors = theme.semantic_tokens().colors;

        let surface = SurfaceTokens::from_theme(&colors, theme.colors.list_active);
        assert_eq!(surface.ghost, colors.muted.opacity(0.04));
        assert_eq!(surface.faint, colors.muted.opacity(0.08));
        assert_eq!(surface.subtle, colors.muted.opacity(0.12));
        assert_eq!(surface.hover, colors.muted.opacity(0.30));
        assert_eq!(surface.active, colors.muted.opacity(0.35));
        assert_eq!(surface.pressed, colors.muted.opacity(0.40));
        assert_eq!(surface.selected, theme.colors.list_active);
        assert_eq!(surface.floating, colors.surface);
        assert_eq!(surface.contrast, colors.foreground.opacity(0.08));

        let stroke = BorderTokens::from_color(colors.border);
        assert_eq!(stroke.faint, colors.border.opacity(0.12));
        assert_eq!(stroke.subtle, colors.border.opacity(0.15));
        assert_eq!(stroke.muted, colors.border.opacity(0.30));
        assert_eq!(stroke.medium, colors.border.opacity(0.40));
        assert_eq!(stroke.strong, colors.border.opacity(0.50));

        let tone = ToneTokens::from_theme(&colors, theme.warning, theme.success);
        assert_eq!(tone.primary_subtle, colors.primary.opacity(0.12));
        assert_eq!(tone.danger_subtle, colors.destructive.opacity(0.12));
        assert_eq!(tone.warning_subtle, theme.warning.opacity(0.12));
        assert_eq!(tone.success_subtle, theme.success.opacity(0.12));

        let text = TextTokens::from_color(colors.muted_foreground);
        assert_eq!(text.subtle, colors.muted_foreground.opacity(0.80));
        assert_eq!(text.muted, colors.muted_foreground.opacity(0.70));
        assert_eq!(text.disabled, colors.muted_foreground.opacity(0.40));
    }
}

