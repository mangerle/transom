//! 已知主机指纹弹窗实体：记录快照、搜索框与删除动作都归它。
//!
//! # 设计原理（Why）
//! 弹窗状态的生命周期与窗口一致，但动作（删除记录、刷新快照）只作用于这份
//! 状态本身，没有理由再挂到外壳上。独立为 `Entity<KnownHostsView>` 后，外壳
//! 只持有句柄，打开/关闭/删除都经实体 `update`；状态变更只重绘本实体。
//!
//! # 代价与局限
//! 删除成功后的状态栏提示由实体事件上报外壳，多一次事件派发；实体不持有
//! `AppShell` 句柄，避免双向引用与双重租借。
use super::state::KnownHostsModalState;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{AppError, HostKeyRecord, KnownHostsStore};

/// 已知主机弹窗对外上报的意图。
pub(crate) enum KnownHostsEvent {
    /// 删除一条记录的结果；`error` 为空表示成功，由外壳决定提示文案。
    ///
    /// 事件只携带结果：调用方不需要知道删的是哪一条，记录快照已由实体
    /// 自己刷新；把 host/port 也带上会让事件消费方误以为需要自行定位。
    Removed { error: Option<AppError> },
}

/// 已知主机指纹管理弹窗实体。
pub(crate) struct KnownHostsView {
    /// 记录快照与搜索框。
    state: KnownHostsModalState,
    /// 记录读写入口；实体自己完成删除与刷新，不经过外壳。
    store: Arc<dyn KnownHostsStore>,
}

impl EventEmitter<KnownHostsEvent> for KnownHostsView {}

impl KnownHostsView {
    /// 组装弹窗实体；启动时传入一次记录快照，避免首帧空列表闪一下。
    pub(crate) fn new(
        store: Arc<dyn KnownHostsStore>,
        records: Vec<HostKeyRecord>,
        search: Entity<InputState>,
    ) -> Self {
        Self {
            state: KnownHostsModalState {
                open: false,
                records,
                search,
            },
            store,
        }
    }

    /// 弹窗是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.open
    }

    /// 打开弹窗并刷新记录：期间可能在别处新增了信任记录。
    pub(crate) fn open(&mut self, cx: &mut Context<Self>) {
        self.state.open = true;
        self.refresh();
        cx.notify();
    }

    /// 关闭弹窗。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.state.open = false;
        cx.notify();
    }

    /// 从存储重新读取记录快照。
    pub(crate) fn refresh(&mut self) {
        self.state.records = self.store.list().unwrap_or_default();
    }

    /// 删除一条记录；结果通过事件交给外壳提示，失败时保留原快照。
    pub(crate) fn remove(&mut self, host: &str, port: u16, cx: &mut Context<Self>) {
        let error = self.store.remove(host, port).err();
        if error.is_none() {
            self.refresh();
        }
        cx.emit(KnownHostsEvent::Removed { error });
        cx.notify();
    }

    /// 记录快照的只读访问：渲染与搜索都读这里。
    pub(crate) fn records(&self) -> &[HostKeyRecord] {
        &self.state.records
    }

    /// 搜索框实体。
    pub(crate) fn search(&self) -> &Entity<InputState> {
        &self.state.search
    }
}
