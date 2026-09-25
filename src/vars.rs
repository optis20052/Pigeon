//! `{{variable}}` substitution and dynamic variables.

use crate::model::KeyValue;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

static VAR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").unwrap());

/// Variable scope, resolved in priority order: environment > collection > globals.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    map: HashMap<String, String>,
}

impl Scope {
    pub fn new(globals: &[KeyValue], collection: &[KeyValue], environment: &[KeyValue]) -> Self {
        let mut map = HashMap::new();
        for layer in [globals, collection, environment] {
            for kv in layer.iter().filter(|kv| kv.enabled && !kv.key.is_empty()) {
                map.insert(kv.key.clone(), kv.value.clone());
            }
        }
        Self { map }
    }

    pub fn get(&self, name: &str) -> Option<String> {
        if let Some(v) = self.map.get(name) {
            return Some(v.clone());
        }
        dynamic(name)
    }

    /// Replaces all `{{name}}` occurrences. Unknown variables are left untouched.
    /// Substitution is repeated a few times so variables may reference other variables.
    pub fn apply(&self, input: &str) -> String {
        let mut out = input.to_string();
        for _ in 0..5 {
            if !out.contains("{{") {
                break;
            }
            let next = VAR_RE
                .replace_all(&out, |caps: &regex::Captures| {
                    self.get(&caps[1]).unwrap_or_else(|| caps[0].to_string())
                })
                .into_owned();
            if next == out {
                break;
            }
            out = next;
        }
        out
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

/// Dynamic variables (`{{$guid}}`, `{{$timestamp}}`, ...).
fn dynamic(name: &str) -> Option<String> {
    let now = chrono::Utc::now();
    Some(match name {
        "$guid" | "$randomUUID" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => now.timestamp().to_string(),
        "$isoTimestamp" => now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "$randomInt" => (uuid::Uuid::new_v4().as_u128() % 1001).to_string(),
        "$randomBoolean" => (uuid::Uuid::new_v4().as_u128() % 2 == 0).to_string(),
        _ => return None,
    })
}

static PATH_VAR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/:([A-Za-z_][A-Za-z0-9_-]*)").unwrap());

/// Byte length of the part of `url` that can hold path variables (before `?` / `#`).
fn path_end(url: &str) -> usize {
    url.find(['?', '#']).unwrap_or(url.len())
}

/// `:name` path variables in `url` with the byte range of `:name`.
pub fn path_vars(url: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let end = path_end(url);
    PATH_VAR_RE
        .captures_iter(&url[..end])
        .map(|c| {
            let m = c.get(0).unwrap();
            (m.start() + 1..m.end(), c[1].to_string())
        })
        .collect()
}

/// Replaces `:name` path segments with their values; unknown names are left as-is.
pub fn apply_path_vars(url: &str, values: &[(String, String)]) -> String {
    let end = path_end(url);
    let path = PATH_VAR_RE.replace_all(&url[..end], |c: &regex::Captures| match values.iter().find(|(k, _)| k == &c[1]) {
        Some((_, v)) if !v.is_empty() => format!("/{v}"),
        _ => c[0].to_string(),
    });
    format!("{path}{}", &url[end..])
}

/// Returns all variable names referenced in `text` with their byte ranges.
pub fn references(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    VAR_RE
        .captures_iter(text)
        .map(|c| (c.get(0).unwrap().range(), c[1].to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_with_priority() {
        let g = vec![KeyValue::new("host", "global"), KeyValue::new("a", "1")];
        let e = vec![KeyValue::new("host", "env")];
        let s = Scope::new(&g, &[], &e);
        assert_eq!(s.apply("{{host}}/{{ a }}/{{missing}}"), "env/1/{{missing}}");
    }

    #[test]
    fn path_variables() {
        let url = "http://localhost:3000/users/:id/files/:file_id?x=:nope";
        let names: Vec<String> = path_vars(url).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, ["id", "file_id"]);
        let vals = vec![("id".to_string(), "7".to_string())];
        assert_eq!(apply_path_vars(url, &vals), "http://localhost:3000/users/7/files/:file_id?x=:nope");
    }

    #[test]
    fn nested() {
        let g = vec![KeyValue::new("base", "http://{{host}}"), KeyValue::new("host", "x")];
        assert_eq!(Scope::new(&g, &[], &[]).apply("{{base}}/p"), "http://x/p");
    }
}
