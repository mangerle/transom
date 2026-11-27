//! 请求表单体编辑器：支持文本字段和 multipart 文件字段。

use super::form_file::{
    FormFileStatus, field_value_from_path, inspect_file, path_from_field_value,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use transom_api::{ApiFormField, ApiFormFieldKind};

/// 表单字段变化事件；携带完整快照，父实体无需回读。
pub(crate) enum FormEditorEvent {
    Changed(Vec<ApiFormField>),
}

pub(super) struct FormRow {
    pub(super) id: u64,
    pub(super) enabled: bool,
    pub(super) kind: ApiFormFieldKind,
    pub(super) name: Entity<InputState>,
    pub(super) value: Entity<InputState>,
    pub(super) file_path: Option<String>,
    pub(super) file_status: FormFileStatus,
    pub(super) description: Entity<InputState>,
    _subscriptions: [Subscription; 3],
}

/// 请求表单体字段表格；multipart 模式下每行可切换文本或文件值。
pub(crate) struct FormEditor {
    pub(super) rows: Vec<FormRow>,
    next_row_id: u64,
    syncing: bool,
    pub(super) allow_files: bool,
    file_pick_task: Option<Task<()>>,
    file_status_task: Option<Task<()>>,
}

fn blank_field() -> ApiFormField {
    ApiFormField {
        name: String::new(),
        value: String::new(),
        description: String::new(),
        enabled: true,
        kind: ApiFormFieldKind::Text,
    }
}

impl EventEmitter<FormEditorEvent> for FormEditor {}

impl FormEditor {
    /// 创建空表单编辑器。
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            rows: Vec::with_capacity(8),
            next_row_id: 0,
            syncing: false,
            allow_files: false,
            file_pick_task: None,
            file_status_task: None,
        };
        editor.push_row(blank_field(), window, cx);
        editor
    }

    /// 增量设置表单字段，优先复用已有行实体并控制当前是否允许文件类型。
    pub(crate) fn set_fields(
        &mut self,
        fields: &[ApiFormField],
        allow_files: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncing = true;
        self.allow_files = allow_files;
        let blank = blank_field();
        let target_len = fields.len() + 1;
        let reuse_len = self.rows.len().min(target_len);
        for (index, row) in self.rows.iter_mut().enumerate().take(reuse_len) {
            let field = fields.get(index).unwrap_or(&blank);
            let file_path = if field.kind == ApiFormFieldKind::File {
                path_from_field_value(&field.value)
            } else {
                None
            };
            row.enabled = field.enabled;
            row.kind = field.kind;
            row.file_path = file_path;
            row.file_status = FormFileStatus::Unknown;
            row.name.update(cx, |state, cx| {
                state.set_value(field.name.clone(), window, cx)
            });
            row.value.update(cx, |state, cx| {
                let text = if field.kind == ApiFormFieldKind::File {
                    String::new()
                } else {
                    field.value.clone()
                };
                state.set_value(text, window, cx);
            });
            row.description.update(cx, |state, cx| {
                state.set_value(field.description.clone(), window, cx)
            });
        }
        self.rows.truncate(target_len);
        for index in reuse_len..target_len {
            let field = fields.get(index).cloned().unwrap_or_else(blank_field);
            self.push_row(field, window, cx);
        }
        self.syncing = false;
        self.refresh_file_statuses(window, cx);
        cx.notify();
    }

    /// 切换是否允许文件字段；禁止文件时按文本语义保存和发送。
    pub(crate) fn set_allow_files(
        &mut self,
        allow_files: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.allow_files == allow_files {
            return;
        }
        self.allow_files = allow_files;
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
        cx.notify();
    }

    /// 返回当前表单字段。
    pub(crate) fn fields(&self, cx: &App) -> Vec<ApiFormField> {
        self.rows
            .iter()
            .filter_map(|row| {
                let kind = if self.allow_files {
                    row.kind
                } else {
                    ApiFormFieldKind::Text
                };
                let file_path = row.file_path.as_deref();
                if kind == ApiFormFieldKind::File && file_path.is_none() {
                    return None;
                }
                let value = if kind == ApiFormFieldKind::File {
                    file_path.map(field_value_from_path).unwrap_or_default()
                } else {
                    row.value.read(cx).value().to_string()
                };
                let field = ApiFormField {
                    name: row.name.read(cx).value().trim().to_string(),
                    value,
                    description: row.description.read(cx).value().trim().to_string(),
                    enabled: row.enabled,
                    kind,
                };
                (!field.name.is_empty() || !field.value.is_empty() || !field.description.is_empty())
                    .then_some(field)
            })
            .collect()
    }

    pub(super) fn push_row(
        &mut self,
        field: ApiFormField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text_value = if field.kind == ApiFormFieldKind::Text {
            field.value.clone()
        } else {
            String::new()
        };
        let file_path = if field.kind == ApiFormFieldKind::File {
            path_from_field_value(&field.value)
        } else {
            None
        };
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.form_name_placeholder").to_string())
                .default_value(field.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.form_value_placeholder").to_string())
                .default_value(text_value)
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.parameter_description").to_string())
                .default_value(field.description)
        });
        let watch_name =
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            });
        let watch_value =
            cx.subscribe_in(&value, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            });
        let watch_description = cx.subscribe_in(
            &description,
            window,
            |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            },
        );
        self.next_row_id = self.next_row_id.saturating_add(1);
        self.rows.push(FormRow {
            id: self.next_row_id,
            enabled: field.enabled,
            kind: field.kind,
            name,
            value,
            file_path,
            file_status: FormFileStatus::Unknown,
            description,
            _subscriptions: [watch_name, watch_value, watch_description],
        });
    }

    pub(super) fn on_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) || self.syncing {
            return;
        }
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }

    pub(super) fn ensure_trailing_blank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.rows.last() else {
            self.push_row(blank_field(), window, cx);
            return;
        };
        let empty = if self.allow_files && last.kind == ApiFormFieldKind::File {
            last.name.read(cx).value().is_empty()
                && last.file_path.is_none()
                && last.description.read(cx).value().is_empty()
        } else {
            last.name.read(cx).value().is_empty()
                && last.value.read(cx).value().is_empty()
                && last.description.read(cx).value().is_empty()
        };
        if !empty {
            self.push_row(blank_field(), window, cx);
        }
    }

    pub(super) fn report(&mut self, cx: &mut Context<Self>) {
        cx.emit(FormEditorEvent::Changed(self.fields(cx)));
        cx.notify();
    }

    pub(super) fn toggle(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.enabled = !row.enabled;
            self.report(cx);
        }
    }

    pub(super) fn set_kind(
        &mut self,
        id: u64,
        kind: ApiFormFieldKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        if row.kind == kind {
            return;
        }
        row.kind = kind;
        if kind == ApiFormFieldKind::File && row.name.read(cx).value().trim().is_empty() {
            row.name
                .update(cx, |input, cx| input.set_value("file", window, cx));
        }
        self.report(cx);
        self.refresh_file_statuses(window, cx);
    }

    pub(super) fn pick_file(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self.file_pick_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().to_string();
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                    row.kind = ApiFormFieldKind::File;
                    if row.name.read(cx).value().trim().is_empty() {
                        row.name
                            .update(cx, |input, cx| input.set_value("file", window, cx));
                    }
                    row.file_path = Some(path);
                    row.file_status = FormFileStatus::Checking;
                    this.report(cx);
                    this.refresh_file_statuses(window, cx);
                }
            });
        }));
    }

    pub(super) fn clear_file(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        row.file_path = None;
        row.file_status = FormFileStatus::Unknown;
        self.report(cx);
    }

    pub(super) fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(blank_field(), window, cx);
        self.report(cx);
    }

    pub(super) fn remove(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }

    fn refresh_file_statuses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let allow_files = self.allow_files;
        let files = self
            .rows
            .iter_mut()
            .filter(|row| allow_files && row.kind == ApiFormFieldKind::File)
            .filter_map(|row| {
                let path = row.file_path.clone()?;
                row.file_status = FormFileStatus::Checking;
                Some((row.id, path))
            })
            .collect::<Vec<_>>();
        if files.is_empty() {
            return;
        }
        let inspect_paths = files.clone();
        self.file_status_task = Some(cx.spawn_in(window, async move |this, cx| {
            let inspected = crate::app::spawn_blocking_local(move || {
                inspect_paths
                    .into_iter()
                    .map(|(id, path)| {
                        let status = inspect_file(&path);
                        (id, path, status)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let Some(inspected) = inspected else {
                return;
            };
            let _ = this.update_in(cx, |this, _, cx| {
                for (id, path, status) in inspected {
                    if let Some(row) = this.rows.iter_mut().find(|row| row.id == id)
                        && row.file_path.as_deref() == Some(path.as_str())
                    {
                        row.file_status = status;
                    }
                }
                cx.notify();
            });
        }));
    }
}
