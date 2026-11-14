use super::*;

#[test]
fn port_bindings_group_by_container_port_and_protocol() {
    let bindings = port_bindings(&[
        transom_core::ContainerPortPublish {
            host: 8080,
            container: 80,
            protocol: "tcp".to_string(),
        },
        transom_core::ContainerPortPublish {
            host: 5353,
            container: 53,
            protocol: "udp".to_string(),
        },
    ]);
    assert_eq!(bindings["80/tcp"][0]["HostPort"], serde_json::json!("8080"));
    assert_eq!(bindings["53/udp"][0]["HostPort"], serde_json::json!("5353"));
    assert!(
        port_bindings(&[])
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
    );
}
