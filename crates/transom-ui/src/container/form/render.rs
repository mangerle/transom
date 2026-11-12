//! 容器引擎连接表单弹窗主体：基础信息卡、分组选择与底部操作区。

use super::*;
use crate::widgets::{
    DialogConfirmKind, action_button, dialog_cancel_button, dialog_confirm_button,
};
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

/// 渲染时由外壳注入的引擎分组快照。
///
/// 实体不反查目录：打开后目录变化不应悄悄改变表单的候选列表，外壳在渲染
/// 时一次性读入，保持“数据向下、意图向上”的单向依赖。
pub(crate) struct ContainerFormFolders {
    /// 分组 id 与显示名，顺序与目录一致。
    pub(crate) options: Vec<(FolderId, String)>,
}

impl ContainerFormView {
    /// 弹窗主体：基本信息、外观、连接类型与按档位展开的字段。
    pub(crate) fn render_body(
        &self,
        folders: &ContainerFormFolders,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(form) = self.state() else {
            return div().into_any_element();
        };
        let title = if form.editing.is_some() {
            rust_i18n::t!("container.form.edit_title").to_string()
        } else {
            rust_i18n::t!("container.form.new_title").to_string()
        };

        // 基本信息卡：名称 + 分组（同排）
        let basic_card = self.render_basic_card(form, folders, p, cx);
        // 外观卡：图标 + 环境色
        let appearance_card = self.render_appearance_card(form, p, cx);
        // 连接类型卡：引擎种类 + 端点分组
        let connection_card = self.render_connection_card(form, p, cx);

        let mut content = v_flex()
            .id(ElementId::Name("engine-form-body".into()))
            .max_h(px(520.0))
            .overflow_y_scroll()
            .gap_3()
            .child(basic_card)
            .child(appearance_card)
            .child(connection_card);

        // 动态扩展字段：根据端点档位追加
        if let Some(extra) = self.render_extra_fields(form, p, cx) {
            content = content.child(extra);
        }

        // 测试连接状态提示
        if let Some(status) = &form.test_status {
            content = content.child(
                div()
                    .px_1()
                    .text_sm()
                    .text_color(if form.test_ok { p.primary } else { p.danger })
                    .child(status.clone()),
            );
        }

        let footer = self.render_footer(form, p, cx);

        crate::widgets::dialog_body("container-form-body", title, None, content, Some(footer))
    }

    /// 基本信息卡：名称输入框与分组下拉同排。
    fn render_basic_card(
        &self,
        form: &ContainerFormState,
        folders: &ContainerFormFolders,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_3()
            .child(div().flex_1().child(crate::widgets::form_field(
                rust_i18n::t!("container.form.name").to_string(),
                form.name.clone(),
                p,
            )))
            .child(
                div()
                    .flex_1()
                    .child(self.render_folder_field(form, folders, p, cx)),
            )
            .into_any_element()
    }

    /// 分组下拉：根层级固定置顶，其余来自渲染时注入的目录快照。
    ///
    /// 菜单项的点击回调拿不到 `Context<Self>`（下拉菜单只给 `&mut App`），
    /// 因此捕获实体句柄后经 `update` 回到实体。
    fn render_folder_field(
        &self,
        form: &ContainerFormState,
        folders: &ContainerFormFolders,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = form.folder_id;
        let label = selected
            .and_then(|id| {
                folders
                    .options
                    .iter()
                    .find(|(folder_id, _)| *folder_id == id)
                    .map(|(_, name)| name.clone())
            })
            .unwrap_or_else(|| rust_i18n::t!("folder.root_level").to_string());
        let options = folders.options.clone();
        let entity = cx.entity();

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.group").to_string()),
            )
            .child(
                action_button("container-form-folder", UiIcon::FOLDER, label).dropdown_menu(
                    move |menu, _, _| {
                        let mut menu = menu;
                        let root_entity = entity.clone();
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!("folder.root_level").to_string())
                                .checked(selected.is_none())
                                .on_click(move |_, _, cx| {
                                    root_entity.update(cx, |this, cx| this.set_folder(None, cx));
                                }),
                        );
                        for (folder_id, name) in &options {
                            let folder_id = *folder_id;
                            let item_entity = entity.clone();
                            menu = menu.item(
                                PopupMenuItem::new(name.clone())
                                    .checked(selected == Some(folder_id))
                                    .on_click(move |_, _, cx| {
                                        item_entity.update(cx, |this, cx| {
                                            this.set_folder(Some(folder_id), cx);
                                        });
                                    }),
                            );
                        }
                        menu
                    },
                ),
            )
            .into_any_element()
    }

    /// 页脚：删除（仅编辑态）、测试连接、取消与保存，错误提示在按钮上方。
    fn render_footer(
        &self,
        form: &ContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editing = form.editing.is_some();
        let mut footer = h_flex().gap_2().items_center().justify_end().w_full();

        if editing {
            footer = footer.child(
                action_button(
                    "container-form-delete",
                    UiIcon::DELETE,
                    rust_i18n::t!("common.delete").to_string(),
                )
                .danger()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.delete(cx))),
            );
        }

        footer = footer
            .child(
                action_button(
                    "container-form-test",
                    CustomIcon::CONNECTION_TEST,
                    rust_i18n::t!("container.form.test").to_string(),
                )
                .ghost()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.test(cx))),
            )
            .child(dialog_cancel_button(
                "container-form-cancel",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)),
            ))
            .child(dialog_confirm_button(
                "container-form-save",
                rust_i18n::t!("common.save").to_string(),
                Some(AppIcon::Ui(UiIcon::CHECK)),
                DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, _, cx| this.save(cx)),
            ));

        let mut footer_block = v_flex().gap_2().items_end();
        if let Some(error) = &form.test_status
            && !form.test_ok
        {
            footer_block =
                footer_block.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer_block = footer_block.child(footer);
        footer_block.into_any_element()
    }
}
