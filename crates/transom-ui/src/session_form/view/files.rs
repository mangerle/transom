//! 会话表单的私钥/证书文件选择。
//!
//! 文件对话框仍在 UI 线程同步弹出；选中的路径直接写回表单输入框，
//! 不需要外壳参与。

use crate::session_form::SessionFormView;
use gpui_kit::*;

impl SessionFormView {
    /// 选择 SSH 私钥文件并回填路径。
    pub(crate) fn pick_private_key_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.state() else {
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
        let Some(form) = self.state() else {
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
