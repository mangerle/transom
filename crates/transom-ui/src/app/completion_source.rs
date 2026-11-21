//! 终端输入补全的候选来源：汇集命令历史与用户片段，缓存候选池。
//!
//! 设计原理（Why）：
//! - 补全在每次按键时查询，直接重建几百条候选（每条三个小写字符串）是热路径上
//!   纯浪费的分配；这里按池内容指纹缓存构建结果，历史与片段任一变化才重建，
//!   指纹只哈希字段字节、不分配，按键路径上先比指纹再决定是否重建；
//! - 终端视图在 UI 线程按键路径上同步调用 [`CompletionSource::complete`]，
//!   因此这里全部读内存快照，不触碰磁盘、不加长临界区；
//! - 片段与历史开关的镜像由界面修改后显式调 [`CompletionSource::apply_settings`]
//!   同步进来，与命令历史共用同一份内存快照容器，变更点收敛且无后台轮询。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};
use transom_core::{AppSettings, CompletionItem, CompletionPool, Snippet};

/// 补全候选来源：按设置汇集历史命令与用户片段，并缓存构建好的候选池。
pub(crate) struct CompletionSource {
    /// 命令历史内存快照：与 `RuntimeState.history` / 设置页共用同一容器，
    /// 历史收录后无需额外同步，指纹比对自动让缓存失效。
    history: Arc<RwLock<Vec<String>>>,
    /// 用户片段镜像：设置修改后经 [`CompletionSource::apply_settings`] 同步。
    snippets: RwLock<Vec<Snippet>>,
    /// 历史总开关镜像：关闭后不再弹任何候选（与"不再采集也不再弹建议"口径一致）。
    history_enabled: AtomicBool,
    /// (指纹, 候选池) 缓存：指纹不符才重建。
    cache: Mutex<Option<(u64, CompletionPool)>>,
}

impl CompletionSource {
    /// 以既有历史快照为数据源创建；片段与开关初始为空/关，需随后 `apply_settings`。
    pub(crate) fn new(history: Arc<RwLock<Vec<String>>>) -> Self {
        Self {
            history,
            snippets: RwLock::new(Vec::new()),
            history_enabled: AtomicBool::new(false),
            cache: Mutex::new(None),
        }
    }

    /// 从设置同步片段与历史开关：片段或开关变化后必须调用，否则候选池过期。
    pub(crate) fn apply_settings(&self, settings: &AppSettings) {
        *self.snippets.write() = settings.snippets.items.clone();
        self.history_enabled
            .store(settings.history.enabled, Ordering::Relaxed);
    }

    /// 按正在输入的词与前文返回排好序的候选；词位判定与打分在 `transom-core`。
    pub(crate) fn complete(&self, token: &str, before_token: &str) -> Vec<CompletionItem> {
        if !self.history_enabled.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let history = self.history.read();
        let snippets = self.snippets.read();
        let fingerprint = CompletionPool::fingerprint(&history, &snippets);
        let mut cache = self.cache.lock();
        if cache.as_ref().is_none_or(|(key, _)| *key != fingerprint) {
            *cache = Some((fingerprint, CompletionPool::build(&history, &snippets)));
        }
        match cache.as_ref() {
            Some((_, pool)) => pool.complete(token, before_token),
            None => Vec::new(),
        }
    }
}
