//! 请求表单体编辑器：支持文本字段和 multipart 文件字段。

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
    pub(super) description: Entity<InputState>,
    _subscriptions: [Subscription; 3],
}

/// 请求表单体三列表格。
pub(crate) struct FormEditor {
    pub(super) rows: Vec<FormRow>,
    next_row_id: u64,
    syncing: bool,
    pub(super) allow_files: bool,
    file_pick_task: Option<Task<()>>,
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
        };
        editor.push_row(blank_field(), window, cx);
        editor
    }

    /// 设置表单字段，并控制当前是否允许文件类型。
    pub(crate) fn set_fields(
        &mut self,
        fields: &[ApiFormField],
        allow_files: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncing = true;
        self.allow_files = allow_files;
        self.rows.clear();
        for field in fields {
            self.push_row(field.clone(), window, cx);
        }
        self.push_row(blank_field(), window, cx);
        self.syncing = false;
        cx.notify();
    }

    /// 切换是否允许文件字段；不允许时把文件行视为文本。
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
        if !allow_files {
            for row in &mut self.rows {
                row.kind = ApiFormFieldKind::Text;
            }
            self.report(cx);
        }
        self.ensure_trailing_blank(window, cx);
        cx.notify();
    }

    /// 返回当前表单字段。
    pub(crate) fn fields(&self, cx: &App) -> Vec<ApiFormField> {
        self.rows
            .iter()
            .map(|row| ApiFormField {
                name: row.name.read(cx).value().trim().to_string(),
                value: row.value.read(cx).value().to_string(),
                description: row.description.read(cx).value().trim().to_string(),
                enabled: row.enabled,
                kind: if self.allow_files {
                    row.kind
                } else {
                    ApiFormFieldKind::Text
                },
            })
            .filter(|field| {
                !field.name.is_empty() || !field.value.is_empty() || !field.description.is_empty()
            })
            .collect()
    }

    pub(super) fn push_row(
        &mut self,
        field: ApiFormField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.form_name_placeholder").to_string())
                .default_value(field.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.form_value_placeholder").to_string())
                .default_value(field.value)
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
        let empty = last.name.read(cx).value().is_empty()
            && last.value.read(cx).value().is_empty()
            && last.description.read(cx).value().is_empty();
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
        row.kind = kind;
        if kind == ApiFormFieldKind::File && row.name.read(cx).value().trim().is_empty() {
            row.name
                .update(cx, |input, cx| input.set_value("file", window, cx));
        }
        self.report(cx);
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
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                    row.kind = ApiFormFieldKind::File;
                    if row.name.read(cx).value().trim().is_empty() {
                        row.name
                            .update(cx, |input, cx| input.set_value("file", window, cx));
                    }
                    row.value.update(cx, |input, cx| {
                        input.set_value(format!("file://{}", path.display()), window, cx)
                    });
                    this.report(cx);
                }
            });
        }));
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
}
