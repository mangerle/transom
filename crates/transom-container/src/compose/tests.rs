use super::*;
use std::path::PathBuf;
use transom_core::TlsConfig;

#[test]
fn test_endpoint_cli_args_mapping() {
    let unix = ContainerEndpoint::Unix {
        path: PathBuf::from("/custom/docker.sock"),
    };
    assert_eq!(
        endpoint_cli_args(&unix),
        vec!["-H", "unix:///custom/docker.sock"]
    );

    let tcp = ContainerEndpoint::Tcp {
        url: "tcp://192.168.1.50:2376".to_string(),
        tls: Some(TlsConfig {
            ca_cert: "/ca.pem".to_string(),
            client_cert: "/cert.pem".to_string(),
            client_key: "/key.pem".to_string(),
        }),
    };
    let args = endpoint_cli_args(&tcp);
    assert!(args.contains(&"-H".to_string()));
    assert!(args.contains(&"tcp://192.168.1.50:2376".to_string()));
    assert!(args.contains(&"--tlsverify".to_string()));
    assert!(args.contains(&"/ca.pem".to_string()));

    let ssh = ContainerEndpoint::Ssh {
        host: "server.com".to_string(),
        port: Some(2222),
        user: Some("deploy".to_string()),
        identity_file: None,
        password: None,
        use_sudo: false,
        sudo_password: None,
    };
    assert_eq!(
        endpoint_cli_args(&ssh),
        vec!["-H", "ssh://deploy@server.com:2222"]
    );
}

#[test]
fn test_build_args_with_remote_and_local_dirs() {
    let project = ComposeProject {
        name: "web_app".to_string(),
        working_dir: Some("/non/existent/remote/dir".to_string()),
        config_files: vec!["docker-compose.yml".to_string()],
        running: 2,
        total: 2,
    };

    let endpoint_args = vec!["-H".to_string(), "tcp://remote:2375".to_string()];

    // 远端且本地无目录：down 操作不带 -f
    let down_args = build_args(
        "docker",
        &project,
        ComposeAction::Down,
        &endpoint_args,
        &[],
        None,
    );
    assert_eq!(
        down_args,
        vec![
            "-H",
            "tcp://remote:2375",
            "compose",
            "-p",
            "web_app",
            "down"
        ]
    );

    // 本地有目录：包含 -f
    let up_args = build_args(
        "docker",
        &project,
        ComposeAction::Up,
        &endpoint_args,
        &[],
        Some("/local/path"),
    );
    assert_eq!(
        up_args,
        vec![
            "-H",
            "tcp://remote:2375",
            "compose",
            "-p",
            "web_app",
            "-f",
            "docker-compose.yml",
            "up",
            "-d"
        ]
    );
}

#[test]
fn wsl_endpoint_runs_compose_inside_distro() {
    let (program, prefix) = compose_program(
        &ContainerEndpoint::Wsl {
            distro: "Ubuntu-24.04".to_string(),
        },
        transom_core::ContainerEngineKind::Docker,
    );
    assert_eq!(program, "wsl.exe");
    assert_eq!(prefix, vec!["-d", "Ubuntu-24.04", "-e", "docker"]);
}

#[test]
fn test_validate_project_rules() {
    let remote_project = ComposeProject {
        name: "test".to_string(),
        working_dir: Some("/remote/dir/not/here".to_string()),
        config_files: vec!["compose.yml".to_string()],
        running: 1,
        total: 1,
    };

    // Down 在远端即使无本地目录也允许通过
    assert!(validate_project(&remote_project, ComposeAction::Down, true).is_ok());

    // Up 在远端且无本地目录时明确返回错误
    assert!(validate_project(&remote_project, ComposeAction::Up, true).is_err());
}
