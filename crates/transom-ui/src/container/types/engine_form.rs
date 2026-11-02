//! 引擎连接表单的状态与校验。

use super::*;

/// 容器表单状态。
pub(crate) struct ContainerFormState {
    pub(crate) editing: Option<EngineProfileId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) kind: ContainerEngineKind,
    pub(crate) endpoint_choice: EndpointChoice,
    pub(crate) endpoint_value: Entity<InputState>,
    /// 测试连接结果提示。
    pub(crate) test_status: Option<String>,
    pub(crate) test_ok: bool,
    pub(crate) test_request: Option<ContainerRequestId>,
}

impl ContainerFormState {
    /// 脏判定用的文本快照：名称与端点值。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        vec![
            self.name.read(cx).value().to_string(),
            self.endpoint_value.read(cx).value().to_string(),
        ]
    }

    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
        profile: Option<&ContainerEngineProfile>,
    ) -> Self {
        let name_value = profile
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let engine = profile
            .map(|profile| profile.engine.clone())
            .unwrap_or_else(|| ContainerEngineRef::auto(ContainerEngineKind::Docker));
        let endpoint_choice = EndpointChoice::from_endpoint(&engine.endpoint);
        let endpoint_value = EndpointChoice::value_for(&engine.endpoint);
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name_value)
                .placeholder(rust_i18n::t!("container.form.name_placeholder").to_string())
        });
        let endpoint = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(endpoint_value)
                .placeholder(rust_i18n::t!("container.form.endpoint_placeholder").to_string())
        });
        Self {
            editing: profile.map(|profile| profile.id),
            name,
            kind: engine.kind,
            endpoint_choice,
            endpoint_value: endpoint,
            test_status: None,
            test_ok: false,
            test_request: None,
        }
    }

    /// 构造保存/测试用的连接配置。
    ///
    /// # Errors
    /// 名称为空、需要端点值却留空，或端点在当前平台不可用时返回中文提示。
    pub(crate) fn to_profile(&self, cx: &App) -> Result<ContainerEngineProfile, String> {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(rust_i18n::t!("container.form.name_required").to_string());
        }
        let value = self.endpoint_value.read(cx).value().to_string();
        if self.endpoint_choice.needs_value() && value.trim().is_empty() {
            return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
        }
        let endpoint = self.endpoint_choice.to_endpoint(&value);
        endpoint
            .ensure_supported()
            .map_err(|reason| reason.to_string())?;
        let engine = ContainerEngineRef::explicit(self.kind, endpoint);
        let mut profile = ContainerEngineProfile::new(name, engine);
        if let Some(id) = self.editing {
            profile.id = id;
        }
        Ok(profile)
    }
}
