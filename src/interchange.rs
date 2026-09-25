//! Import/export of collection (v2.0 / v2.1 JSON) and environment files,
//! plus parsing of cURL commands.

use crate::model::*;
use anyhow::bail;
use serde_json::{Value, json};

// ---------------------------------------------------------------- import

pub enum Imported {
    Collection(Collection),
    Environment(Environment),
}

/// Imports a collection/environment (v2.x JSON), an OpenAPI / Swagger spec (JSON or YAML),
/// or a Pigeon collection. `source` is the URL or path it came from, if any.
pub fn import(text: &str, source: Option<&str>) -> anyhow::Result<Imported> {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(json_err) => match serde_yaml_ng::from_str::<Value>(text) {
            Ok(v) if v.is_object() => v,
            _ => return Err(anyhow::anyhow!("not valid JSON or YAML ({json_err})")),
        },
    };
    if crate::openapi::is_spec(&v) {
        return Ok(Imported::Collection(crate::openapi::import(&v, source)?));
    }
    if v.get("info").is_some() && v.get("item").is_some() {
        return Ok(Imported::Collection(import_collection(&v)));
    }
    if v.get("values").is_some() {
        return Ok(Imported::Environment(import_environment(&v)));
    }
    // Pigeon's own native collection format
    if let Ok(c) = serde_json::from_value::<Collection>(v.clone()) {
        if !c.name.is_empty() {
            let mut c = c;
            c.id = new_id();
            return Ok(Imported::Collection(c));
        }
    }
    bail!("unrecognized file: expected a collection, environment or OpenAPI spec")
}

fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn description(v: &Value) -> String {
    match v.get("description") {
        Some(Value::String(s)) => s.clone(),
        Some(obj) => s(obj, "content"),
        None => String::new(),
    }
}

fn kvs(v: Option<&Value>) -> Vec<KeyValue> {
    v.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| KeyValue {
                    enabled: !e.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false)
                        && e.get("enabled").and_then(|d| d.as_bool()).unwrap_or(true),
                    key: s(e, "key"),
                    value: s(e, "value"),
                    description: description(e),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn import_environment(v: &Value) -> Environment {
    Environment { id: new_id(), name: s(v, "name"), variables: kvs(v.get("values")) }
}

fn import_collection(v: &Value) -> Collection {
    let info = &v["info"];
    Collection {
        id: new_id(),
        name: s(info, "name"),
        items: import_items(v.get("item")),
        auth: v.get("auth").map(import_auth).unwrap_or(Auth::Inherit),
        variables: kvs(v.get("variable")),
        description: description(info),
        openapi_source: None,
        color: None,
    }
}

fn import_items(v: Option<&Value>) -> Vec<Item> {
    let Some(arr) = v.and_then(|v| v.as_array()) else { return vec![] };
    arr.iter()
        .map(|it| {
            if it.get("item").is_some() {
                Item::Folder(Folder {
                    id: new_id(),
                    name: s(it, "name"),
                    items: import_items(it.get("item")),
                    auth: it.get("auth").map(import_auth).unwrap_or(Auth::Inherit),
                    description: description(it),
                    color: None,
                })
            } else {
                Item::Request(import_request(it))
            }
        })
        .collect()
}

/// Reads an auth parameter from either the v2.1 array form or the v2.0 object form.
fn auth_param(section: &Value, key: &str) -> String {
    match section {
        Value::Array(arr) => arr.iter().find(|e| s(e, "key") == key).map(|e| s(e, "value")).unwrap_or_default(),
        Value::Object(_) => s(section, key),
        _ => String::new(),
    }
}

fn import_auth(v: &Value) -> Auth {
    let ty = s(v, "type");
    let section = v.get(&ty).cloned().unwrap_or(Value::Null);
    match ty.as_str() {
        "noauth" => Auth::None,
        "bearer" => Auth::Bearer { token: auth_param(&section, "token") },
        "basic" => Auth::Basic { username: auth_param(&section, "username"), password: auth_param(&section, "password") },
        "apikey" => Auth::ApiKey {
            key: auth_param(&section, "key"),
            value: auth_param(&section, "value"),
            location: if auth_param(&section, "in") == "query" { ApiKeyLocation::Query } else { ApiKeyLocation::Header },
        },
        _ => Auth::Inherit,
    }
}

fn import_request(it: &Value) -> Request {
    let r = it.get("request").cloned().unwrap_or(Value::Null);
    // A request may be just a URL string.
    if let Value::String(url) = &r {
        return Request { name: s(it, "name"), url: url.clone(), ..Default::default() };
    }
    let url = match r.get("url") {
        Some(Value::String(u)) => u.clone(),
        Some(obj) => s(obj, "raw"),
        None => String::new(),
    };
    let params = match r.get("url") {
        Some(obj @ Value::Object(_)) => kvs(obj.get("query")),
        _ => crate::ui::util::parse_query(&url),
    };
    let path_vars = match r.get("url") {
        Some(obj @ Value::Object(_)) => kvs(obj.get("variable")),
        _ => vec![],
    };
    let body = r.get("body").map(import_body).unwrap_or_default();
    Request {
        id: new_id(),
        name: s(it, "name"),
        method: Method::parse(&s(&r, "method")),
        url,
        params,
        path_vars,
        headers: kvs(r.get("header")),
        body,
        auth: r.get("auth").map(import_auth).unwrap_or(Auth::Inherit),
        settings: RequestSettings::default(),
        description: description(&r),
        tests: vec![],
        operation: None,
    }
}

fn import_body(b: &Value) -> Body {
    match s(b, "mode").as_str() {
        "raw" => {
            let lang = b.pointer("/options/raw/language").and_then(|l| l.as_str()).unwrap_or("text");
            let language = match lang {
                "json" => RawLanguage::Json,
                "xml" => RawLanguage::Xml,
                "html" => RawLanguage::Html,
                "javascript" => RawLanguage::JavaScript,
                _ => RawLanguage::Text,
            };
            Body::Raw { language, content: s(b, "raw") }
        }
        "urlencoded" => Body::UrlEncoded { fields: kvs(b.get("urlencoded")) },
        "formdata" => Body::FormData {
            fields: b
                .get("formdata")
                .and_then(|f| f.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|e| {
                            let is_file = s(e, "type") == "file";
                            let value = if is_file {
                                match e.get("src") {
                                    Some(Value::Array(a)) => a.first().and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                    _ => s(e, "src"),
                                }
                            } else {
                                s(e, "value")
                            };
                            FormField {
                                enabled: !e.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false),
                                key: s(e, "key"),
                                value,
                                is_file,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        "file" => Body::Binary { path: b.get("file").map(|f| s(f, "src")).unwrap_or_default() },
        "graphql" => {
            let g = b.get("graphql").cloned().unwrap_or(Value::Null);
            Body::GraphQl { query: s(&g, "query"), variables: s(&g, "variables") }
        }
        _ => Body::None,
    }
}

// ---------------------------------------------------------------- export

pub fn export_collection(c: &Collection) -> String {
    let mut v = json!({
        "info": {
            "_postman_id": c.id,
            "name": c.name,
            "description": c.description,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": export_items(&c.items),
        "variable": export_kvs(&c.variables),
    });
    if let Some(a) = export_auth(&c.auth) {
        v["auth"] = a;
    }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

pub fn export_environment(e: &Environment) -> String {
    let v = json!({
        "id": e.id,
        "name": e.name,
        "values": e.variables.iter().map(|kv| json!({"key": kv.key, "value": kv.value, "enabled": kv.enabled, "type": "default"})).collect::<Vec<_>>(),
        "_postman_variable_scope": "environment",
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

fn export_kvs(kvs: &[KeyValue]) -> Value {
    Value::Array(
        kvs.iter()
            .map(|kv| {
                let mut o = json!({"key": kv.key, "value": kv.value});
                if !kv.enabled {
                    o["disabled"] = json!(true);
                }
                if !kv.description.is_empty() {
                    o["description"] = json!(kv.description);
                }
                o
            })
            .collect(),
    )
}

fn export_auth(a: &Auth) -> Option<Value> {
    let kv = |k: &str, v: &str| json!({"key": k, "value": v, "type": "string"});
    Some(match a {
        Auth::Inherit => return None,
        Auth::None => json!({"type": "noauth"}),
        Auth::Bearer { token } => json!({"type": "bearer", "bearer": [kv("token", token)]}),
        Auth::Basic { username, password } => json!({"type": "basic", "basic": [kv("username", username), kv("password", password)]}),
        Auth::ApiKey { key, value, location } => json!({"type": "apikey", "apikey": [
            kv("key", key), kv("value", value),
            kv("in", if *location == ApiKeyLocation::Query { "query" } else { "header" })
        ]}),
    })
}

fn export_items(items: &[Item]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| match item {
                Item::Folder(f) => {
                    let mut o = json!({"name": f.name, "item": export_items(&f.items), "description": f.description});
                    if let Some(a) = export_auth(&f.auth) {
                        o["auth"] = a;
                    }
                    o
                }
                Item::Request(r) => export_request(r),
            })
            .collect(),
    )
}

fn export_request(r: &Request) -> Value {
    let mut req = json!({
        "method": r.method.as_str(),
        "header": export_kvs(&r.headers),
        "url": export_url(r),
        "description": r.description,
    });
    if let Some(a) = export_auth(&r.auth) {
        req["auth"] = a;
    }
    let body = match &r.body {
        Body::None => None,
        Body::Raw { language, content } => Some(json!({
            "mode": "raw", "raw": content,
            "options": {"raw": {"language": language.label().to_lowercase()}}
        })),
        Body::UrlEncoded { fields } => Some(json!({"mode": "urlencoded", "urlencoded": export_kvs(fields)})),
        Body::FormData { fields } => Some(json!({"mode": "formdata", "formdata": fields.iter().map(|f| {
            let mut o = if f.is_file { json!({"key": f.key, "type": "file", "src": f.value}) } else { json!({"key": f.key, "type": "text", "value": f.value}) };
            if !f.enabled { o["disabled"] = json!(true); }
            o
        }).collect::<Vec<_>>()})),
        Body::Binary { path } => Some(json!({"mode": "file", "file": {"src": path}})),
        Body::GraphQl { query, variables } => Some(json!({"mode": "graphql", "graphql": {"query": query, "variables": variables}})),
    };
    if let Some(b) = body {
        req["body"] = b;
    }
    json!({"name": r.name, "request": req, "response": []})
}

fn export_url(r: &Request) -> Value {
    let raw = r.url.clone();
    let (base, _) = raw.split_once('?').unwrap_or((&raw, ""));
    let (protocol, rest) = base.split_once("://").map(|(p, r)| (Some(p), r)).unwrap_or((None, base));
    let mut parts = rest.splitn(2, '/');
    let host = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let mut o = json!({
        "raw": raw,
        "host": host.split('.').collect::<Vec<_>>(),
        "path": path.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>(),
    });
    if let Some(p) = protocol {
        o["protocol"] = json!(p);
    }
    if !r.params.is_empty() {
        o["query"] = export_kvs(&r.params);
    }
    if !r.path_vars.is_empty() {
        o["variable"] = export_kvs(&r.path_vars);
    }
    o
}

// ---------------------------------------------------------------- cURL

/// Splits a shell command line into words, honouring quotes and backslash continuations.
fn shell_words(input: &str) -> Vec<String> {
    let mut words = vec![];
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(&n) = chars.peek() {
                                if matches!(n, '"' | '\\' | '$' | '`') {
                                    cur.push(n);
                                    chars.next();
                                    continue;
                                }
                            }
                            cur.push('\\');
                        }
                        _ => cur.push(c),
                    }
                }
            }
            '\\' => match chars.next() {
                Some('\n') | Some('\r') => {}
                Some(n) => {
                    in_word = true;
                    cur.push(n);
                }
                None => {}
            },
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

pub fn looks_like_curl(text: &str) -> bool {
    text.trim_start().starts_with("curl ")
}

pub fn parse_curl(cmd: &str) -> anyhow::Result<Request> {
    let words = shell_words(cmd.trim());
    if words.first().map(|w| w.as_str()) != Some("curl") {
        bail!("not a curl command");
    }
    let mut req = Request { name: "Imported from cURL".into(), ..Default::default() };
    let mut method: Option<Method> = None;
    let mut data: Vec<String> = vec![];
    let mut urlencoded: Vec<KeyValue> = vec![];
    let mut form: Vec<FormField> = vec![];
    let mut get_mode = false;
    let mut it = words.into_iter().skip(1).peekable();
    while let Some(w) = it.next() {
        let mut next = || it.next().unwrap_or_default();
        match w.as_str() {
            "-X" | "--request" => method = Some(Method::parse(&next())),
            "-H" | "--header" => {
                let h = next();
                if let Some((k, v)) = h.split_once(':') {
                    req.headers.push(KeyValue::new(k.trim(), v.trim()));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-ascii" | "--data-binary" => {
                let d = next();
                if let Some(path) = d.strip_prefix('@').filter(|_| w == "--data-binary") {
                    req.body = Body::Binary { path: path.to_string() };
                } else {
                    data.push(d);
                }
            }
            "--data-urlencode" => {
                let d = next();
                let (k, v) = d.split_once('=').unwrap_or((&d, ""));
                urlencoded.push(KeyValue::new(k, v));
            }
            "-F" | "--form" | "--form-string" => {
                let f = next();
                let (k, v) = f.split_once('=').unwrap_or((&f, ""));
                let is_file = v.starts_with('@') && w != "--form-string";
                let v = v.trim_start_matches('@').trim_matches('"');
                form.push(FormField { enabled: true, key: k.into(), value: v.into(), is_file });
            }
            "-u" | "--user" => {
                let u = next();
                let (user, pass) = u.split_once(':').unwrap_or((&u, ""));
                req.auth = Auth::Basic { username: user.into(), password: pass.into() };
            }
            "-A" | "--user-agent" => req.headers.push(KeyValue::new("User-Agent", next())),
            "-b" | "--cookie" => req.headers.push(KeyValue::new("Cookie", next())),
            "-e" | "--referer" => req.headers.push(KeyValue::new("Referer", next())),
            "-G" | "--get" => get_mode = true,
            "-I" | "--head" => method = Some(Method::HEAD),
            "-k" | "--insecure" => req.settings.verify_tls = false,
            "--url" => req.url = next(),
            "-o" | "--output" | "-m" | "--max-time" | "--connect-timeout" | "-x" | "--proxy" | "-w" | "--write-out" => {
                next();
            }
            s if s.starts_with('-') => {}
            s => {
                if req.url.is_empty() {
                    req.url = s.to_string();
                }
            }
        }
    }

    if get_mode && !data.is_empty() {
        let sep = if req.url.contains('?') { '&' } else { '?' };
        req.url = format!("{}{}{}", req.url, sep, data.join("&"));
        data.clear();
    }

    let content_type = req
        .headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase())
        .unwrap_or_default();

    if !form.is_empty() {
        req.body = Body::FormData { fields: form };
        req.headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
    } else if !urlencoded.is_empty() {
        req.body = Body::UrlEncoded { fields: urlencoded };
    } else if !data.is_empty() {
        let joined = data.join("&");
        if content_type.contains("x-www-form-urlencoded") || (content_type.is_empty() && !joined.trim_start().starts_with(['{', '['])) {
            req.body = Body::UrlEncoded {
                fields: joined
                    .split('&')
                    .map(|p| {
                        let (k, v) = p.split_once('=').unwrap_or((p, ""));
                        KeyValue::new(
                            urlencoding::decode(k).map(|c| c.into_owned()).unwrap_or_else(|_| k.into()),
                            urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.into()),
                        )
                    })
                    .collect(),
            };
        } else {
            let language = if content_type.contains("json") || joined.trim_start().starts_with(['{', '[']) {
                RawLanguage::Json
            } else if content_type.contains("xml") {
                RawLanguage::Xml
            } else {
                RawLanguage::Text
            };
            req.body = Body::Raw { language, content: joined };
        }
    }

    req.method = method.unwrap_or(if matches!(req.body, Body::None) { Method::GET } else { Method::POST });
    req.params = crate::ui::util::parse_query(&req.url);
    if let Ok(u) = url::Url::parse(&req.url) {
        req.name = format!("{} {}", u.host_str().unwrap_or(""), u.path());
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_json() {
        let r = parse_curl(r#"curl -X PUT 'https://api.x.com/v1/a?b=1' -H 'Content-Type: application/json' \
            --data-raw '{"a": "it'\''s"}'"#).unwrap();
        assert_eq!(r.method, Method::PUT);
        assert_eq!(r.url, "https://api.x.com/v1/a?b=1");
        assert_eq!(r.params[0].key, "b");
        assert_eq!(r.body, Body::Raw { language: RawLanguage::Json, content: r#"{"a": "it's"}"#.into() });
    }

    #[test]
    fn curl_form() {
        let r = parse_curl(r#"curl https://x.io -F "file=@/tmp/a.png" -F name=bob -u me:pw"#).unwrap();
        assert_eq!(r.method, Method::POST);
        assert!(matches!(r.body, Body::FormData { ref fields } if fields.len() == 2 && fields[0].is_file));
        assert!(matches!(r.auth, Auth::Basic { .. }));
    }

    #[test]
    fn roundtrip_collection() {
        let mut c = Collection::new("Test");
        c.items.push(Item::Request(Request { name: "r".into(), url: "https://a.b/c?x=1".into(), method: Method::POST,
            body: Body::Raw { language: RawLanguage::Json, content: "{}".into() },
            auth: Auth::Bearer { token: "t".into() }, ..Default::default() }));
        let text = export_collection(&c);
        let Imported::Collection(back) = import(&text, None).unwrap() else { panic!() };
        let Item::Request(r) = &back.items[0] else { panic!() };
        assert_eq!(r.method, Method::POST);
        assert_eq!(r.auth, Auth::Bearer { token: "t".into() });
        assert_eq!(r.body, Body::Raw { language: RawLanguage::Json, content: "{}".into() });
    }
}
