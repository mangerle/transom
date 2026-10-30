//! 设置中心的字体族行：可搜索的字体下拉与本地字体导入。
//!
//! 字体族是封闭取值，只允许从系统已安装的字体里选，因此这里直接复用
//! gpui-kit 的 `Select`（只读触发器 + 弹层内搜索），不在应用层重写
//! 焦点、键盘或弹层逻辑；导入只负责把新字体注册进文本系统并刷新选项。

use std::borrow::Cow;
use std::collections::HashSet;

use super::font_select::{
    CJK_DEFAULT_KEY, FontSelect, MONO_DEFAULT_KEY, UI_DEFAULT_KEY, reset_font_select,
};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::select::Select;
use gpui_kit::*;

/// 字体族下拉的宽度（rem，标准档 256px，与组件库 `w_64` 文本字段同宽）。
///
/// 字体族名通常只有十几个字符，按整行铺开会留下大片空白，导入按钮也被推得
/// 离下拉很远；定宽后整个控件簇靠右收敛，与全页"左标题、右控件"的行式
/// 布局对齐。
const FONT_FIELD_WIDTH: f32 = 16.0;

/// 字体族设置行的装配参数。
///
/// 收成一个结构体是因为字段已超过五个：再接一个独立入参，调用点就会出现一长串
/// 位置参数，把"说明挂在哪个下拉下"读错。
pub(crate) struct FontFieldSpec {
    pub id: &'static str,
    pub label: String,
    pub select: Entity<FontSelect>,
    /// 标题下方的补充说明（文案键）；与下拉同属一行，一起画分割线。
    pub note_key: Option<&'static str>,
}

impl AppShell {
    /// 字体配置行：左侧标题与说明，右侧是字体族下拉 + 本地字体导入按钮。
    ///
    /// 下拉的选项来自文本系统的已安装字体清单，弹层里可输入搜索；默认项
    /// 置顶，表示跟随主题/系统回退。导入按钮把字体文件注册进文本系统后
    /// 刷新三个字体字段的选项，并选中新注册的族名。
    pub(crate) fn setting_font_field(
        spec: FontFieldSpec,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let FontFieldSpec {
            id,
            label,
            select,
            note_key,
        } = spec;
        let control = h_flex()
            .gap_1p5()
            .items_center()
            .child(
                div()
                    .w(rems(FONT_FIELD_WIDTH))
                    .child(Select::new(&select).w_full()),
            )
            .child(
                Button::new(format!("{id}-import"))
                    .outline()
                    .small()
                    .icon(IconName::FolderOpen)
                    .tooltip(rust_i18n::t!("settings.import_font_tooltip").to_string())
                    .on_click(cx.listener({
                        let select = select.clone();
                        move |this, _, window, cx| {
                            this.import_local_font_file(&select, window, cx);
                        }
                    })),
            )
            .into_any_element();
        let note = note_key.map(|key| rust_i18n::t!(key).to_string());
        Self::setting_text_row(label, note, control, p)
    }

    /// 唤起系统文件选择器导入外部字体文件（.ttf, .otf, .ttc 等），注册后选中新族名。
    ///
    /// 字体文件里的族名不一定等于文件名（如 `SourceHanSansSC-Regular.otf`
    /// 的族名是 `Source Han Sans SC`），所以用导入前后 `all_font_names()`
    /// 的差集取新注册的族名，而不是把文件 stem 写进设置。
    pub(crate) fn import_local_font_file(
        &mut self,
        select: &Entity<FontSelect>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("settings.pick_font_title").to_string())
            .add_filter(
                rust_i18n::t!("settings.font_file_filter").to_string(),
                &["ttf", "otf", "ttc", "woff2"],
            )
            .pick_file();

        let Some(path) = picked else {
            return;
        };

        match std::fs::read(&path) {
            Ok(bytes) => {
                let before: HashSet<String> =
                    cx.text_system().all_font_names().into_iter().collect();
                if let Err(err) = cx.text_system().add_fonts(vec![Cow::Owned(bytes)]) {
                    log::warn!("加载外部字体文件失败，路径: {:?}，原因: {err}", path);
                    self.status =
                        Some(rust_i18n::t!("settings.font_load_failed", error = err).to_string());
                    cx.notify();
                    return;
                }

                let family = cx
                    .text_system()
                    .all_font_names()
                    .into_iter()
                    .find(|name| !before.contains(name));

                let Some(family) = family else {
                    log::warn!("字体文件未注册新的字体族，路径: {:?}", path);
                    self.status =
                        Some(rust_i18n::t!("settings.font_import_no_new_family").to_string());
                    cx.notify();
                    return;
                };

                self.refresh_font_selects(window, cx);
                select.update(cx, |state, cx| {
                    state.set_selected_value(&family, window, cx);
                });
                log::info!("成功注册本地字体: {}，路径: {:?}", family, path);
                self.status = Some(
                    rust_i18n::t!("settings.font_imported_success", name = family).to_string(),
                );
                cx.notify();
            }
            Err(err) => {
                log::error!("读取本地字体文件失败，路径: {:?}，原因: {err}", path);
                self.status =
                    Some(rust_i18n::t!("settings.font_read_failed", error = err).to_string());
                cx.notify();
            }
        }
    }

    /// 重建三个字体字段的选项：系统字体集合是全局的，导入后三个下拉都要看到新族名。
    fn refresh_font_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fields = [
            (self.modals.settings.font_family.clone(), MONO_DEFAULT_KEY),
            (self.modals.settings.cjk_font.clone(), CJK_DEFAULT_KEY),
            (self.modals.settings.ui_font_family.clone(), UI_DEFAULT_KEY),
        ];

        for (select, key) in fields {
            let current = select.read(cx).selected_value().cloned();
            let default_label = rust_i18n::t!(key).to_string();
            reset_font_select(&select, &default_label, current.as_deref(), window, cx);
        }
    }
}
