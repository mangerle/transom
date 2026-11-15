//! SFTP 文件权限辅助：右键一键授予执行位。

/// 在现有权限上叠加属主/组/其他三方的执行位，其余位（含 setuid、sticky）原样保留。
///
/// 设计原理（Why）：
/// - 实现初衷：口径对齐 `chmod +x`。上传脚本后要跑，用户要的是"能执行"而不是"重写整串权限"，
///   所以只能在原值上或出执行位；
/// - 核心优势：直接写死 755 会把组写位、特殊位一并覆盖，`u+x` 又常在别人执行时失败，
///   按位叠加对两种误操作都免疫；服务端未回权限时退回目录 755、文件 644，
///   与 Chmod 弹窗打开时用的基线保持同一套约定；
/// - 代价与局限：这是"只加不减"，已经带执行位的条目再点一次结果不变，不提供撤销入口——
///   减位属于精细调整，交给 Chmod 弹窗。
pub(super) fn exec_granted_mode(current: Option<u32>, is_dir: bool) -> u32 {
    let baseline = current.unwrap_or(if is_dir { 0o755 } else { 0o644 }) & 0o7777;
    baseline | 0o111
}

#[cfg(test)]
mod tests {
    use super::exec_granted_mode;

    #[test]
    fn adds_exec_bits_without_touching_others() {
        assert_eq!(exec_granted_mode(Some(0o644), false), 0o755);
        assert_eq!(
            exec_granted_mode(Some(0o664), false),
            0o775,
            "组写位必须保留"
        );
        assert_eq!(
            exec_granted_mode(Some(0o600), false),
            0o711,
            "口径是 +x 而非 +X：没有读位的主体同样拿到执行位"
        );
    }

    #[test]
    fn keeps_special_bits_and_stays_idempotent() {
        assert_eq!(
            exec_granted_mode(Some(0o4755), false),
            0o4755,
            "setuid 位不应被抹掉"
        );
        assert_eq!(exec_granted_mode(Some(0o2644), false), 0o2755);
        assert_eq!(
            exec_granted_mode(Some(0o755), false),
            0o755,
            "重复点击结果不变"
        );
    }

    #[test]
    fn falls_back_to_chmod_modal_baseline_when_server_omits_permissions() {
        assert_eq!(exec_granted_mode(None, true), 0o755);
        assert_eq!(
            exec_granted_mode(None, false),
            0o755,
            "文件按 644 打底再或上执行位"
        );
    }

    #[test]
    fn masks_type_bits_out_of_incoming_mode() {
        // 本地路径传入的是 st_mode（含 0o100000 文件类型位），叠加后只保留权限位。
        assert_eq!(exec_granted_mode(Some(0o100644), false), 0o755);
        assert_eq!(exec_granted_mode(Some(0o40755), true), 0o755);
    }
}
