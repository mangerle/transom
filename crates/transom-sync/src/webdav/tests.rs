use super::digest::{DigestAlgorithm, DigestChallenge, parse_digest_challenge};
use super::*;

fn target(base: &str, directory: &str) -> Result<WebDavTarget, AppError> {
    WebDavTarget::new(
        reqwest::Client::new(),
        base,
        "user@example.com",
        Some("app-password"),
        directory,
    )
}

#[test]
fn object_and_directory_urls_are_joined_level_by_level() {
    let target = target("https://dav.example.com/remote.php/dav/files/user", "a/b").unwrap();
    assert_eq!(
        target.object_url.as_str(),
        "https://dav.example.com/remote.php/dav/files/user/a/b/catalog.transombak"
    );
    assert_eq!(
        target
            .directory_urls
            .iter()
            .map(reqwest::Url::as_str)
            .collect::<Vec<_>>(),
        vec![
            "https://dav.example.com/remote.php/dav/files/user/a/",
            "https://dav.example.com/remote.php/dav/files/user/a/b/"
        ]
    );
}

#[test]
fn base_query_and_fragment_are_dropped() {
    let target = target(
        "https://dav.example.com/root?token=leak#fragment",
        "transom",
    )
    .unwrap();
    let url = target.object_url.as_str();
    assert!(!url.contains("token=leak"));
    assert!(!url.contains("#fragment"));
}

#[test]
fn insecure_and_non_https_schemes_are_rejected() {
    for base in ["http://dav.example.com", "ftp://dav.example.com"] {
        assert!(target(base, "t").is_err());
    }
}

#[test]
fn malformed_url_is_rejected() {
    assert!(target("not a url", "t").is_err());
}

#[test]
fn basic_authorization_carries_encoded_credentials() {
    let target = target("https://dav.example.com", "t").unwrap();
    let header = target
        .authorization(&reqwest::Method::GET, &target.object_url, false)
        .unwrap()
        .unwrap();
    assert_eq!(header, "Basic dXNlckBleGFtcGxlLmNvbTphcHAtcGFzc3dvcmQ=");
}

#[test]
fn anonymous_endpoints_skip_authorization() {
    let target = WebDavTarget::new(
        reqwest::Client::new(),
        "https://dav.example.com",
        "",
        None,
        "t",
    )
    .unwrap();
    assert!(
        target
            .authorization(&reqwest::Method::GET, &target.object_url, false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn digest_challenge_parses_quoted_qop_and_opaque() {
    let challenge = parse_digest_challenge(
        r#"Digest realm="dav", nonce="abc,123", algorithm=SHA-256, qop="auth,auth-int", opaque="opaque\"" "#,
    )
    .unwrap();
    assert_eq!(challenge.realm, "dav");
    assert_eq!(challenge.nonce, "abc,123");
    assert_eq!(challenge.algorithm, DigestAlgorithm::Sha256);
    assert_eq!(challenge.qop.as_deref(), Some("auth"));
    assert_eq!(challenge.opaque.as_deref(), Some("opaque\""));
}

#[test]
fn digest_authorization_is_deterministic_for_fixed_inputs() {
    let challenge = DigestChallenge {
        realm: "testrealm@host.com".into(),
        nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".into(),
        algorithm: DigestAlgorithm::Md5,
        qop: Some("auth".into()),
        opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".into()),
    };
    let header = challenge
        .authorization(
            "GET",
            "/dir/index.html",
            "Mufasa",
            "Circle Of Life",
            1,
            "0a4f113b",
        )
        .unwrap();
    assert!(header.starts_with("Digest username=\"Mufasa\""));
    assert!(header.contains("response=\"6629fae49393a05397450978507c4ef1\""));
    assert!(header.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
}
