/// 镜像层瘦身与最佳实践建议诊断结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageSlimmingAdvice {
    pub(crate) is_warning: bool,
    pub(crate) title: String,
    pub(crate) suggestion: String,
}

/// 针对单层镜像指令与增量体积进行静态启发式分析，产出针对性的瘦身与最佳实践建议。
///
/// ### 设计初衷 (Why)
/// 容器开发者在编写 Dockerfile 时常因疏忽遗留包管理器下载缓存或未剥离构建产物，
/// 导致镜像体积膨胀数十至数百兆。通过在前端直接分析构建历史，即时给出可落地的瘦身优化方案。
///
/// ### 核心优势
/// 纯本地静态匹配启发式规则（包管理器缓存、推荐依赖、大层预警），零网络开销即可辅助开发者快速定位臃肿根源。
///
/// ### 代价与局限
/// 依赖构建历史中的原始指令文本；若镜像被打平（squash）或元数据被外部工具剥离，部分规则将自动静默退化。
pub(crate) fn inspect_layer_slimming(
    cmd_raw: &str,
    layer_size: u64,
    total_image_size: u64,
) -> Vec<ImageSlimmingAdvice> {
    let mut advice = Vec::new();
    let lower = cmd_raw.to_ascii_lowercase();

    // 1. 单层大文件体积检查 (超过 100MB 或超过总镜像大小的 40%)
    if layer_size >= 100 * 1024 * 1024
        || (total_image_size > 0
            && layer_size * 100 / total_image_size >= 40
            && layer_size > 30 * 1024 * 1024)
    {
        advice.push(ImageSlimmingAdvice {
            is_warning: true,
            title: "大体积镜像层".to_string(),
            suggestion: format!(
                "本层增量达 {}，建议检查是否打包了非必要的大型二进制、测试资源或未清理的构建依赖，可考虑多阶段构建或只复制产物。",
                crate::format::format_size(layer_size)
            ),
        });
    }

    // 2. APT 缓存未清理
    if lower.contains("apt-get install") || lower.contains("apt install") {
        if !lower.contains("rm -rf /var/lib/apt/lists") && !lower.contains("rm -rf /var/cache/apt")
        {
            advice.push(ImageSlimmingAdvice {
                is_warning: true,
                title: "APT 索引缓存残留".to_string(),
                suggestion: "检测到 apt 安装指令但未清理索引，建议在同一 RUN 指令末尾追加 `&& rm -rf /var/lib/apt/lists/*` 降低镜像体积。".to_string(),
            });
        }
        if !lower.contains("--no-install-recommends") {
            advice.push(ImageSlimmingAdvice {
                is_warning: false,
                title: "包含非必要推荐包".to_string(),
                suggestion: "建议为 apt-get install 添加 `--no-install-recommends` 参数，避免安装文档与额外推荐包。".to_string(),
            });
        }
    }

    // 3. Alpine APK 缓存未跳过
    if lower.contains("apk add") && !lower.contains("--no-cache") {
        advice.push(ImageSlimmingAdvice {
            is_warning: true,
            title: "Alpine APK 缓存未跳过".to_string(),
            suggestion:
                "建议使用 `apk add --no-cache` 代替常规安装，避免生成 `/var/cache/apk/*` 临时索引。"
                    .to_string(),
        });
    }

    // 4. Python Pip 缓存
    if lower.contains("pip install") && !lower.contains("--no-cache-dir") {
        advice.push(ImageSlimmingAdvice {
            is_warning: true,
            title: "Python Pip 缓存残留".to_string(),
            suggestion:
                "建议为 pip install 添加 `--no-cache-dir` 参数，避免将 wheel 和下载缓存写入只读层。"
                    .to_string(),
        });
    }

    // 5. NPM/Yarn 缓存
    if (lower.contains("npm install") || lower.contains("npm i "))
        && !lower.contains("cache clean")
        && layer_size > 20 * 1024 * 1024
    {
        advice.push(ImageSlimmingAdvice {
            is_warning: false,
            title: "Node 模块缓存建议清理".to_string(),
            suggestion:
                "建议在依赖安装后执行 `npm cache clean --force` 并使用 `--omit=dev` 剔除开发依赖。"
                    .to_string(),
        });
    }

    advice
}

#[cfg(test)]
mod tests {
    use super::inspect_layer_slimming;

    #[test]
    fn test_inspect_layer_slimming_apt_cache() {
        let cmd = "RUN apt-get update && apt-get install -y curl git";
        let advices = inspect_layer_slimming(cmd, 30 * 1024 * 1024, 100 * 1024 * 1024);
        assert!(advices.iter().any(|a| a.title == "APT 索引缓存残留"));
        assert!(advices.iter().any(|a| a.title == "包含非必要推荐包"));
    }

    #[test]
    fn test_inspect_layer_slimming_large_layer() {
        let cmd = "COPY big_model.bin /app/";
        let advices = inspect_layer_slimming(cmd, 150 * 1024 * 1024, 200 * 1024 * 1024);
        assert!(advices.iter().any(|a| a.title == "大体积镜像层"));
    }

    #[test]
    fn test_inspect_layer_slimming_clean_pip() {
        let cmd = "RUN pip install --no-cache-dir -r requirements.txt";
        let advices = inspect_layer_slimming(cmd, 10 * 1024 * 1024, 200 * 1024 * 1024);
        assert!(advices.is_empty());
    }
}
