use super::actions::parse_build_args;

#[test]
fn build_args_parse_comma_and_newline_forms() {
    let args = parse_build_args("HTTP_PROXY=http://proxy,NPM_TOKEN=abc\nEMPTY=").expect("解析成功");
    assert_eq!(
        args.get("HTTP_PROXY").map(String::as_str),
        Some("http://proxy")
    );
    assert_eq!(args.get("NPM_TOKEN").map(String::as_str), Some("abc"));
    assert_eq!(args.get("EMPTY").map(String::as_str), Some(""));
}

#[test]
fn build_args_reject_missing_equals_and_empty_key() {
    assert!(parse_build_args("NO_EQUALS").is_err());
    assert!(parse_build_args("=value").is_err());
}
