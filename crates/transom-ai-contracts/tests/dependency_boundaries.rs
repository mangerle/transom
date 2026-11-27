//! AI 契约层与存储层的依赖边界测试。

#[test]
fn store_depends_on_contracts_instead_of_runtime() {
    let store_manifest = include_str!("../../transom-store/Cargo.toml");
    assert!(store_manifest.contains("transom-ai-contracts.workspace = true"));
    assert!(
        !store_manifest
            .lines()
            .any(|line| line.trim() == "transom-ai.workspace = true")
    );
}

#[test]
fn contracts_do_not_depend_on_runtime_or_transport_crates() {
    let contracts_manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "rig-core",
        "rig-agent",
        "reqwest",
        "tokio",
        "image",
        "similar",
    ] {
        assert!(
            !contracts_manifest
                .lines()
                .any(|line| line.trim_start().starts_with(forbidden)),
            "AI 契约层不得依赖运行时或传输 crate: {forbidden}"
        );
    }
}
