//! 密钥生成与管理向导实体：打开状态、输入控件与公钥扫描都归它。
//!
//! # 设计原理（Why）
//! 向导此前由 `ModalHub.tools.key_wizard: Option<KeyWizardState>` 加散落的
//! `impl AppShell` 方法维护：打开、扫描、生成、复制、导出与部署分居
//! `keys/mod.rs` 与 `keys/view.rs`，读者要同时读三处才能回答“谁拥有这次打开”。
//! 实体化后，一次打开期的输入控件、公钥清单与在途生成任务同生共死，外壳只
//! 持有句柄；状态变更也只重绘本实体，不再牵动整个外壳。
//!
//! # 代价与局限
//! 部署目标列表来自外壳的工作区标签页，实体不反查外壳，改由外壳在渲染时
//! 以 `&[KeyTarget]` 注入；跨域的状态栏提示通过事件上报。

use super::keygen::scan_public_keys;
use super::state::KeyWizardState;
use crate::palette::localized_error;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::AppCommand;

/// 密钥生成与管理向导实体。
pub(crate) struct KeyWizardView {
    /// 未打开时为 `None`；每次打开都重新创建输入控件，避免残留上次的路径与口令。
    state: Option<KeyWizardState>,
    /// SSH 目录在构造时固定，打开与生成完成后都扫描这一份。
    ssh_dir: PathBuf,
    /// 命令通道：部署公钥直接投递，不经过外壳。
    command_tx: flume::Sender<AppCommand>,
    /// 在途生成任务；下一次生成覆盖旧句柄，关闭弹窗时随之取消。
    task: Option<Task<()>>,
}

impl KeyWizardView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new(ssh_dir: PathBuf, command_tx: flume::Sender<AppCommand>) -> Self {
        Self {
            state: None,
            ssh_dir,
            command_tx,
            task: None,
        }
    }

    /// 向导是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn state(&self) -> Option<&KeyWizardState> {
        self.state.as_ref()
    }

    /// 当前打开期的可变状态；动作在实体内部就地修改。
    pub(crate) fn state_mut(&mut self) -> Option<&mut KeyWizardState> {
        self.state.as_mut()
    }

    /// 命令通道的只读访问。
    pub(crate) fn command_tx(&self) -> &flume::Sender<AppCommand> {
        &self.command_tx
    }

    /// 打开向导并扫描本地公钥。
    ///
    /// 扫描与迁移前一样在打开时同步执行：目录通常只有几个 `.pub` 文件，
    /// 异步化会让列表先空一帧；真正耗时的密钥生成本身已放到后台线程。
    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut state = KeyWizardState::new(window, cx, self.ssh_dir.clone());
        match scan_public_keys(&self.ssh_dir) {
            Ok(keys) => {
                state.selected = (!keys.is_empty()).then_some(0);
                state.keys = keys;
            }
            Err(error) => state.error = Some(localized_error(&error)),
        }
        self.state = Some(state);
        cx.notify();
    }

    /// 关闭向导；在途生成任务随句柄一起取消。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.task.take();
        self.state = None;
        cx.notify();
    }

    /// 在弹窗内显示一条成功提示，并清空错误。
    pub(crate) fn set_notice(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.state.as_mut() {
            state.message = Some(message.to_string());
            state.error = None;
        }
        cx.notify();
    }

    /// 在弹窗内显示一条错误，并清空成功提示。
    pub(crate) fn set_error(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.state.as_mut() {
            state.error = Some(message.to_string());
            state.message = None;
        }
        cx.notify();
    }
}
