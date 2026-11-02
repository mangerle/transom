//! 容器引擎连接表单的保存与测试连接动作。

use super::*;

impl AppShell {
    pub(crate) fn save_container_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.containers.form.as_ref() else {
            return;
        };
        let profile = match form.to_profile(cx) {
            Ok(profile) => profile,
            Err(message) => {
                if let Some(form) = self.state.containers.form.as_mut() {
                    form.test_ok = false;
                    form.test_status = Some(message);
                }
                cx.notify();
                return;
            }
        };
        self.state.containers.form = None;
        self.save_container_profile(profile, cx);
    }

    pub(super) fn test_container_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.containers.form.as_ref() else {
            return;
        };
        let profile = match form.to_profile(cx) {
            Ok(profile) => profile,
            Err(message) => {
                if let Some(form) = self.state.containers.form.as_mut() {
                    form.test_ok = false;
                    form.test_status = Some(message);
                }
                cx.notify();
                return;
            }
        };
        let request_id = ContainerRequestId::new();
        if let Some(form) = self.state.containers.form.as_mut() {
            form.test_request = Some(request_id);
            form.test_status = Some(rust_i18n::t!("container.form.testing").to_string());
        }
        if !self.state.send_command(ContainerCommand::TestConnection {
            request_id,
            profile,
        }) && let Some(form) = self.state.containers.form.as_mut()
        {
            form.test_request = None;
        }
        cx.notify();
    }
}
