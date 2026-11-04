//! 容器引擎系统级指标与磁盘用量分析模型。

use serde::{Deserialize, Serialize};

/// 全局磁盘空间占用分析（对应 Docker `system df`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSystemDiskUsage {
    /// 镜像合计占用大小（字节）。
    #[serde(default)]
    pub images_size: u64,
    /// 镜像可回收大小（字节）。
    #[serde(default)]
    pub images_reclaimable: u64,
    /// 镜像总数。
    #[serde(default)]
    pub images_count: usize,
    /// 活动容器可写层合计占用（字节）。
    #[serde(default)]
    pub containers_size: u64,
    /// 容器可回收大小（字节）。
    #[serde(default)]
    pub containers_reclaimable: u64,
    /// 容器总数。
    #[serde(default)]
    pub containers_count: usize,
    /// 本地数据卷合计占用（字节）。
    #[serde(default)]
    pub volumes_size: u64,
    /// 数据卷可回收大小（字节）。
    #[serde(default)]
    pub volumes_reclaimable: u64,
    /// 数据卷总数。
    #[serde(default)]
    pub volumes_count: usize,
    /// 构建缓存占用大小（字节）。
    #[serde(default)]
    pub build_cache_size: u64,
    /// 构建缓存可回收大小（字节）。
    #[serde(default)]
    pub build_cache_reclaimable: u64,
    /// 构建缓存条目数。
    #[serde(default)]
    pub build_cache_count: usize,
    /// 总占用空间（字节）。
    #[serde(default)]
    pub total_size: u64,
    /// 总可回收空间（字节）。
    #[serde(default)]
    pub total_reclaimable: u64,
}

impl ContainerSystemDiskUsage {
    /// 重新计算汇总总大小与总可回收大小。
    pub fn recalculate_totals(&mut self) {
        self.total_size = self
            .images_size
            .saturating_add(self.containers_size)
            .saturating_add(self.volumes_size)
            .saturating_add(self.build_cache_size);
        self.total_reclaimable = self
            .images_reclaimable
            .saturating_add(self.containers_reclaimable)
            .saturating_add(self.volumes_reclaimable)
            .saturating_add(self.build_cache_reclaimable);
    }
}

#[cfg(test)]
mod tests {
    use super::ContainerSystemDiskUsage;

    #[test]
    fn recalculates_totals_with_saturating_add() {
        let mut usage = ContainerSystemDiskUsage {
            images_size: 100,
            images_reclaimable: 20,
            images_count: 2,
            containers_size: 50,
            containers_reclaimable: 10,
            containers_count: 1,
            volumes_size: 200,
            volumes_reclaimable: 50,
            volumes_count: 3,
            build_cache_size: 300,
            build_cache_reclaimable: 150,
            build_cache_count: 5,
            ..Default::default()
        };
        usage.recalculate_totals();
        assert_eq!(usage.total_size, 650);
        assert_eq!(usage.total_reclaimable, 230);
    }
}
