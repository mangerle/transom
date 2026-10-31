//! 会话表单的私钥/证书文件选择。

use super::*;

impl AppShell {
    pub(crate) fn pick_private_key_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.state.modals.form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_key").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.key_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }

    /// 选择 CA 签发的用户证书文件（OpenSSH 惯例是 `私钥名-cert.pub`）。
    pub(crate) fn pick_certificate_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.state.modals.form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_cert").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .add_filter(rust_i18n::t!("form.cert_filter").to_string(), &["pub"])
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.cert_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }
}
