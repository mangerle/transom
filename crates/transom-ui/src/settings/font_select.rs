//! 字体族选择器的数据装配：已安装族名、默认项与未安装存量项。
//!
//! 设置里的字体是封闭取值：用户只能从系统已安装的字体族里选，不能手输
//! 自由文本。gpui-kit 的 `Select` 正好覆盖这个模型（只读触发器 + 弹层内
//! 搜索），所以这里只负责把 `TextSystem` 的字体清单包装成可过滤的选项，
//! 不在应用层重写焦点、键盘或弹层逻辑。

use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::SelectState;
use gpui_kit::{App, AppContext, Context, Entity, SharedString, Window};

/// 三个字体字段各自的默认项文案键。
pub(crate) const MONO_DEFAULT_KEY: &str = "settings.font_default_mono_label";
pub(crate) const CJK_DEFAULT_KEY: &str = "settings.font_preset_system_fallback";
pub(crate) const UI_DEFAULT_KEY: &str = "settings.ui_font_family_hint";

/// 字体选择器的状态类型别名，避免在字段和调用点重复展开泛型。
pub(crate) type FontSelect = SelectState<SearchableVec<FontChoice>>;

/// 字体下拉里的一项。
///
/// `value` 是写进设置的族名；空串表示“跟随默认”。`title` 只负责显示：
/// 未安装的存量值会带上标记，但仍保持可选中，保存时原样写回，避免换机器
/// 或卸载字体后静默清空用户的配置。
#[derive(Clone)]
pub(crate) struct FontChoice {
    value: String,
    title: SharedString,
}

impl SearchableListItem for FontChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

/// 构造字体选择器：默认项置顶，其余是当前系统已安装的字体族。
///
/// 存量值不在已安装清单里时（换机器、卸载字体），额外补一条带“未安装”
/// 标记的选项，保证它可见、可继续保存。
pub(crate) fn new_font_select<P: 'static>(
    default_label: &str,
    current: Option<&str>,
    window: &mut Window,
    cx: &mut Context<P>,
) -> Entity<FontSelect> {
    let current = normalized(current);
    let items = font_choices(default_label, current, cx);
    let selected = current.unwrap_or_default().to_string();

    cx.new(|cx| {
        let mut state =
            SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true);
        state.set_selected_value(&selected, window, cx);
        state
    })
}

/// 用最新的系统字体清单重建选择器，并保持/恢复指定值。
///
/// 导入字体后系统字体集合会变化，三个字体字段的清单都要重建；传入的值
/// 不在新清单里时会作为“未安装”项保留下来。
pub(crate) fn reset_font_select(
    select: &Entity<FontSelect>,
    default_label: &str,
    value: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) {
    let value = normalized(value);
    let items = font_choices(default_label, value, cx);
    let selected = value.unwrap_or_default().to_string();

    select.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(items), window, cx);
        state.set_selected_value(&selected, window, cx);
    });
}

/// 读取当前选中的字体族；默认项（空串）折回 `None`。
pub(crate) fn selected_font(select: &Entity<FontSelect>, cx: &App) -> Option<String> {
    let value = select.read(cx).selected_value()?;
    (!value.is_empty()).then(|| value.clone())
}

/// 配置里的族名在边界处去掉首尾空白；空串一律视为“跟随默认”。
fn normalized(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn font_choices(default_label: &str, current: Option<&str>, cx: &App) -> Vec<FontChoice> {
    let installed = cx.text_system().all_font_names();
    choices_from_installed(installed, default_label, current)
}

/// 纯装配逻辑，便于覆盖“默认项 / 已安装 / 未安装存量值”三种情况。
fn choices_from_installed(
    installed: Vec<String>,
    default_label: &str,
    current: Option<&str>,
) -> Vec<FontChoice> {
    let current = normalized(current);
    let mut items = Vec::with_capacity(installed.len() + 2);
    items.push(FontChoice {
        value: String::new(),
        title: SharedString::from(default_label.to_string()),
    });

    let mut current_installed = false;
    for name in installed {
        if current == Some(name.as_str()) {
            current_installed = true;
        }
        items.push(FontChoice {
            value: name.clone(),
            title: SharedString::from(name),
        });
    }

    if let Some(name) = current.filter(|name| !name.is_empty() && !current_installed) {
        items.push(FontChoice {
            value: name.to_string(),
            title: SharedString::from(format!(
                "{name}{}",
                rust_i18n::t!("settings.font_missing_suffix")
            )),
        });
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> Vec<String> {
        vec!["Alpha".to_string(), "Beta".to_string()]
    }

    #[test]
    fn default_item_is_first_and_empty() {
        let items = choices_from_installed(installed(), "默认", None);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].value, "");
        assert_eq!(items[0].title.to_string(), "默认");
    }

    #[test]
    fn installed_value_does_not_add_a_duplicate() {
        let items = choices_from_installed(installed(), "默认", Some(" Beta "));
        assert_eq!(items.len(), 3);
        assert!(items.iter().any(|item| item.value == "Beta"));
    }

    #[test]
    fn missing_value_stays_selectable_with_a_marker() {
        let items = choices_from_installed(installed(), "默认", Some("Gone"));
        assert_eq!(items.len(), 4);
        let missing = items.last().expect("未安装项必须存在");
        assert_eq!(missing.value, "Gone");
        assert_ne!(missing.title.to_string(), "Gone");
    }

    #[test]
    fn empty_value_is_treated_as_default() {
        let items = choices_from_installed(installed(), "默认", Some("   "));
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].value, "");
    }
}
