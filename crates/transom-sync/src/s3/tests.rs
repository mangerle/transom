use super::*;

fn target() -> S3Target {
    S3Target::new(
        reqwest::Client::new(),
        S3TargetOptions {
            endpoint: "https://s3.example.com:9000",
            region: "us-east-1",
            bucket: "transom-vault",
            access_key: "AKIDEXAMPLE",
            secret_key: Zeroizing::new("secret".to_string()),
            session_token: None,
            directory: "backups",
            virtual_host_style: false,
        },
    )
    .unwrap()
}

#[test]
fn object_url_uses_path_style_addressing() {
    let (url, canonical_uri, host) = target().object_url().expect("对象地址应可解析");
    assert_eq!(
        url.as_str(),
        "https://s3.example.com:9000/transom-vault/backups/catalog.transombak"
    );
    assert_eq!(canonical_uri, "/transom-vault/backups/catalog.transombak");
    assert_eq!(host, "s3.example.com:9000");
}

#[test]
fn invalid_bucket_is_rejected_instead_of_panicking() {
    for bucket in ["", "bad bucket", "bucket?x", "bucket#frag", "a/b"] {
        let error = S3Target::new(
            reqwest::Client::new(),
            S3TargetOptions {
                endpoint: "https://s3.example.com",
                region: "us-east-1",
                bucket,
                access_key: "AKID",
                secret_key: Zeroizing::new("secret".to_string()),
                session_token: None,
                directory: "t",
                virtual_host_style: false,
            },
        )
        .err()
        .expect("非法桶名应在构造期失败");
        assert_eq!(error.code, ErrorCode::Configuration);
    }
}

#[test]
fn host_header_keeps_nonstandard_port() {
    let (_, _, host) = target().object_url().unwrap();
    assert_eq!(host, "s3.example.com:9000");

    let standard = S3Target::new(
        reqwest::Client::new(),
        S3TargetOptions {
            endpoint: "https://s3.amazonaws.com",
            region: "us-east-1",
            bucket: "bucket",
            access_key: "AKID",
            secret_key: Zeroizing::new("secret".to_string()),
            session_token: None,
            directory: "t",
            virtual_host_style: false,
        },
    )
    .unwrap();
    let (_, _, host) = standard.object_url().unwrap();
    assert_eq!(host, "s3.amazonaws.com");
}

#[test]
fn virtual_host_style_moves_bucket_into_subdomain() {
    let target = S3Target::new(
        reqwest::Client::new(),
        S3TargetOptions {
            endpoint: "https://s3.amazonaws.com",
            region: "us-east-1",
            bucket: "transom-vault",
            access_key: "AKID",
            secret_key: Zeroizing::new("secret".to_string()),
            session_token: None,
            directory: "backups",
            virtual_host_style: true,
        },
    )
    .unwrap();
    let (url, canonical_uri, host) = target.object_url().unwrap();
    assert_eq!(
        url.as_str(),
        "https://transom-vault.s3.amazonaws.com/backups/catalog.transombak"
    );
    assert_eq!(canonical_uri, "/backups/catalog.transombak");
    assert_eq!(host, "transom-vault.s3.amazonaws.com");
}

#[test]
fn insecure_and_non_https_endpoints_are_rejected() {
    for endpoint in ["http://s3.example.com", "ftp://s3.example.com"] {
        let error = S3Target::new(
            reqwest::Client::new(),
            S3TargetOptions {
                endpoint,
                region: "us-east-1",
                bucket: "bucket",
                access_key: "AKID",
                secret_key: Zeroizing::new("secret".to_string()),
                session_token: None,
                directory: "t",
                virtual_host_style: false,
            },
        )
        .err()
        .expect("预期构造失败");
        assert_eq!(error.code, ErrorCode::Configuration);
    }
}
