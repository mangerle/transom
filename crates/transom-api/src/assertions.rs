//! API 响应断言执行。

use super::{ApiAssertionResult, ApiRequest, ApiResponseSnapshot};

/// 断言执行结果的简单聚合，供 UI 展示。
pub fn assertion_summary(results: &[ApiAssertionResult]) -> (usize, usize) {
    let passed = results.iter().filter(|result| result.passed).count();
    (passed, results.len())
}

/// 执行文本断言；响应体或 JSON 取值无法解析时以失败明细返回。
pub fn evaluate_assertions(
    request: &ApiRequest,
    response: &ApiResponseSnapshot,
) -> Vec<ApiAssertionResult> {
    let enabled = request
        .assertions
        .iter()
        .filter(|assertion| assertion.enabled)
        .collect::<Vec<_>>();
    let mut results = Vec::with_capacity(enabled.len());
    for assertion in enabled {
        let expression = assertion.expression.trim();
        let result = match evaluate_one(expression, response) {
            Ok(passed) => ApiAssertionResult {
                expression: expression.to_string(),
                passed,
                detail: if passed {
                    "通过".to_string()
                } else {
                    "不匹配".to_string()
                },
            },
            Err(detail) => ApiAssertionResult {
                expression: expression.to_string(),
                passed: false,
                detail,
            },
        };
        results.push(result);
    }
    results
}

fn evaluate_one(expression: &str, response: &ApiResponseSnapshot) -> Result<bool, String> {
    let lower = expression.to_lowercase();
    if let Some(rest) = lower.strip_prefix("status ") {
        if let Some(expected) = lower.split("!=").nth(1) {
            return Ok(response.status.to_string() != expected.trim());
        }
        let expected = rest.trim_start_matches("==").trim();
        let expected = expected
            .parse::<u16>()
            .map_err(|_| "状态码不是整数".to_string())?;
        return Ok(response.status == expected);
    }
    if let Some(rest) = lower.strip_prefix("body ") {
        return evaluate_contains(rest, &response.body);
    }
    if let Some(rest) = lower.strip_prefix("text ") {
        return evaluate_contains(rest, &response.body);
    }
    if let Some(rest) = lower.strip_prefix("header ") {
        let Some((name, expected)) = rest.split_once(" contains ") else {
            return Err("header 断言必须使用 contains".to_string());
        };
        let value = response
            .headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name.trim()))
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        return Ok(value.to_lowercase().contains(expected.trim()));
    }
    if let Some(rest) = lower.strip_prefix("json ") {
        if let Some((path, expected)) = rest.split_once(" ==") {
            let value = json_value(&response.body, path.trim())?;
            return Ok(value.eq_ignore_ascii_case(expected.trim()));
        }
        if let Some(path) = rest.strip_suffix(" exists") {
            return Ok(json_value(&response.body, path.trim()).is_ok());
        }
        return Err("json 断言必须使用 == 或 exists".to_string());
    }
    Err("不支持的断言语法".to_string())
}

fn evaluate_contains(rest: &str, body: &str) -> Result<bool, String> {
    let Some(expected) = rest.strip_prefix("contains ") else {
        return Err("body/text 断言必须使用 contains".to_string());
    };
    Ok(body.to_lowercase().contains(expected.trim()))
}

fn json_value(body: &str, path: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("响应体不是 JSON，原因: {error}"))?;
    let mut current = &value;
    for segment in path.split('.').filter(|segment| !segment.is_empty()) {
        current = current
            .get(segment)
            .ok_or_else(|| format!("JSON 路径不存在: {path}"))?;
    }
    if let Some(text) = current.as_str() {
        Ok(text.to_string())
    } else if let Some(bool_value) = current.as_bool() {
        Ok(bool_value.to_string())
    } else if let Some(number) = current.as_number() {
        Ok(number.to_string())
    } else if current.is_null() {
        Ok("null".to_string())
    } else {
        Ok(current.to_string())
    }
}
