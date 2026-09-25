//! Import of OpenAPI 3.x and Swagger 2.0 specs into a collection.
//!
//! - one folder per tag (untagged operations go to the collection root)
//! - `{param}` path segments become `:param` path variables
//! - request bodies get an example generated from the schema
//! - security schemes become collection / request auth using `{{variables}}`

use crate::model::*;
use anyhow::bail;
use serde_json::{Map, Value, json};

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
const MAX_DEPTH: usize = 8;

pub fn is_spec(v: &Value) -> bool {
    (v.get("openapi").is_some() || v.get("swagger").is_some()) && v.get("paths").is_some()
}

/// Key identifying an operation across re-imports.
pub fn operation_key(method: &str, path: &str) -> String {
    format!("{} {}", method.to_ascii_uppercase(), path)
}

struct Spec<'a> {
    root: &'a Value,
    swagger2: bool,
}

impl<'a> Spec<'a> {
    /// Follows a local `$ref` (`#/components/schemas/X`, `#/definitions/X`, ...).
    fn resolve(&self, v: &'a Value) -> &'a Value {
        let mut cur = v;
        for _ in 0..16 {
            match cur.get("$ref").and_then(|r| r.as_str()) {
                Some(r) if r.starts_with('#') => match self.root.pointer(&r[1..].replace("~1", "/")) {
                    Some(target) => cur = target,
                    None => return cur,
                },
                _ => return cur,
            }
        }
        cur
    }

    /// Builds an example value for a schema.
    fn example(&self, schema: &Value, depth: usize) -> Value {
        let schema = self.resolve(schema);
        if depth > MAX_DEPTH {
            return Value::Null;
        }
        for key in ["example", "default", "const"] {
            if let Some(v) = schema.get(key) {
                return v.clone();
            }
        }
        if let Some(Value::Array(ex)) = schema.get("examples") {
            if let Some(first) = ex.first() {
                return first.clone();
            }
        }
        if let Some(Value::Array(e)) = schema.get("enum") {
            if let Some(first) = e.first() {
                return first.clone();
            }
        }
        if let Some(Value::Array(all)) = schema.get("allOf") {
            let mut merged = Map::new();
            for part in all {
                match self.example(part, depth + 1) {
                    Value::Object(o) => merged.extend(o),
                    other if all.len() == 1 => return other,
                    _ => {}
                }
            }
            return Value::Object(merged);
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(Value::Array(opts)) = schema.get(key) {
                if let Some(first) = opts.iter().find(|o| self.resolve(o).get("type").and_then(|t| t.as_str()) != Some("null")) {
                    return self.example(first, depth + 1);
                }
            }
        }
        let ty = match schema.get("type") {
            Some(Value::String(t)) => t.as_str(),
            // OpenAPI 3.1: ["string", "null"]
            Some(Value::Array(ts)) => ts.iter().filter_map(|t| t.as_str()).find(|t| *t != "null").unwrap_or("null"),
            _ if schema.get("properties").is_some() => "object",
            _ if schema.get("items").is_some() => "array",
            _ => "",
        };
        match ty {
            "object" => {
                let mut out = Map::new();
                if let Some(Value::Object(props)) = schema.get("properties") {
                    for (name, prop) in props {
                        if self.resolve(prop).get("readOnly").and_then(|r| r.as_bool()) == Some(true) {
                            continue;
                        }
                        out.insert(name.clone(), self.example(prop, depth + 1));
                    }
                }
                Value::Object(out)
            }
            "array" => match schema.get("items") {
                Some(items) => json!([self.example(items, depth + 1)]),
                None => json!([]),
            },
            "integer" => json!(schema.get("minimum").and_then(|m| m.as_i64()).unwrap_or(0)),
            "number" => schema.get("minimum").cloned().unwrap_or(json!(0)),
            "boolean" => json!(true),
            "string" => json!(string_example(schema.get("format").and_then(|f| f.as_str()).unwrap_or(""))),
            _ => Value::Null,
        }
    }

    /// Example for a parameter, as a string (empty when nothing sensible is known).
    fn param_example(&self, p: &Value) -> String {
        let direct = p.get("example").or_else(|| p.get("default"));
        let from_schema = || {
            let schema = if self.swagger2 { p } else { p.get("schema")? };
            let schema = self.resolve(schema);
            schema.get("example").or_else(|| schema.get("default")).or_else(|| schema.get("enum").and_then(|e| e.get(0))).cloned()
        };
        match direct.cloned().or_else(from_schema) {
            Some(Value::String(s)) => s,
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        }
    }
}

fn string_example(format: &str) -> String {
    match format {
        "email" => "user@example.com".into(),
        "date-time" => chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "date" => chrono::Utc::now().format("%Y-%m-%d").to_string(),
        "time" => "12:00:00".into(),
        "uuid" => "00000000-0000-0000-0000-000000000000".into(),
        "uri" | "url" => "https://example.com".into(),
        "hostname" => "example.com".into(),
        "ipv4" => "127.0.0.1".into(),
        "password" => "password".into(),
        "binary" | "byte" => String::new(),
        _ => "string".into(),
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(|s| s.as_str()).unwrap_or("").to_string()
}

/// Base URL for requests: first server (with variables filled in) or Swagger 2 host/basePath,
/// resolved against the URL the spec was fetched from when relative.
fn base_url(spec: &Value, swagger2: bool, source: Option<&str>) -> String {
    let source_origin = source.and_then(|s| url::Url::parse(s).ok()).map(|u| u.origin().ascii_serialization()).filter(|o| o != "null");
    let raw = if swagger2 {
        let base_path = text(spec, "basePath");
        match spec.get("host").and_then(|h| h.as_str()) {
            Some(host) => {
                let scheme = spec.pointer("/schemes/0").and_then(|s| s.as_str()).unwrap_or("https");
                format!("{scheme}://{host}{base_path}")
            }
            None => base_path,
        }
    } else {
        match spec.pointer("/servers/0") {
            Some(server) => {
                let mut url = text(server, "url");
                if let Some(Value::Object(vars)) = server.get("variables") {
                    for (name, var) in vars {
                        url = url.replace(&format!("{{{name}}}"), &text(var, "default"));
                    }
                }
                url
            }
            None => String::new(),
        }
    };
    let raw = raw.trim_end_matches('/').to_string();
    if raw.contains("://") {
        raw
    } else {
        format!("{}{}", source_origin.unwrap_or_else(|| "http://localhost".into()), raw)
    }
}

/// Maps a security scheme to auth using variables the user fills in per environment.
fn scheme_auth(scheme: &Value) -> (Auth, Vec<&'static str>) {
    match (text(scheme, "type").as_str(), text(scheme, "scheme").to_ascii_lowercase().as_str()) {
        ("http", "basic") | ("basic", _) => {
            (Auth::Basic { username: "{{username}}".into(), password: "{{password}}".into() }, vec!["username", "password"])
        }
        ("http", _) => (Auth::Bearer { token: "{{bearerToken}}".into() }, vec!["bearerToken"]),
        ("apiKey", _) => {
            let location = if text(scheme, "in") == "query" { ApiKeyLocation::Query } else { ApiKeyLocation::Header };
            if text(scheme, "in") == "cookie" {
                return (Auth::Inherit, vec![]);
            }
            (Auth::ApiKey { key: text(scheme, "name"), value: "{{apiKey}}".into(), location }, vec!["apiKey"])
        }
        ("oauth2", _) | ("openIdConnect", _) => (Auth::Bearer { token: "{{accessToken}}".into() }, vec!["accessToken"]),
        _ => (Auth::Inherit, vec![]),
    }
}

pub fn import(spec: &Value, source: Option<&str>) -> anyhow::Result<Collection> {
    let swagger2 = spec.get("swagger").is_some();
    let s = Spec { root: spec, swagger2 };
    let Some(Value::Object(paths)) = spec.get("paths") else { bail!("spec has no paths") };

    let info = spec.get("info").cloned().unwrap_or(Value::Null);
    let mut collection = Collection::new(match text(&info, "title") {
        t if t.is_empty() => "Imported API".to_string(),
        t => t,
    });
    collection.description = text(&info, "description");
    collection.openapi_source = source.map(String::from);

    let schemes = if swagger2 { spec.get("securityDefinitions") } else { spec.pointer("/components/securitySchemes") };
    let scheme_for = |name: &str| schemes.and_then(|s| s.get(name)).map(|v| s.resolve(v));
    let mut auth_vars: Vec<&'static str> = vec![];

    // collection auth from the global security requirement
    let global_scheme = spec.pointer("/security/0").and_then(|r| r.as_object()).and_then(|o| o.keys().next().cloned());
    collection.auth = match global_scheme.as_deref().and_then(scheme_for) {
        Some(scheme) => {
            let (auth, vars) = scheme_auth(scheme);
            auth_vars.extend(vars);
            auth
        }
        None => Auth::Inherit,
    };

    // spec tags: order and descriptions
    let tag_list: Vec<(String, String)> = spec
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|tags| tags.iter().map(|t| (text(t, "name"), text(t, "description"))).collect())
        .unwrap_or_default();

    let mut ops: Vec<Op> = vec![];
    for (path, item) in paths {
        let item = s.resolve(item);
        let shared_params: Vec<&Value> = item.get("parameters").and_then(|p| p.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            let mut req = build_request(&s, method, path, op, &shared_params);

            // per-operation security overrides
            if let Some(Value::Array(reqs)) = op.get("security") {
                let first = reqs.first().and_then(|r| r.as_object()).and_then(|o| o.keys().next().cloned());
                req.auth = match first {
                    None => Auth::None, // `security: []` or `[{}]` = public endpoint
                    Some(name) if Some(&name) == global_scheme.as_ref() => Auth::Inherit,
                    Some(name) => match scheme_for(&name) {
                        Some(scheme) => {
                            let (auth, vars) = scheme_auth(scheme);
                            auth_vars.extend(vars);
                            auth
                        }
                        None => Auth::Inherit,
                    },
                };
            }
            ops.push(Op { path: path.clone(), tag: op.pointer("/tags/0").and_then(|t| t.as_str()).map(String::from), req });
        }
    }

    collection.items = build_tree(ops, &tag_list);
    collection.variables.push(KeyValue {
        enabled: true,
        key: "baseUrl".into(),
        value: base_url(spec, swagger2, source),
        description: "Server URL from the OpenAPI spec".into(),
    });
    auth_vars.sort();
    auth_vars.dedup();
    for var in auth_vars {
        collection.variables.push(KeyValue {
            enabled: true,
            key: var.into(),
            value: String::new(),
            description: "Credential used by the imported auth. Tip: define it in an environment instead.".into(),
        });
    }
    Ok(collection)
}

struct Op {
    path: String,
    tag: Option<String>,
    req: Request,
}

fn static_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|seg| !seg.is_empty()).take_while(|seg| !seg.starts_with('{')).collect()
}

/// "customer-portal" -> "Customer Portal"
fn title_case(segment: &str) -> String {
    segment
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "Customer Clinics" under audience "Customer" -> "Clinics"; `None` when the tag is the audience itself.
fn strip_audience(tag: &str, audience: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let aud = audience.to_lowercase();
    if lower == aud {
        return None;
    }
    for sep in [" ", "-", "_", ": ", " - "] {
        let prefix = format!("{aud}{sep}");
        if lower.starts_with(&prefix) {
            let rest = tag[prefix.len()..].trim();
            return if rest.is_empty() { None } else { Some(rest.to_string()) };
        }
    }
    Some(tag.to_string())
}

/// Arranges operations into folders.
///
/// When the API is split by audience (e.g. `/admin/...`, `/customer/...`, each spanning several
/// tags), the first path segment becomes a top-level folder with one subfolder per tag.
/// Otherwise operations are grouped by tag only. Untagged operations stay at their level's root.
fn build_tree(ops: Vec<Op>, tag_list: &[(String, String)]) -> Vec<Item> {
    use std::collections::{HashMap, HashSet};

    // skip leading segments every path shares (e.g. /api/v1)
    let all: Vec<Vec<String>> = ops.iter().map(|o| static_segments(&o.path).into_iter().map(String::from).collect()).collect();
    let mut common = 0;
    if all.len() > 1 {
        while let Some(seg) = all[0].get(common) {
            if all.iter().all(|a| a.len() > common + 1 && a.get(common) == Some(seg)) {
                common += 1;
            } else {
                break;
            }
        }
    }
    let audience_of = |i: usize| all[i].get(common).cloned();

    // an audience qualifies when its operations span several tags
    let mut tags_per_audience: HashMap<String, HashSet<String>> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        if let (Some(aud), Some(tag)) = (audience_of(i), &op.tag) {
            tags_per_audience.entry(aud).or_default().insert(tag.clone());
        }
    }
    let qualifying: HashSet<String> = tags_per_audience.iter().filter(|(_, t)| t.len() >= 2).map(|(a, _)| a.clone()).collect();
    let use_audiences = qualifying.len() >= 2;

    let tag_rank = |tag: &str| tag_list.iter().position(|(n, _)| n == tag).unwrap_or(usize::MAX / 2);
    let tag_desc = |tag: &str| tag_list.iter().find(|(n, _)| n == tag).map(|(_, d)| d.clone()).unwrap_or_default();

    let mut root: Vec<Item> = vec![];
    let mut ranks: HashMap<String, usize> = HashMap::new();
    let mut untagged_rank = tag_list.len() + 1_000;
    for (i, op) in ops.into_iter().enumerate() {
        let rank = match &op.tag {
            Some(t) => tag_rank(t),
            None => {
                untagged_rank += 1;
                untagged_rank
            }
        };
        // folder path: [(name, description)]
        let mut path: Vec<(String, String)> = vec![];
        match (use_audiences.then(|| audience_of(i)).flatten().filter(|a| qualifying.contains(a)), &op.tag) {
            (Some(aud), tag) => {
                let aud_title = title_case(&aud);
                path.push((aud_title.clone(), String::new()));
                if let Some(sub) = tag.as_ref().and_then(|t| strip_audience(t, &aud_title)) {
                    path.push((sub, tag.as_deref().map(tag_desc).unwrap_or_default()));
                }
            }
            (None, Some(tag)) => path.push((tag.clone(), tag_desc(tag))),
            (None, None) => {}
        }
        let folder_ids = insert_at_path(&mut root, &path, Item::Request(op.req));
        for id in folder_ids {
            let r = ranks.entry(id).or_insert(rank);
            *r = (*r).min(rank);
        }
    }
    sort_items(&mut root, &ranks);
    root
}

/// Inserts `item` under the folder path (creating folders as needed). Returns the ids of the
/// folders along the path.
fn insert_at_path(items: &mut Vec<Item>, path: &[(String, String)], item: Item) -> Vec<String> {
    let Some(((name, desc), rest)) = path.split_first() else {
        items.push(item);
        return vec![];
    };
    let pos = match items.iter().position(|i| matches!(i, Item::Folder(f) if &f.name == name)) {
        Some(p) => p,
        None => {
            let mut f = Folder::new(name.clone());
            f.description = desc.clone();
            items.push(Item::Folder(f));
            items.len() - 1
        }
    };
    let Item::Folder(folder) = &mut items[pos] else { unreachable!() };
    let mut ids = vec![folder.id.clone()];
    ids.extend(insert_at_path(&mut folder.items, rest, item));
    ids
}

/// Folders by rank (spec tag order), requests after folders in their original order.
fn sort_items(items: &mut [Item], ranks: &std::collections::HashMap<String, usize>) {
    items.sort_by_key(|i| match i {
        Item::Folder(f) => ranks.get(&f.id).copied().unwrap_or(usize::MAX - 1),
        Item::Request(_) => usize::MAX,
    });
    for i in items.iter_mut() {
        if let Item::Folder(f) = i {
            sort_items(&mut f.items, ranks);
        }
    }
}

fn build_request(s: &Spec, method: &str, path: &str, op: &Value, shared: &[&Value]) -> Request {
    // operation parameters override path-level ones with the same name+location
    let mut params: Vec<&Value> = vec![];
    for p in shared.iter().copied().chain(op.get("parameters").and_then(|p| p.as_array()).into_iter().flatten()) {
        let p = s.resolve(p);
        params.retain(|q| !(text(q, "name") == text(p, "name") && text(q, "in") == text(p, "in")));
        params.push(p);
    }

    let mut req = Request {
        name: {
            let summary = text(op, "summary");
            let base = if !summary.is_empty() {
                summary
            } else if !text(op, "operationId").is_empty() {
                text(op, "operationId")
            } else {
                format!("{} {}", method.to_ascii_uppercase(), path)
            };
            if op.get("deprecated").and_then(|d| d.as_bool()) == Some(true) { format!("{base} (deprecated)") } else { base }
        },
        method: Method::parse(method),
        auth: Auth::Inherit,
        operation: Some(operation_key(method, path)),
        ..Default::default()
    };
    let mut description = text(op, "description");
    if description.is_empty() && req.name != text(op, "summary") {
        description = text(op, "summary");
    }

    // `{id}` -> `:id`
    let mut url_path = String::new();
    for segment in path.split('/') {
        if !url_path.is_empty() || !segment.is_empty() {
            url_path.push('/');
        }
        match segment.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
            Some(name) => {
                url_path.push(':');
                url_path.push_str(name);
            }
            None => url_path.push_str(segment),
        }
    }
    if url_path.is_empty() {
        url_path.push('/');
    }

    let mut form_fields: Vec<FormField> = vec![];
    let mut form_urlencoded = false;
    let mut docs: Vec<String> = vec![];
    for p in &params {
        let name = text(p, "name");
        let value = s.param_example(p);
        let required = p.get("required").and_then(|r| r.as_bool()).unwrap_or(false);
        let desc = text(p, "description");
        if !desc.is_empty() {
            docs.push(format!("- `{name}` ({}){}: {desc}", text(p, "in"), if required { ", required" } else { "" }));
        }
        let kv = KeyValue { enabled: required || !value.is_empty(), key: name.clone(), value: value.clone(), description: desc };
        match text(p, "in").as_str() {
            "path" => req.path_vars.push(KeyValue { enabled: true, ..kv }),
            "query" => req.params.push(kv),
            "header" => {
                if !["accept", "content-type", "authorization"].contains(&name.to_ascii_lowercase().as_str()) {
                    req.headers.push(kv);
                }
            }
            "formData" => {
                if text(p, "in") == "formData" && text(p, "type") != "file" {
                    form_urlencoded = true;
                }
                form_fields.push(FormField { enabled: true, key: name, value, is_file: text(p, "type") == "file" });
            }
            "body" => {
                if let Some(schema) = p.get("schema") {
                    let example = s.example(schema, 0);
                    req.body = Body::Raw { language: RawLanguage::Json, content: serde_json::to_string_pretty(&example).unwrap_or_default() };
                }
            }
            _ => {}
        }
    }

    // Swagger 2 form parameters
    if !form_fields.is_empty() {
        let has_file = form_fields.iter().any(|f| f.is_file);
        let consumes_multipart = op.get("consumes").and_then(|c| c.as_array()).is_some_and(|c| c.iter().any(|x| x.as_str() == Some("multipart/form-data")));
        req.body = if has_file || consumes_multipart || !form_urlencoded {
            Body::FormData { fields: form_fields }
        } else {
            Body::UrlEncoded { fields: form_fields.into_iter().map(|f| KeyValue { enabled: f.enabled, key: f.key, value: f.value, description: String::new() }).collect() }
        };
    }

    // OpenAPI 3 request body
    if let Some(body) = op.get("requestBody").map(|b| s.resolve(b)) {
        if let Some(Value::Object(content)) = body.get("content") {
            let pick = ["application/json", "application/x-www-form-urlencoded", "multipart/form-data"]
                .iter()
                .find_map(|ct| content.get_key_value(*ct))
                .or_else(|| content.iter().find(|(ct, _)| ct.contains("json")))
                .or_else(|| content.iter().next());
            if let Some((ct, media)) = pick {
                req.body = media_body(s, ct, media);
                if !ct.contains("json") && !ct.contains("form") && !matches!(req.body, Body::Binary { .. }) {
                    req.headers.push(KeyValue::new("Content-Type", ct.clone()));
                }
            }
        }
    }

    if !docs.is_empty() {
        description = format!("{description}\n\nParameters:\n{}", docs.join("\n")).trim().to_string();
    }
    req.description = description;
    req.url = format!("{{{{baseUrl}}}}{url_path}");
    req.url = crate::ui::util::with_query(&req.url, &req.params);
    req
}

fn media_body(s: &Spec, content_type: &str, media: &Value) -> Body {
    let schema = media.get("schema").map(|sc| s.resolve(sc)).unwrap_or(&Value::Null);
    let example = media
        .get("example")
        .cloned()
        .or_else(|| media.get("examples").and_then(|e| e.as_object()).and_then(|e| e.values().next()).map(|e| s.resolve(e)).and_then(|e| e.get("value").cloned()))
        .unwrap_or_else(|| s.example(schema, 0));

    let fields_from_schema = || -> Vec<(String, String, bool)> {
        let props = schema.get("properties").and_then(|p| p.as_object()).cloned().unwrap_or_default();
        props
            .iter()
            .map(|(name, prop)| {
                let prop = s.resolve(prop);
                let is_file = text(prop, "format") == "binary" || text(prop, "type") == "file";
                let value = match example.get(name) {
                    Some(Value::String(v)) => v.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(v) => v.to_string(),
                };
                (name.clone(), if is_file { String::new() } else { value }, is_file)
            })
            .collect()
    };

    if content_type.contains("json") {
        Body::Raw { language: RawLanguage::Json, content: serde_json::to_string_pretty(&example).unwrap_or_default() }
    } else if content_type == "application/x-www-form-urlencoded" {
        Body::UrlEncoded { fields: fields_from_schema().into_iter().map(|(k, v, _)| KeyValue::new(k, v)).collect() }
    } else if content_type.starts_with("multipart/") {
        Body::FormData { fields: fields_from_schema().into_iter().map(|(key, value, is_file)| FormField { enabled: true, key, value, is_file }).collect() }
    } else if content_type.contains("xml") {
        Body::Raw { language: RawLanguage::Xml, content: String::new() }
    } else if content_type.starts_with("text/") {
        Body::Raw { language: RawLanguage::Text, content: example.as_str().unwrap_or("").to_string() }
    } else {
        Body::Binary { path: String::new() }
    }
}

/// Collects operation keys present in a tree.
pub fn operation_keys(items: &[Item], out: &mut std::collections::HashSet<String>) {
    for i in items {
        match i {
            Item::Request(r) => {
                if let Some(k) = &r.operation {
                    out.insert(k.clone());
                }
            }
            Item::Folder(f) => operation_keys(&f.items, out),
        }
    }
}

/// Adds operations from `fresh` that `existing` doesn't have yet, placing them in the folder
/// with the same name path (created if missing) and leaving everything the user already has
/// (and any edits) untouched. Returns (added, no longer in spec).
pub fn sync(existing: &mut Collection, fresh: Collection) -> (usize, usize) {
    let mut have = std::collections::HashSet::new();
    operation_keys(&existing.items, &mut have);
    let mut in_spec = std::collections::HashSet::new();
    operation_keys(&fresh.items, &mut in_spec);

    fn walk(items: Vec<Item>, path: &mut Vec<(String, String)>, have: &std::collections::HashSet<String>, target: &mut Vec<Item>, added: &mut usize) {
        for item in items {
            match item {
                Item::Request(r) => {
                    if r.operation.as_ref().is_some_and(|k| !have.contains(k)) {
                        insert_at_path(target, path, Item::Request(r));
                        *added += 1;
                    }
                }
                Item::Folder(f) => {
                    path.push((f.name, f.description));
                    walk(f.items, path, have, target, added);
                    path.pop();
                }
            }
        }
    }
    let mut added = 0;
    walk(fresh.items, &mut vec![], &have, &mut existing.items, &mut added);

    // variables the user doesn't have yet (e.g. a new auth scheme)
    for var in fresh.variables {
        if !existing.variables.iter().any(|v| v.key == var.key) {
            existing.variables.push(var);
        }
    }
    let removed = have.difference(&in_spec).count();
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Value {
        json!({
            "openapi": "3.0.3",
            "info": {"title": "Pets"},
            "servers": [{"url": "/api/{v}", "variables": {"v": {"default": "v1"}}}],
            "security": [{"bearerAuth": []}],
            "components": {
                "securitySchemes": {"bearerAuth": {"type": "http", "scheme": "bearer"}, "key": {"type": "apiKey", "in": "header", "name": "X-Key"}},
                "schemas": {"Pet": {"type": "object", "required": ["name"], "properties": {
                    "id": {"type": "integer", "readOnly": true},
                    "name": {"type": "string", "example": "Rex"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "owner": {"allOf": [{"$ref": "#/components/schemas/Owner"}]}
                }}, "Owner": {"type": "object", "properties": {"email": {"type": "string", "format": "email"}}}}
            },
            "tags": [{"name": "pets", "description": "Pet ops"}],
            "paths": {
                "/pets/{petId}": {
                    "parameters": [{"name": "petId", "in": "path", "required": true, "schema": {"type": "integer", "example": 7}}],
                    "get": {"tags": ["pets"], "summary": "Get pet", "parameters": [{"name": "full", "in": "query", "schema": {"type": "boolean"}}]},
                    "put": {"tags": ["pets"], "operationId": "updatePet", "security": [{"key": []}],
                        "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pet"}}}}}
                },
                "/health": {"get": {"security": []}}
            }
        })
    }

    #[test]
    fn imports_openapi3() {
        let c = import(&spec(), Some("http://localhost:3000/docs/json")).unwrap();
        assert_eq!(c.name, "Pets");
        assert_eq!(c.auth, Auth::Bearer { token: "{{bearerToken}}".into() });
        assert_eq!(c.variables[0].value, "http://localhost:3000/api/v1");
        let Item::Folder(pets) = &c.items[0] else { panic!() };
        assert_eq!(pets.name, "pets");
        let Item::Request(get) = &pets.items[0] else { panic!() };
        assert_eq!(get.url, "{{baseUrl}}/pets/:petId");
        assert_eq!(get.path_vars[0].value, "7");
        assert!(!get.params[0].enabled);
        let Item::Request(put) = &pets.items[1] else { panic!() };
        assert_eq!(put.name, "updatePet");
        assert!(matches!(put.auth, Auth::ApiKey { ref key, .. } if key == "X-Key"));
        let Body::Raw { content, .. } = &put.body else { panic!() };
        let body: Value = serde_json::from_str(content).unwrap();
        assert_eq!(body, json!({"name": "Rex", "tags": ["string"], "owner": {"email": "user@example.com"}}));
        let Item::Request(health) = &c.items[1] else { panic!() };
        assert_eq!(health.auth, Auth::None);
        assert_eq!(health.operation.as_deref(), Some("GET /health"));
    }

    #[test]
    fn sync_adds_only_new() {
        let mut existing = import(&spec(), None).unwrap();
        if let Item::Folder(f) = &mut existing.items[0] {
            f.items.remove(1); // user deleted PUT
            if let Item::Request(r) = &mut f.items[0] {
                r.name = "My edit".into();
            }
        }
        existing.items.pop(); // and /health
        let (added, removed) = sync(&mut existing, import(&spec(), None).unwrap());
        assert_eq!((added, removed), (2, 0));
        let Item::Folder(f) = &existing.items[0] else { panic!() };
        assert_eq!(f.items.len(), 2);
        assert!(matches!(&f.items[0], Item::Request(r) if r.name == "My edit"));
    }

    #[test]
    fn groups_by_audience() {
        let op = |tag: &str| json!({"get": {"tags": [tag]}});
        let v = json!({"openapi": "3.0.0", "info": {"title": "Omega"},
            "tags": [{"name": "Account"}, {"name": "Clinics", "description": "Admin clinics"}, {"name": "Patients"}, {"name": "Customer Clinics"}, {"name": "Customer Patients"}],
            "paths": {
                "/api/health": {"get": {}},
                "/api/customer/clinics": op("Customer Clinics"),
                "/api/customer/patients/{id}": op("Customer Patients"),
                "/api/admin/clinics": op("Clinics"),
                "/api/admin/patients": op("Patients"),
                "/api/account/login": op("Account"),
            }});
        let c = import(&v, None).unwrap();
        let names = |items: &[Item]| items.iter().map(|i| i.name().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&c.items), ["Account", "Admin", "Customer", "GET /api/health"]);
        let Item::Folder(admin) = &c.items[1] else { panic!() };
        assert_eq!(names(&admin.items), ["Clinics", "Patients"]);
        let Item::Folder(clinics) = &admin.items[0] else { panic!() };
        assert_eq!(clinics.description, "Admin clinics");
        let Item::Folder(customer) = &c.items[2] else { panic!() };
        assert_eq!(names(&customer.items), ["Clinics", "Patients"]);

        // sync puts a new customer endpoint into Customer/Clinics
        let mut existing = c.clone();
        if let Item::Folder(cust) = &mut existing.items[2] {
            if let Item::Folder(cl) = &mut cust.items[0] {
                cl.items.clear();
            }
        }
        let (added, _) = sync(&mut existing, c);
        assert_eq!(added, 1);
        let Item::Folder(cust) = &existing.items[2] else { panic!() };
        assert_eq!(cust.items.len(), 2);
        let Item::Folder(cl) = &cust.items[0] else { panic!() };
        assert_eq!(cl.items.len(), 1);
    }

    #[test]
    fn plain_api_groups_by_tag_only() {
        let v = json!({"openapi": "3.0.0", "info": {"title": "Pets"}, "paths": {
            "/pets": {"get": {"tags": ["pets"]}}, "/pets/{id}": {"get": {"tags": ["pets"]}}, "/users": {"get": {"tags": ["users"]}}}});
        let c = import(&v, None).unwrap();
        assert_eq!(c.items.iter().map(|i| i.name()).collect::<Vec<_>>(), ["pets", "users"]);
    }

    #[test]
    fn swagger2() {
        let v = json!({"swagger": "2.0", "info": {"title": "Old"}, "host": "api.x.io", "basePath": "/v2", "schemes": ["https"],
            "paths": {"/upload": {"post": {"consumes": ["multipart/form-data"], "parameters": [
                {"name": "file", "in": "formData", "type": "file"}, {"name": "note", "in": "formData", "type": "string"}]}}}});
        let c = import(&v, None).unwrap();
        assert_eq!(c.variables[0].value, "https://api.x.io/v2");
        let Item::Request(r) = &c.items[0] else { panic!() };
        assert!(matches!(&r.body, Body::FormData { fields } if fields[0].is_file && fields.len() == 2));
    }
}
