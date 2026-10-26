//! 会话增删改查、标签页激活/关闭/轮转与界面语言切换模块。

use crate::app::{AppShell, TabGroup, TabStatus, TerminalTab};
use crate::palette::localized_error;
use crate::pane_tree::{LayoutNode, RemoveOutcome};
use crate::session_form::SessionForm;
use crate::sftp::PendingEditorClose;
use crate::tiling::StripEntry;
use gpui_kit::component::set_locale;
use gpui_kit::*;
use transom_core::{
    AppCommand, AppError, Connection, ErrorCode, GroupId, SavedTabGroup, SecretKey, SecretKind,
    SecretStore, Session, SessionId, TabId,
};
use zeroize::Zeroizing;

impl AppShell {
    /// 新建标签组并返回组 ID；调用方随后把首个窗格加入 `tabs`。
    ///
    /// 顺带分配同会话的额外标签序号：该会话的第一个标签拿 0（界面不显示后缀），
    /// 之后依次 1、2... 序号一经确定就不回改已有标签——新开一个标签不该让原来那个
    /// 从 "web-01" 变成 "web-01 #1"。当前同会话标签全部关完后，下一个重新回到 0。
    pub(crate) fn create_group(&mut self, tab_id: TabId, session_id: SessionId) -> GroupId {
        let serial = self
            .groups
            .iter()
            .filter(|group| group.session_id == session_id)
            .map(|group| group.serial)
            .max()
            .map_or(0, |max| max + 1);
        let group = TabGroup::new(tab_id, session_id, serial);
        let id = group.id;
        self.groups.push(group);
        id
    }

    /// 新建本地终端标签组，序号按"本地终端的基础标题"分配。
    ///
    /// 本地终端每次打开都会新建一个临时 Session，ID 各不相同；若沿用
    /// `create_group` 的"按会话 ID 计序号"，每个本地标签都会拿到 0，标签栏上
    /// 就会出现一排同名条目。这里改为按显示标题归组：普通本地终端共用一个
    /// 序列，同一目录打开的终端共用另一个序列（标题里的目录名相同），
    /// 第一个不带后缀，之后依次 `#1`、`#2`。这是 `create_group` 的序号规则
    /// 在临时会话上的等价物，已确定的序号同样不回改。
    pub(crate) fn create_local_group(
        &mut self,
        tab_id: TabId,
        session_id: SessionId,
        base_title: &str,
    ) -> GroupId {
        let serial = self
            .groups
            .iter()
            .filter_map(|group| {
                let tab = self.tabs.iter().find(|tab| tab.id == group.active_pane)?;
                if !matches!(tab.session.connection, Connection::Local(_)) {
                    return None;
                }
                (tab.session.name == base_title).then_some(group.serial)
            })
            .max()
            .map_or(0, |max| max + 1);
        let id = self.create_group(tab_id, session_id);
        if let Some(group) = self.groups.iter_mut().find(|group| group.id == id) {
            group.serial = serial;
        }
        id
    }

    /// 根据窗口设置恢复一个标签组及其分割树布局，返回实际创建的标签组。
    ///
    /// 布局直接来自设置里的分割树形状。设置文件属外部输入：叶子数超过上限或
    /// 形状损坏时按单窗格处理，绝不一次拉起大量连接。
    ///
    /// 这里不按会话去重：同一会话并列多个标签组是正常状态，去重会让第二个及以后的
    /// 同会话标签在重启后凭空消失。恢复的组数天然等于关窗口时实际打开的标签数。
    /// 会话缺失等无法创建组的情况返回 `None`，供并排视图按原下标精确回填。
    pub(crate) fn restore_tab_group(
        &mut self,
        saved: &SavedTabGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<GroupId> {
        let pane_count = LayoutNode::saved_pane_count(&saved.layout).unwrap_or(1);

        let groups_before = self.groups.len();
        if saved.session == "local" {
            self.open_local_terminal(window, cx);
        } else {
            let session = self
                .catalog
                .sessions
                .iter()
                .find(|session| session.id.to_string() == saved.session)
                .cloned()?;
            self.connect_session(session.id, window, cx);
        }
        if self.groups.len() <= groups_before {
            return None;
        }
        let group_index = self.groups.len() - 1;
        let group_id = self.groups[group_index].id;
        if pane_count < 2 {
            return Some(group_id);
        }
        let first = self.groups[group_index].active_pane;
        let Some(session) = self.session_for_tab(first) else {
            return Some(group_id);
        };
        // 串口端口独占，恢复时只保留首个窗格。
        if matches!(session.connection, Connection::Serial(_)) {
            return Some(group_id);
        }
        let extra: Vec<TabId> = (1..pane_count).map(|_| TabId::new()).collect();
        let mut ids = std::iter::once(first).chain(extra.iter().copied());
        let Some(root) = LayoutNode::from_saved(&saved.layout, &mut ids) else {
            return Some(group_id);
        };
        self.groups[group_index].root = root;
        for pane_id in extra {
            self.tabs
                .push(TerminalTab::pending(pane_id, session.clone()));
            self.start_connection(pane_id, session.clone(), cx);
        }
        cx.notify();
        Some(group_id)
    }

    /// 取窗格应当使用的会话配置：优先目录里的最新记录，退回标签上的快照。
    ///
    /// 分屏、重连与窗口恢复都要拿"当前生效的会话"再开一条连接，集中一处，
    /// 避免目录被批量编辑或导入替换后三条路径读到陈旧配置。
    pub(crate) fn session_for_tab(&self, tab_id: TabId) -> Option<Session> {
        let index = self.tabs.iter().position(|tab| tab.id == tab_id)?;
        let session_id = self.tabs[index].session_id;
        Some(
            self.catalog
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .cloned()
                .unwrap_or_else(|| self.tabs[index].session.clone()),
        )
    }

    /// 将已打开标签的会话快照同步为目录中的最新配置。
    ///
    /// 批量编辑、备份导入等操作会整体替换目录记录；已连接终端不受影响，
    /// 但分屏、重连与初始化命令应使用新配置。
    /// 目录里的凭据模板快照，供会话表单渲染可选项。
    ///
    /// 只给 id/名字/用户名三项：表单不需要认证细节，把目录对象整个传进去会让
    /// 表单有能力改目录数据，破坏"表单只产出 BuiltSession"的边界。
    pub(crate) fn identity_options(&self) -> Vec<crate::session_form::IdentityOption> {
        self.catalog
            .identities
            .iter()
            .map(|identity| crate::session_form::IdentityOption {
                id: identity.id,
                name: identity.name.clone(),
                username: identity.username.clone(),
            })
            .collect()
    }

    pub(crate) fn refresh_tab_sessions(&mut self) {
        for tab in &mut self.tabs {
            if let Some(session) = self
                .catalog
                .sessions
                .iter()
                .find(|session| session.id == tab.session_id)
            {
                // 刷新写回的是"叠加凭据模板之后"的会话：标签上的用户名与认证
                // 方式必须与真正拿去连接的参数一致，否则改完模板界面还显示旧身份。
                tab.session = self.catalog.resolved_session(session);
            }
        }
    }

    pub(crate) fn group_index_for_tab(&self, tab_id: TabId) -> Option<usize> {
        self.groups.iter().position(|group| group.contains(tab_id))
    }

    /// 该窗格此刻是否正被渲染：属于可见标签组、在分割树里，且没被"单窗格最大化"挡在后面。
    ///
    /// 传输进度这类高频事件据此决定要不要整帧重绘外壳：后台标签、并排视图之外的
    /// 标签以及被最大化隐藏的兄弟窗格重绘了也看不见，白付一次布局成本。
    pub(crate) fn is_rendered_pane(&self, tab_id: TabId) -> bool {
        let Some(group_index) = self.group_index_for_tab(tab_id) else {
            return false;
        };
        let group = &self.groups[group_index];
        // 活动标签在并排网格里时，整张网格都可见；否则只显示活动标签组。
        let visible = match self.active_tiling() {
            Some(tiled) => tiled.contains(group.id),
            None => self.active_group_index() == Some(group_index),
        };
        if !visible {
            return false;
        }
        match group.zoomed {
            Some(zoomed) => zoomed == tab_id,
            None => group.root.contains(tab_id),
        }
    }

    pub(crate) fn active_group_index(&self) -> Option<usize> {
        self.active_tab
            .and_then(|tab_id| self.group_index_for_tab(tab_id))
    }

    /// 从所属标签组的分割树里摘除一个窗格，返回应接管的焦点窗格。
    ///
    /// 仅处理组与布局关系，不触碰 `tabs` 本身或发送断开命令，供关闭窗格、
    /// 删除会话、摘出为独立标签等多条路径复用。摘到无窗格时整组移除。
    fn detach_pane_from_group(&mut self, tab_id: TabId) -> Option<TabId> {
        let group_index = self.group_index_for_tab(tab_id)?;
        let was_active = self.active_tab == Some(tab_id);
        let replacement = match self.groups[group_index].root.remove_leaf(tab_id) {
            RemoveOutcome::Empty => {
                self.groups.remove(group_index);
                if was_active {
                    self.groups
                        .get(group_index.saturating_sub(1))
                        .or_else(|| self.groups.first())
                        .map(|group| group.active_pane)
                } else {
                    None
                }
            }
            RemoveOutcome::Removed { next_focus } => {
                let group = &mut self.groups[group_index];
                if group.active_pane == tab_id {
                    group.active_pane = next_focus;
                }
                if group.zoomed == Some(tab_id) {
                    group.zoomed = None;
                }
                if was_active { Some(next_focus) } else { None }
            }
        };
        self.prune_pane_split_states();
        replacement
    }

    pub(crate) fn open_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(SessionForm::new(
            window,
            cx,
            None,
            None,
            self.identity_options(),
        ));
        cx.notify();
    }

    pub(crate) fn open_edit_session(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
        else {
            return;
        };
        let folder_path = session
            .folder_id
            .and_then(|id| self.catalog.folder_path(id));
        let is_serial = matches!(session.connection, Connection::Serial(_));
        let mut form = SessionForm::new(
            window,
            cx,
            Some(session),
            folder_path,
            self.identity_options(),
        );
        self.prefill_secrets(&mut form, window, cx);
        self.form = Some(form);
        if is_serial {
            self.refresh_serial_ports(window, cx);
        }
    }

    /// 把编辑中会话已保存在钥匙串里的凭据填回表单输入框。
    ///
    /// 界面因此把输入框当作"已存口令的编辑视图"：显示为掩码点，点眼睛可以看，
    /// 清空它就表示要删掉这条凭据（见 `save_session_form`）。只填当前会显示出来的
    /// 那一项；读不到就留空，由界面上的空值提示说明后果。
    fn prefill_secrets(
        &mut self,
        form: &mut SessionForm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = form.base.as_ref() else {
            return;
        };
        let session_id = base.id;
        // 只回填勾选了保存的凭据类型；输入框先按空值打开，读到后异步回填
        let mut fields = Vec::new();
        if form.save_password {
            fields.push((SecretKind::Password, form.password.clone()));
        }
        if form.save_passphrase {
            fields.push((SecretKind::KeyPassphrase, form.passphrase.clone()));
        }
        if fields.is_empty() {
            return;
        }
        // 钥匙串走 D-Bus 可能挂起秒级：后台读取，完成后经窗口上下文回填输入框
        let secret_store = self.services.secret_store.clone();
        let executor = cx.background_executor().clone();
        let read_fields = fields.clone();
        window
            .spawn(cx, async move |cx| {
                let secrets = executor
                    .spawn(async move {
                        read_fields
                            .iter()
                            .map(|(kind, _)| {
                                secret_store
                                    .get(&SecretKey::new(session_id, *kind))
                                    .ok()
                                    .flatten()
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = cx.update(|window, cx| {
                    for ((_, field), secret) in fields.into_iter().zip(secrets) {
                        if let Some(secret) = secret {
                            field.update(cx, |state, cx| {
                                state.set_value(secret.as_str(), window, cx);
                            });
                        }
                    }
                });
            })
            .detach();
    }

    /// 保存会话表单：只做目录落盘与凭据写入，不发起连接。
    ///
    /// 保存与连接是两个动作：新建会话后要用它，去列表里双击（或右键"连接"）。
    /// 早先新建路径会顺手 `connect_session`，那既绕过了"双击才连接"的约定，
    /// 也会在没保存凭据时刚填完表单又被追问一次密码。
    pub(crate) fn save_session_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let editing = form.base.is_some();
        let save_password = form.save_password;
        let save_passphrase = form.save_passphrase;
        let built = match form.build_session(cx) {
            Ok(built) => built,
            Err(error) => {
                form.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.form = None;

        let mut built = built;
        built.session.folder_id = built
            .folder_path
            .as_deref()
            .and_then(|path| self.catalog.ensure_folder_path(path));
        let password_key = SecretKey::new(built.session.id, SecretKind::Password);
        let passphrase_key = SecretKey::new(built.session.id, SecretKind::KeyPassphrase);
        // 勾选保存但把内容清空，语义就是"删掉这条凭据"：输入框现在是已存口令的
        // 编辑视图，清空必须能生效，否则用户的删除意图会被无声忽略。
        // 与登录密码同规则：清空已回填的口令即表示删除。
        //
        // 引用凭据模板时整段跳过：口令归模板所有，会话这边既不该写进自己的槽位，
        // 也不该把模板的槽位误删——会话自己的残留值留到它退回内联认证时再说。
        let mut secret_writes = Vec::new();
        if built.session.identity_id.is_none() {
            if save_password {
                match &built.password {
                    Some(password) => {
                        secret_writes.push(SecretWrite::Set(password_key, password.clone()))
                    }
                    None => secret_writes.push(SecretWrite::Delete(password_key)),
                }
            } else {
                secret_writes.push(SecretWrite::Delete(password_key));
            }
            if save_passphrase {
                match &built.key_passphrase {
                    Some(passphrase) => {
                        secret_writes.push(SecretWrite::Set(passphrase_key, passphrase.clone()))
                    }
                    None => secret_writes.push(SecretWrite::Delete(passphrase_key)),
                }
            } else {
                secret_writes.push(SecretWrite::Delete(passphrase_key));
            }
        }
        // 钥匙串写入/删除走 D-Bus 可能挂起秒级：放后台执行，失败结果异步回填错误栏
        let secret_store = self.services.secret_store.clone();
        cx.spawn(async move |this, cx| {
            let error = if secret_writes.is_empty() {
                None
            } else {
                cx.background_executor()
                    .spawn(
                        async move { apply_secret_writes(secret_store.as_ref(), &secret_writes) },
                    )
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(error) = error {
                    this.error = Some(localized_error(&error));
                    cx.notify();
                }
            });
        })
        .detach();

        let session_id = built.session.id;
        if editing {
            if let Some(existing) = self
                .catalog
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
            {
                *existing = built.session;
            }
        } else {
            self.catalog.sessions.push(built.session);
        }
        self.persist_catalog();
        cx.notify();
    }

    pub(crate) fn delete_session_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.take() else {
            return;
        };
        let Some(base) = form.base else {
            return;
        };
        self.delete_session_by_id(base.id, cx);
    }

    pub(crate) fn delete_session_by_id(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        // 会话下任一标签存在未保存的编辑面板时，删除会被闸门挂起，
        // 待用户处理完毕后续行（同关闭标签组的保护水位）。
        let removed_tabs: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|tab| tab.session_id == session_id)
            .map(|tab| tab.id)
            .collect();
        if self.block_close_for_dirty_editors(
            &removed_tabs,
            PendingEditorClose::Session(session_id),
            cx,
        ) {
            return;
        }
        self.catalog
            .sessions
            .retain(|session| session.id != session_id);
        self.selected_sessions.remove(&session_id);
        for tab_id in &removed_tabs {
            self.send_command(AppCommand::CloseTab { tab_id: *tab_id });
        }
        self.tabs.retain(|tab| tab.session_id != session_id);
        self.prune_editor_panels();
        let tiled_focus = self.active_tiling().and_then(|tiled| tiled.focused_group());
        let mut replacement = None;
        for tab_id in removed_tabs {
            if let Some(next) = self.detach_pane_from_group(tab_id) {
                replacement = Some(next);
            }
        }
        self.prune_tilings();
        if self
            .active_tab
            .is_none_or(|id| !self.tabs.iter().any(|tab| tab.id == id))
        {
            self.active_tab = replacement
                .or_else(|| {
                    self.active_tiling()
                        .and_then(|tiled| tiled.focused_group())
                        .and_then(|group| {
                            self.groups
                                .iter()
                                .find(|item| item.id == group)
                                .map(|item| item.active_pane)
                        })
                })
                .or_else(|| {
                    tiled_focus.and_then(|group| {
                        self.groups
                            .iter()
                            .find(|item| item.id == group)
                            .map(|item| item.active_pane)
                    })
                })
                .or_else(|| self.groups.first().map(|group| group.active_pane));
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
        let _ = self
            .services
            .secret_store
            .delete(&SecretKey::new(session_id, SecretKind::Password));
        let _ = self
            .services
            .secret_store
            .delete(&SecretKey::new(session_id, SecretKind::KeyPassphrase));
        self.persist_catalog();
        cx.notify();
    }

    pub(crate) fn duplicate_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(mut session) = self
            .catalog
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .cloned()
        else {
            return;
        };
        let new_id = SessionId::new();
        let name = session.name.clone();
        session.id = new_id;
        session.name = self.unique_duplicate_name(&name);
        self.catalog.sessions.push(session);
        // 凭据按会话 ID 存放：不跟着复制，副本就成了"配置一样却要重新输密码"的会话。
        self.copy_secrets(session_id, new_id);
        self.persist_catalog();
        cx.notify();
    }

    /// 为副本挑一个目录里未被占用的名字：`X 副本`、`X 副本 #1`、`X 副本 #2`...
    ///
    /// 与标签序号同构的约定：已有会话的名字一个都不改，只在候选里取第一个空闲位，
    /// 因此连点两次复制不会造出两个同名会话。候选上界取目录大小 + 2，按鸽笼原理
    /// 必然存在未占用位；最后的兜底分支理论上不可达，只为免掉 unwrap。
    fn unique_duplicate_name(&self, original: &str) -> String {
        let base = duplicate_base(original);
        let first = rust_i18n::t!("common.duplicate_name", name = base).to_string();
        let taken = |candidate: &str| self.catalog.sessions.iter().any(|s| s.name == candidate);
        if !taken(&first) {
            return first;
        }
        let limit = self.catalog.sessions.len() + 2;
        (1..=limit)
            .map(|index| format!("{first} #{index}"))
            .find(|candidate| !taken(candidate))
            .unwrap_or_else(|| format!("{first} #{}", limit + 1))
    }

    /// 把一个会话名下的钥匙串条目复制到另一个会话 ID。
    ///
    /// 钥匙串不可用只降级为提示，副本记录照常创建：复制会话本身不依赖凭据。
    fn copy_secrets(&mut self, from: SessionId, to: SessionId) {
        for kind in [SecretKind::Password, SecretKind::KeyPassphrase] {
            let source = SecretKey::new(from, kind);
            let target = SecretKey::new(to, kind);
            match self.services.secret_store.get(&source) {
                Ok(Some(secret)) => {
                    if let Err(error) = self.services.secret_store.set(&target, &secret)
                        && error.code != ErrorCode::SecretStoreUnavailable
                    {
                        self.error = Some(localized_error(&error));
                    }
                }
                Ok(None) => {}
                Err(error) if error.code == ErrorCode::SecretStoreUnavailable => {}
                Err(error) => self.error = Some(localized_error(&error)),
            }
        }
    }

    pub(crate) fn close_other_tabs(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_group) = self.group_index_for_tab(tab_id) else {
            return;
        };
        let target_group_id = self.groups[target_group].id;
        // 合并条目代表整个并排网格："关闭其他标签"必须保留网格里的全部标签，
        // 否则用户会在一个视觉条目上操作，却把旁边的搭档也关掉。
        let mut keep = vec![target_group_id];
        if let Some(tiled) = self.tiling_of_group(target_group_id) {
            keep.extend(tiled.occupied_groups());
        }
        let others: Vec<GroupId> = self
            .groups
            .iter()
            .filter(|group| !keep.contains(&group.id))
            .map(|group| group.id)
            .collect();
        for group_id in others {
            self.close_group(group_id, window, cx);
        }
    }

    pub(crate) fn reconnect_tab(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(session) = self.session_for_tab(tab_id) else {
            return;
        };
        let tab = &mut self.tabs[index];
        tab.terminal = None;
        tab.sftp = None;
        tab._subscription = None;
        tab.connection_id = None;
        tab.forward_statuses.clear();
        tab.network_traffic = None;
        tab.status = TabStatus::Connecting;
        tab.title = session.name.clone();
        self.active_tab = Some(tab_id);
        if let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.groups[group_index].active_pane = tab_id;
        }
        self.present_active_group();
        self.start_connection(tab_id, session, cx);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 键盘/标签栏切换不一定会让鼠标离开原文件行，主动收起旧标签的悬停
        // 预览，避免切回该标签时浮层凭空重现。
        if let Some(previous) = self.active_tab
            && previous != tab_id
        {
            self.dismiss_sftp_hover_preview(previous, cx);
        }
        // 并排网格是持久的：点到一个不在网格里的标签时，单屏显示它、网格保留；
        // 点回网格里的标签则恢复多格同屏，并更新焦点槽。
        let group_id = self
            .group_index_for_tab(tab_id)
            .map(|index| self.groups[index].id);
        if let Some(group_id) = group_id
            && let Some(tiled) = self
                .tiled_sets
                .iter_mut()
                .find(|tiled| tiled.contains(group_id))
            && let Some(slot) = tiled.slot_of(group_id)
        {
            tiled.focused = slot;
        }
        self.active_tab = Some(tab_id);
        if let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.groups[group_index].active_pane = tab_id;
        }
        if let Some(terminal) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.terminal.as_ref())
        {
            let focus_handle = terminal.read(cx).focus_handle().clone();
            window.focus(&focus_handle, cx);
        }
        // 广播条开着时，目标集合跟随标签切换；同一标签内换焦点不会重置。
        self.sync_broadcast_targets();
        cx.notify();
    }

    /// 关闭窗格所属的整个标签组。
    pub(crate) fn close_tab(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self
            .group_index_for_tab(tab_id)
            .map(|index| self.groups[index].id)
        else {
            return;
        };
        self.close_group(group_id, window, cx);
    }

    /// 关闭标签组，并回收其中全部窗格的连接。
    ///
    /// 组内任一窗格存在未保存的编辑面板时，关闭会被闸门挂起：先弹出各面板的
    /// 丢弃确认横幅，用户全部处理完毕后自动续行本次关闭。
    pub(crate) fn close_group(
        &mut self,
        group_id: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.groups.iter().position(|group| group.id == group_id) else {
            return;
        };
        let panes = self.groups[index].pane_ids();
        if self.block_close_for_dirty_editors(&panes, PendingEditorClose::Group(group_id), cx) {
            return;
        }
        // 关闭前记录所在并排组的焦点：网格解除后仍可回到原焦点标签。
        let tiled_focus = self
            .tiling_of_group(group_id)
            .and_then(|tiled| tiled.focused_group());
        self.groups.remove(index);
        for tab_id in &panes {
            self.send_command(AppCommand::CloseTab { tab_id: *tab_id });
        }
        self.tabs.retain(|tab| !panes.contains(&tab.id));
        self.prune_pane_split_states();
        self.prune_editor_panels();
        self.tab_selection.remove(&group_id);
        self.prune_tilings();

        let active_removed = self.active_tab.is_some_and(|id| panes.contains(&id));
        if active_removed {
            // 网格仍在时回到网格焦点；网格已解除时回到原焦点组或相邻标签。
            let tiled_return = self
                .active_tiling()
                .and_then(|tiled| tiled.focused_group())
                .or(tiled_focus)
                .and_then(|id| self.groups.iter().find(|group| group.id == id))
                .map(|group| group.active_pane);
            self.active_tab = tiled_return.or_else(|| {
                self.groups
                    .get(index.saturating_sub(1))
                    .or_else(|| self.groups.first())
                    .map(|group| group.active_pane)
            });
            if let Some(active) = self.active_tab {
                self.activate_tab(active, window, cx);
            }
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
        if self.groups.is_empty() && self.settings.window.close_window_on_all_tabs_closed {
            self.close_app(window, cx);
            return;
        }
        cx.notify();
    }

    /// 关闭单个窗格；组内最后一个窗格关闭时等价于关闭整个标签。
    ///
    /// 窗格存在未保存的编辑面板时，关闭会被闸门挂起（同 `close_group`）。
    pub(crate) fn close_pane(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group_index) = self.group_index_for_tab(tab_id) else {
            return;
        };
        if self.groups[group_index].pane_count() == 1 {
            let group_id = self.groups[group_index].id;
            self.close_group(group_id, window, cx);
            return;
        }
        if self.block_close_for_dirty_editors(
            std::slice::from_ref(&tab_id),
            PendingEditorClose::Pane(tab_id),
            cx,
        ) {
            return;
        }

        self.send_command(AppCommand::CloseTab { tab_id });
        self.tabs.retain(|tab| tab.id != tab_id);
        self.prune_editor_panels();
        let replacement = self.detach_pane_from_group(tab_id);
        // 摘除后标签组可能整组消失，所在并排网格要跟着清理。
        self.prune_tilings();
        if let Some(next) = replacement {
            self.activate_tab(next, window, cx);
        }
        cx.notify();
    }

    /// 连接尚未建立时的静默移除，不向控制器发送关闭命令。
    pub(crate) fn remove_tab(&mut self, tab_id: TabId) {
        let was_active = self.active_tab == Some(tab_id);
        let group_id = self
            .group_index_for_tab(tab_id)
            .map(|index| self.groups[index].id);
        let tiled_focus = group_id
            .and_then(|group| self.tiling_of_group(group))
            .and_then(|tiled| tiled.focused_group());
        self.tabs.retain(|tab| tab.id != tab_id);
        self.prune_editor_panels();
        let replacement = self.detach_pane_from_group(tab_id);
        self.prune_tilings();
        if was_active {
            // 组内还有别的窗格时替换窗格优先；整组被摘掉时退到并排焦点或标签栏首项。
            self.active_tab = replacement
                .or_else(|| {
                    self.active_tiling()
                        .and_then(|tiled| tiled.focused_group())
                        .and_then(|group| {
                            self.groups
                                .iter()
                                .find(|item| item.id == group)
                                .map(|item| item.active_pane)
                        })
                })
                .or_else(|| {
                    tiled_focus.and_then(|group| {
                        self.groups
                            .iter()
                            .find(|item| item.id == group)
                            .map(|item| item.active_pane)
                    })
                })
                .or_else(|| self.groups.first().map(|group| group.active_pane));
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
    }

    /// 在标签栏条目之间轮转：并排网格算一条，组内成员不单独占位。
    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.strip_entries();
        if entries.len() <= 1 {
            return;
        }
        let current = self
            .active_group_id()
            .and_then(|group| {
                entries
                    .iter()
                    .position(|entry| self.entry_contains(*entry, group))
            })
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % entries.len()
        } else {
            (current + entries.len() - 1) % entries.len()
        };
        let target = match entries[next] {
            StripEntry::Single(group) => Some(group),
            StripEntry::Tiled { id, .. } => self
                .tiling_by_id(id)
                .and_then(|tiled| tiled.focused_group()),
        };
        if let Some(group) = target {
            self.activate_group(group, window, cx);
        }
    }

    pub(crate) fn toggle_language(&mut self, cx: &mut Context<Self>) {
        self.settings.language = if self.settings.language == "zh-CN" {
            "en".to_string()
        } else {
            "zh-CN".to_string()
        };
        set_locale(&self.settings.language);
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        }
        cx.notify();
    }
}

/// 计算副本命名基准：剥掉我们自己加上的"副本"后缀、以及紧随其后的 ` #N` 编号。
///
/// 只有确认剥到了"副本"后缀时才继续剥编号，避免误伤用户自己的命名：
/// `db #12` 会被复制成 `db #12 副本`，而不是被剪成 `db 副本`。
fn duplicate_base(name: &str) -> String {
    let suffix = rust_i18n::t!("common.duplicate_name", name = "").to_string();
    let trimmed = name.trim_end();
    if let Some(base) = strip_copy_suffix(trimmed, &suffix) {
        return base.to_string();
    }
    if let Some(position) = trimmed.rfind(" #")
        && let Some(tail) = trimmed.get(position + 2..)
        && !tail.is_empty()
        && tail.chars().all(|c| c.is_ascii_digit())
        && let Some(base) = strip_copy_suffix(trimmed[..position].trim_end(), &suffix)
    {
        return base.to_string();
    }
    trimmed.to_string()
}

/// 去掉本地化的"副本"后缀；不是我们生成的后缀就返回 None，不猜测用户命名。
fn strip_copy_suffix<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    text.strip_suffix(suffix).map(str::trim_end)
}

/// 会话表单保存时需要执行的钥匙串写操作。
enum SecretWrite {
    Set(SecretKey, Zeroizing<String>),
    Delete(SecretKey),
}

/// 顺序执行钥匙串写操作，返回最后一个错误（与此前同步逐个覆盖错误栏的次序一致）。
/// 删除操作的"钥匙串不可用"按既有约定视为成功，不产生错误。
/// 阻塞函数（走 D-Bus），仅允许在后台执行器线程池上调用。
fn apply_secret_writes(secret_store: &dyn SecretStore, writes: &[SecretWrite]) -> Option<AppError> {
    let mut last_error = None;
    for write in writes {
        let result = match write {
            SecretWrite::Set(key, value) => secret_store.set(key, value.as_str()),
            SecretWrite::Delete(key) => match secret_store.delete(key) {
                Err(error) if error.code == ErrorCode::SecretStoreUnavailable => Ok(()),
                other => other,
            },
        };
        if let Err(error) = result {
            last_error = Some(error);
        }
    }
    last_error
}

#[cfg(test)]
mod tests {
    use super::{duplicate_base, strip_copy_suffix};

    #[test]
    fn duplicate_base_strips_only_our_own_suffix() {
        let suffix = rust_i18n::t!("common.duplicate_name", name = "").to_string();
        // 单层副本与"副本叠副本"都回到原始名。
        assert_eq!(
            duplicate_base(&format!("web-01{suffix}")),
            "web-01".to_string()
        );
        assert_eq!(
            duplicate_base(&format!("web-01{suffix} #2")),
            "web-01".to_string()
        );
        // 用户自己的命名绝不改动：没有我们的后缀时，连尾部的 #N 也不剥。
        assert_eq!(duplicate_base("db #12"), "db #12".to_string());
        assert_eq!(duplicate_base("web-01"), "web-01".to_string());
        assert_eq!(
            duplicate_base("release #12 #13"),
            "release #12 #13".to_string()
        );
        // 只剥一次我们的后缀，不做递归猜测。
        assert_eq!(strip_copy_suffix(&format!("a{suffix}"), &suffix), Some("a"));
        assert_eq!(strip_copy_suffix("plain", &suffix), None);
    }
}
