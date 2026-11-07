use super::body::read_body_prefix;
use super::{percent_encode, read_body_bytes, read_body_text};
use bytes::Bytes;
use http_body_util::Full;

#[test]
fn percent_encoding_keeps_unreserved_and_encodes_separators() {
    assert_eq!(percent_encode("/var/lib"), "%2Fvar%2Flib");
    assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
    assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
    assert_eq!(percent_encode("a b"), "a%20b");
}

#[tokio::test]
async fn body_prefix_stops_at_limit_and_marks_truncation() {
    let body = Full::new(Bytes::from(vec![b'a'; 64]));
    let (prefix, truncated) = read_body_prefix(body, 16).await.expect("读取前缀必须成功");
    assert_eq!(prefix.len(), 16);
    assert!(truncated);
}

#[tokio::test]
async fn body_reads_reject_oversized_json_but_accept_exact_limit() {
    let oversized = Full::new(Bytes::from(vec![b'a'; 32]));
    let error = read_body_bytes(oversized, 16)
        .await
        .expect_err("超过上限必须报错");
    assert!(matches!(error, crate::error::ContainerError::Decode { .. }));

    let exact = Full::new(Bytes::from(vec![b'a'; 16]));
    let bytes = read_body_bytes(exact, 16)
        .await
        .expect("恰好达到上限必须允许");
    assert_eq!(bytes.len(), 16);

    let textual = Full::new(Bytes::from(vec![b'b'; 32]));
    let text = read_body_text(textual, 8).await;
    assert_eq!(text, "bbbbbbbb");
}
