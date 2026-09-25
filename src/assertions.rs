//! Evaluates declarative tests against a response.

use crate::http::Response;
use crate::model::{AssertOp, Assertion};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

/// Navigates a JSON value by a path such as `data.items[0].id` or `[2].name`.
pub fn json_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    let path = path.trim().trim_start_matches('$').trim_start_matches('.');
    if path.is_empty() {
        return Some(cur);
    }
    for segment in path.split('.') {
        let (name, rest) = match segment.find('[') {
            Some(i) => (&segment[..i], &segment[i..]),
            None => (segment, ""),
        };
        if !name.is_empty() {
            cur = cur.get(name)?;
        }
        let mut rest = rest;
        while let Some(stripped) = rest.strip_prefix('[') {
            let end = stripped.find(']')?;
            let idx: usize = stripped[..end].trim().parse().ok()?;
            cur = cur.get(idx)?;
            rest = &stripped[end + 1..];
        }
    }
    Some(cur)
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Extracts the actual value for an assertion source.
pub fn extract(source: &str, resp: &Response) -> Option<String> {
    let source = source.trim();
    if source == "status" {
        return Some(resp.status.to_string());
    }
    if source == "time" {
        return Some(resp.elapsed.as_millis().to_string());
    }
    if source == "body" {
        return Some(resp.text());
    }
    if let Some(h) = source.strip_prefix("header.") {
        return resp.header(h);
    }
    if let Some(p) = source.strip_prefix("json") {
        let root: Value = serde_json::from_slice(&resp.body).ok()?;
        return json_path(&root, p).map(value_to_string);
    }
    None
}

pub fn run(tests: &[Assertion], resp: &Response) -> Vec<TestResult> {
    tests
        .iter()
        .filter(|t| t.enabled && !t.source.trim().is_empty())
        .map(|t| {
            let actual = extract(&t.source, resp);
            let name = if t.op == AssertOp::Exists {
                format!("{} exists", t.source)
            } else {
                format!("{} {} {}", t.source, t.op.label(), t.expected)
            };
            let (passed, message) = evaluate(t, actual.as_deref());
            TestResult { name, passed, message }
        })
        .collect()
}

fn evaluate(t: &Assertion, actual: Option<&str>) -> (bool, String) {
    let Some(actual) = actual else {
        return (false, format!("{} not found", t.source));
    };
    let exp = t.expected.as_str();
    let num = |s: &str| s.trim().parse::<f64>().ok();
    let passed = match t.op {
        AssertOp::Exists => true,
        AssertOp::Equals => actual == exp || matches!((num(actual), num(exp)), (Some(a), Some(b)) if a == b),
        AssertOp::NotEquals => actual != exp,
        AssertOp::Contains => actual.contains(exp),
        AssertOp::NotContains => !actual.contains(exp),
        AssertOp::LessThan => matches!((num(actual), num(exp)), (Some(a), Some(b)) if a < b),
        AssertOp::GreaterThan => matches!((num(actual), num(exp)), (Some(a), Some(b)) if a > b),
        AssertOp::Matches => regex::Regex::new(exp).map(|r| r.is_match(actual)).unwrap_or(false),
    };
    let shown: String = actual.chars().take(120).collect();
    (passed, if passed { String::new() } else { format!("actual: {shown}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let v: Value = serde_json::json!({"data": {"items": [{"id": 7}]}, "list": [1, [2, 3]]});
        assert_eq!(json_path(&v, "data.items[0].id"), Some(&serde_json::json!(7)));
        assert_eq!(json_path(&v, ".list[1][0]"), Some(&serde_json::json!(2)));
        assert_eq!(json_path(&v, "$.nope"), None);
    }
}
