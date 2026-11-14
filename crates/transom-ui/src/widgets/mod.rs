//! 应用级表现层组件重导出：无状态基础组件已剥离至 `transom-ui-kit`。

pub(crate) use transom_ui_kit::widgets::*;

use crate::palette::Palette;
use gpui_kit::AnyElement;

/// Zed 风格设置行（按国际化键翻译标题与说明）。
pub(crate) fn setting_row(
    title_key: &'static str,
    description_key: Option<&'static str>,
    control: AnyElement,
    p: &Palette,
) -> AnyElement {
    transom_ui_kit::widgets::setting_text_row(
        rust_i18n::t!(title_key).to_string(),
        description_key.map(|key| rust_i18n::t!(key).to_string()),
        control,
        p,
    )
}
