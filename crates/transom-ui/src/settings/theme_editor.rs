//! 设置中心的主题文档编辑器。
//!
//! 编辑器直接维护完整的 `ThemeSet` JSON，而不是复制 nyaterm 的逐色表单：
//! Transom 的 gpui-kit token 数量多，逐字段表单会形成一张巨大的镜像结构；
//! 单文档编辑可以覆盖 UI、语法高亮与终端伴生色板，同时保留未知扩展字段。
//! 文本修改后立即解析并预览，解析失败时保留上一份有效画面并显示具体错误。

use super::SettingsEvent;
use crate::app::{app_settings, spawn_blocking_local, update_app_settings};
use crate::palette::Palette;
use crate::settings::SettingsView;
use crate::theme::{
    MAX_THEME_DOCUMENT_BYTES, ThemeDraft, apply_theme_preview, parse_single_theme_document,
    register_theme_document, serialize_theme_document, theme_file_stem, unique_theme_name,
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::theme::ThemeRegistry;
use gpui_kit::{Context, Entity, IntoElement, Task, Window, div, prelude::*, px};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct ThemeEditorStatus {
    pub(crate) message: String,
    pub(crate) is_error: bool,
}

/// 主题编辑器的生命周期随设置实体；任务句柄保留到下一次操作，避免脱缰任务。
pub(crate) struct ThemeEditorState {
    pub(crate) json: Entity<TextareaState>,
    dirty: bool,
    status: Option<ThemeEditorStatus>,
    tasks: Vec<Task<()>>,
}

impl ThemeEditorState {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        Self {
            json: cx.new(|cx| TextareaState::new(window, cx)),
            dirty: false,
            status: None,
            tasks: Vec::new(),
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn discard_draft(&mut self) {
        self.dirty = false;
        self.status = None;
    }

    fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = Some(ThemeEditorStatus {
            message: message.into(),
            is_error,
        });
    }
}

impl SettingsView {
    /// 每次打开设置时把当前生效主题载入编辑器；有未保存编辑时保留草稿。
    pub(crate) fn prepare_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme_editor.dirty {
            return;
        }
        if let Err(error) = self.load_active_theme_into_editor(window, cx) {
            self.theme_editor.set_status(error, true);
        }
    }

    fn load_active_theme_into_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let name = cx.theme().theme_name().to_string();
        let Some(config) = ThemeRegistry::global(cx)
            .themes()
            .get(name.as_str())
            .cloned()
        else {
            return Err(
                rust_i18n::t!("settings.theme_editor_missing_active", name = name).to_string(),
            );
        };
        let draft = ThemeDraft {
            set_name: format!("{}-custom", theme_file_stem(&name)),
            config: (*config).clone(),
            terminal_palette: crate::theme::terminal_palette_for_ui_theme(&name, cx),
        };
        let content = serialize_theme_document(&draft).map_err(|error| error.to_string())?;
        self.theme_editor.json.update(cx, |textarea, cx| {
            textarea.set_value(content, window, cx);
        });
        self.theme_editor.dirty = false;
        self.theme_editor.status = None;
        Ok(())
    }

    /// 编辑器文本变化后的即时预览。无效 JSON 不覆盖当前画面。
    pub(crate) fn preview_theme_editor(&mut self, cx: &mut Context<Self>) {
        let content = self.theme_editor.json.read(cx).value().to_string();
        match parse_single_theme_document(&content) {
            Ok(draft) => {
                let terminal_changed = apply_theme_preview(&draft, cx);
                self.theme_editor.dirty = true;
                self.theme_editor.set_status(
                    rust_i18n::t!("settings.theme_editor_preview_ok").to_string(),
                    false,
                );
                if terminal_changed {
                    cx.emit(SettingsEvent::ProfilePreview);
                }
            }
            Err(error) => {
                self.theme_editor.dirty = true;
                self.theme_editor.set_status(error.to_string(), true);
            }
        }
        cx.notify();
    }

    /// 保存到用户主题目录并立即注册、应用。
    pub(crate) fn save_theme_editor(&mut self, cx: &mut Context<Self>) {
        let content = self.theme_editor.json.read(cx).value().to_string();
        let mut draft = match parse_single_theme_document(&content) {
            Ok(draft) => draft,
            Err(error) => {
                self.theme_editor.set_status(error.to_string(), true);
                cx.notify();
                return;
            }
        };
        let original_name = draft.config.name.to_string();
        let existing = ThemeRegistry::global(cx)
            .themes()
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let unique = unique_theme_name(&original_name, existing);
        draft.config.name = unique.clone().into();
        draft.set_name = format!("user-{}", theme_file_stem(&unique));
        let content = match serialize_theme_document(&draft) {
            Ok(content) => content,
            Err(error) => {
                self.theme_editor.set_status(error.to_string(), true);
                cx.notify();
                return;
            }
        };
        let path = transom_platform::paths::themes_dir()
            .join(format!("{}.json", theme_file_stem(&unique)));
        let write_content = content.clone();
        let write_path = path.clone();
        let task = cx.spawn(async move |this, cx| {
            let result =
                spawn_blocking_local(move || write_theme_file(&write_path, &write_content)).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_theme_editor_save(result, unique, path, content, cx);
            });
        });
        self.theme_editor.tasks = vec![task];
    }

    fn finish_theme_editor_save(
        &mut self,
        result: Option<Result<(), String>>,
        name: String,
        path: PathBuf,
        content: String,
        cx: &mut Context<Self>,
    ) {
        match result {
            Some(Ok(())) => {
                if let Err(error) = register_theme_document(&content, cx) {
                    self.theme_editor.set_status(error.to_string(), true);
                    cx.notify();
                    return;
                }
                update_app_settings(cx, |settings| {
                    settings.appearance.ui_theme = name.clone();
                });
                let appearance = app_settings(cx).appearance.clone();
                crate::theme::apply_appearance(&appearance, cx);
                self.sync_terminal_palette(cx);
                self.theme_editor.dirty = false;
                self.theme_editor.set_status(
                    rust_i18n::t!(
                        "settings.theme_editor_saved",
                        name = name,
                        path = path.display().to_string()
                    )
                    .to_string(),
                    false,
                );
            }
            Some(Err(error)) => self.theme_editor.set_status(
                rust_i18n::t!("settings.theme_editor_save_failed", error = error).to_string(),
                true,
            ),
            None => self.theme_editor.set_status(
                rust_i18n::t!("settings.theme_editor_thread_failed").to_string(),
                true,
            ),
        }
        cx.notify();
    }

    /// 从磁盘导入单主题 JSON。文件选择器同步弹出，实际读取走阻塞线程池。
    pub(crate) fn import_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("settings.theme_editor_import").to_string())
            .add_filter(
                rust_i18n::t!("settings.theme_editor_json_filter").to_string(),
                &["json"],
            )
            .pick_file()
        else {
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            let read_path = path.clone();
            let result = spawn_blocking_local(move || read_theme_file(&read_path)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_theme_editor_import(result, window, cx);
            });
        });
        self.theme_editor.tasks = vec![task];
    }

    fn finish_theme_editor_import(
        &mut self,
        result: Option<Result<String, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Some(Ok(content)) => match parse_single_theme_document(&content) {
                Ok(_) => {
                    self.theme_editor.json.update(cx, |textarea, cx| {
                        textarea.set_value(content, window, cx);
                    });
                    self.theme_editor.dirty = true;
                    self.preview_theme_editor(cx);
                    self.theme_editor.set_status(
                        rust_i18n::t!("settings.theme_editor_imported").to_string(),
                        false,
                    );
                }
                Err(error) => self.theme_editor.set_status(error.to_string(), true),
            },
            Some(Err(error)) => self.theme_editor.set_status(
                rust_i18n::t!("settings.theme_editor_import_failed", error = error).to_string(),
                true,
            ),
            None => self.theme_editor.set_status(
                rust_i18n::t!("settings.theme_editor_thread_failed").to_string(),
                true,
            ),
        }
        cx.notify();
    }

    /// 导出当前编辑文本到用户选择的 JSON 文件。
    pub(crate) fn export_theme_editor(&mut self, cx: &mut Context<Self>) {
        let content = self.theme_editor.json.read(cx).value().to_string();
        if let Err(error) = parse_single_theme_document(&content) {
            self.theme_editor.set_status(error.to_string(), true);
            cx.notify();
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("settings.theme_editor_export").to_string())
            .set_file_name("transom-theme.json")
            .add_filter(
                rust_i18n::t!("settings.theme_editor_json_filter").to_string(),
                &["json"],
            )
            .save_file()
        else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let write_path = path.clone();
            let result =
                spawn_blocking_local(move || write_theme_file(&write_path, &content)).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Some(Ok(())) => this.theme_editor.set_status(
                        rust_i18n::t!(
                            "settings.theme_editor_exported",
                            path = path.display().to_string()
                        )
                        .to_string(),
                        false,
                    ),
                    Some(Err(error)) => this.theme_editor.set_status(
                        rust_i18n::t!("settings.theme_editor_export_failed", error = error)
                            .to_string(),
                        true,
                    ),
                    None => this.theme_editor.set_status(
                        rust_i18n::t!("settings.theme_editor_thread_failed").to_string(),
                        true,
                    ),
                }
                cx.notify();
            });
        });
        self.theme_editor.tasks = vec![task];
    }

    /// 主题编辑器分组：完整 JSON 编辑、即时预览、保存、导入与导出。
    pub(crate) fn render_settings_theme_editor(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let status = self.theme_editor.status.clone();
        let note = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("settings.theme_editor_note").to_string());
        let editor = Textarea::new(&self.theme_editor.json).h(px(280.0)).w_full();
        let controls = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("theme-editor-load-active")
                    .small()
                    .label(rust_i18n::t!("settings.theme_editor_load_active").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Err(error) = this.load_active_theme_into_editor(window, cx) {
                            this.theme_editor.set_status(error, true);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("theme-editor-save")
                    .primary()
                    .small()
                    .label(rust_i18n::t!("settings.theme_editor_save").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.save_theme_editor(cx))),
            )
            .child(
                Button::new("theme-editor-import")
                    .small()
                    .label(rust_i18n::t!("settings.theme_editor_import").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_theme_editor(window, cx);
                    })),
            )
            .child(
                Button::new("theme-editor-export")
                    .small()
                    .label(rust_i18n::t!("settings.theme_editor_export").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.export_theme_editor(cx))),
            );
        let status_row = status.map(|status| {
            div()
                .text_xs()
                .text_color(if status.is_error { p.danger } else { p.success })
                .child(status.message)
        });
        crate::widgets::setting_frame(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(note)
                .child(editor)
                .child(controls)
                .children(status_row)
                .into_any_element(),
            p,
        )
    }
}

fn read_theme_file(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| {
        rust_i18n::t!(
            "settings.theme_editor_open_failed",
            path = path.display(),
            error = error
        )
        .to_string()
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_THEME_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            rust_i18n::t!(
                "settings.theme_editor_read_failed",
                path = path.display(),
                error = error
            )
            .to_string()
        })?;
    if bytes.len() > MAX_THEME_DOCUMENT_BYTES {
        return Err(rust_i18n::t!(
            "settings.theme_editor_file_too_large",
            path = path.display(),
            bytes = bytes.len()
        )
        .to_string());
    }
    String::from_utf8(bytes).map_err(|error| {
        rust_i18n::t!(
            "settings.theme_editor_not_utf8",
            path = path.display(),
            error = error
        )
        .to_string()
    })
}

fn write_theme_file(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        rust_i18n::t!(
            "settings.theme_editor_output_without_parent",
            path = path.display()
        )
        .to_string()
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        rust_i18n::t!(
            "settings.theme_editor_create_dir_failed",
            path = parent.display(),
            error = error
        )
        .to_string()
    })?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("theme"),
        std::process::id()
    ));
    std::fs::write(&temp, content).map_err(|error| {
        rust_i18n::t!(
            "settings.theme_editor_temp_write_failed",
            path = temp.display(),
            error = error
        )
        .to_string()
    })?;
    std::fs::rename(&temp, path).map_err(|error| {
        if let Err(cleanup_error) = std::fs::remove_file(&temp) {
            log::warn!(
                "清理主题临时文件失败，路径: {}，原因: {cleanup_error}",
                temp.display()
            );
        }
        rust_i18n::t!(
            "settings.theme_editor_replace_failed",
            path = path.display(),
            error = error
        )
        .to_string()
    })
}

/// 监听编辑器文本变化并触发即时预览。
pub(crate) fn install_theme_editor_subscription(
    view: &mut SettingsView,
    cx: &mut Context<SettingsView>,
) {
    let json = view.theme_editor.json.clone();
    let subscription = cx.subscribe(&json, |this, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::Change) {
            this.preview_theme_editor(cx);
        }
    });
    view._subscriptions.push(subscription);
}
