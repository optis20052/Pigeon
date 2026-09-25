//! Request preparation (variable resolution, auth, body) and execution.

use crate::model::{ApiKeyLocation, Auth, Body, FormField, KeyValue, Method, Request, RequestSettings};
use crate::vars::Scope;
use base64::Engine;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Cookie jar shared by all requests, like a browser session.
static COOKIE_JAR: Lazy<RwLock<Arc<reqwest::cookie::Jar>>> = Lazy::new(|| RwLock::new(Arc::new(reqwest::cookie::Jar::default())));

pub fn clear_cookies() {
    *COOKIE_JAR.write().unwrap() = Arc::new(reqwest::cookie::Jar::default());
}

/// Tokio runtime that performs all network I/O off the GTK main thread.
pub static RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("failed to start tokio runtime")
});

#[derive(Debug, Clone)]
pub enum PreparedBody {
    None,
    Text { content: String, content_type: Option<String> },
    UrlEncoded(Vec<(String, String)>),
    Multipart(Vec<(String, String, bool)>),
    File(String),
}

/// A request with all variables resolved and auth applied.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: PreparedBody,
    pub settings: RequestSettings,
}

fn enabled(kvs: &[KeyValue], scope: &Scope) -> Vec<(String, String)> {
    kvs.iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        .map(|kv| (scope.apply(&kv.key), scope.apply(&kv.value)))
        .collect()
}

pub fn prepare(req: &Request, auth: &Auth, scope: &Scope) -> Prepared {
    let path_values: Vec<(String, String)> =
        req.path_vars.iter().filter(|kv| kv.enabled).map(|kv| (kv.key.clone(), scope.apply(&kv.value))).collect();
    let mut url = crate::vars::apply_path_vars(&scope.apply(req.url.trim()), &path_values);
    if !url.is_empty() && !url.contains("://") {
        url = format!("http://{url}");
    }

    // Query params are kept in sync with the URL by the UI, so the URL already carries them.
    let mut headers = enabled(&req.headers, scope);
    let has_header = |headers: &[(String, String)], name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));

    match auth {
        Auth::Bearer { token } => {
            if !has_header(&headers, "authorization") {
                headers.push(("Authorization".into(), format!("Bearer {}", scope.apply(token))));
            }
        }
        Auth::Basic { username, password } => {
            if !has_header(&headers, "authorization") {
                let raw = format!("{}:{}", scope.apply(username), scope.apply(password));
                let enc = base64::engine::general_purpose::STANDARD.encode(raw);
                headers.push(("Authorization".into(), format!("Basic {enc}")));
            }
        }
        Auth::ApiKey { key, value, location } => {
            let (k, v) = (scope.apply(key), scope.apply(value));
            if !k.is_empty() {
                match location {
                    ApiKeyLocation::Header => headers.push((k, v)),
                    ApiKeyLocation::Query => {
                        if let Ok(mut u) = url::Url::parse(&url) {
                            u.query_pairs_mut().append_pair(&k, &v);
                            url = u.to_string();
                        }
                    }
                }
            }
        }
        Auth::None | Auth::Inherit => {}
    }

    let body = match &req.body {
        Body::None => PreparedBody::None,
        Body::Raw { language, content } => PreparedBody::Text {
            content: scope.apply(content),
            content_type: Some(language.content_type().to_string()),
        },
        Body::GraphQl { query, variables } => {
            let vars: serde_json::Value = serde_json::from_str(&scope.apply(variables)).unwrap_or(serde_json::Value::Null);
            let payload = serde_json::json!({ "query": scope.apply(query), "variables": vars });
            PreparedBody::Text { content: payload.to_string(), content_type: Some("application/json".into()) }
        }
        Body::UrlEncoded { fields } => PreparedBody::UrlEncoded(enabled(fields, scope)),
        Body::FormData { fields } => PreparedBody::Multipart(
            fields
                .iter()
                .filter(|f: &&FormField| f.enabled && !f.key.is_empty())
                .map(|f| (scope.apply(&f.key), scope.apply(&f.value), f.is_file))
                .collect(),
        ),
        Body::Binary { path } => PreparedBody::File(scope.apply(path)),
    };

    if let PreparedBody::Text { content_type: Some(ct), .. } = &body {
        if !has_header(&headers, "content-type") {
            headers.push(("Content-Type".into(), ct.clone()));
        }
    }

    Prepared { method: req.method, url, headers, body, settings: req.settings.clone() }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub elapsed: Duration,
    pub final_url: String,
    pub remote_addr: Option<String>,
}

impl Response {
    pub fn content_type(&self) -> String {
        self.header("content-type").unwrap_or_default().to_ascii_lowercase()
    }

    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_json(&self) -> bool {
        let ct = self.content_type();
        ct.contains("json") || (ct.is_empty() && serde_json::from_slice::<serde_json::Value>(&self.body).is_ok())
    }

    /// Size of headers + body.
    pub fn size(&self) -> usize {
        self.body.len() + self.headers.iter().map(|(k, v)| k.len() + v.len() + 4).sum::<usize>()
    }
}

pub async fn execute(p: Prepared) -> anyhow::Result<Response> {
    let mut builder = reqwest::Client::builder()
        .cookie_provider(COOKIE_JAR.read().unwrap().clone())
        .user_agent(concat!("Pigeon/", env!("CARGO_PKG_VERSION")))
        .tls_danger_accept_invalid_certs(!p.settings.verify_tls)
        .redirect(if p.settings.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        });
    if p.settings.timeout_ms > 0 {
        builder = builder.timeout(Duration::from_millis(p.settings.timeout_ms));
    }
    let client = builder.build()?;

    let method = reqwest::Method::from_bytes(p.method.as_str().as_bytes())?;
    let mut rb = client.request(method, &p.url);
    for (k, v) in &p.headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    rb = match p.body {
        PreparedBody::None => rb,
        PreparedBody::Text { content, .. } => rb.body(content),
        PreparedBody::UrlEncoded(pairs) => rb.form(&pairs),
        PreparedBody::Multipart(fields) => {
            let mut form = reqwest::multipart::Form::new();
            for (k, v, is_file) in fields {
                if is_file {
                    let bytes = tokio::fs::read(&v).await.map_err(|e| anyhow::anyhow!("reading {v}: {e}"))?;
                    let name = std::path::Path::new(&v).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mime = mime_guess::from_path(&v).first_or_octet_stream();
                    let part = reqwest::multipart::Part::bytes(bytes).file_name(name).mime_str(mime.as_ref())?;
                    form = form.part(k, part);
                } else {
                    form = form.text(k, v);
                }
            }
            rb.multipart(form)
        }
        PreparedBody::File(path) => {
            let bytes = tokio::fs::read(&path).await.map_err(|e| anyhow::anyhow!("reading {path}: {e}"))?;
            if !p.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                rb = rb.header("Content-Type", mime_guess::from_path(&path).first_or_octet_stream().as_ref());
            }
            rb.body(bytes)
        }
    };

    let start = Instant::now();
    let resp = rb.send().await?;
    let status = resp.status();
    let version = format!("{:?}", resp.version());
    let final_url = resp.url().to_string();
    let remote_addr = resp.remote_addr().map(|a| a.to_string());
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    let body = resp.bytes().await?.to_vec();
    let elapsed = start.elapsed();

    Ok(Response {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("").to_string(),
        version,
        headers,
        body,
        elapsed,
        final_url,
        remote_addr,
    })
}

/// Parses `Set-Cookie` headers into (name, value, attributes) triples for display.
pub fn parse_set_cookies(resp: &Response) -> Vec<(String, String, String)> {
    resp.headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| {
            let mut parts = v.splitn(2, ';');
            let pair = parts.next().unwrap_or("");
            let attrs = parts.next().unwrap_or("").trim().to_string();
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (name.trim().to_string(), value.trim().to_string(), attrs)
        })
        .collect()
}

pub fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn human_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 { format!("{ms:.0} ms") } else { format!("{:.2} s", ms / 1000.0) }
}

/// Downloads an importable document (OpenAPI or collection JSON / YAML). When `url` points at a
/// docs page (e.g. Swagger UI) the usual spec locations next to it are tried as well.
/// Returns the text and the URL it was found at.
pub async fn fetch_spec(url: &str) -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).user_agent(concat!("Pigeon/", env!("CARGO_PKG_VERSION"))).build()?;
    let url = if url.contains("://") { url.to_string() } else { format!("http://{url}") };
    let base = url.trim_end_matches('/').to_string();
    let origin = url::Url::parse(&url).map(|u| u.origin().ascii_serialization()).unwrap_or_else(|_| base.clone());
    let mut candidates = vec![url.clone()];
    for suffix in ["/json", "/openapi.json", "/swagger.json", "/openapi.yaml", "/v3/api-docs", "/swagger/v1/swagger.json"] {
        candidates.push(format!("{base}{suffix}"));
    }
    for path in ["/openapi.json", "/swagger.json", "/docs/json", "/api-docs", "/v3/api-docs", "/swagger/v1/swagger.json", "/api/openapi.json"] {
        candidates.push(format!("{origin}{path}"));
    }
    candidates.dedup();

    let mut first_error = None;
    for candidate in candidates {
        match client.get(&candidate).header("Accept", "application/json, application/yaml;q=0.9, */*;q=0.5").send().await {
            Ok(resp) if resp.status().is_success() => {
                let text = resp.text().await.unwrap_or_default();
                let trimmed = text.trim_start();
                let looks_importable = (trimmed.starts_with('{') && (text.contains("\"openapi\"") || text.contains("\"swagger\"") || text.contains("\"info\"")))
                    || trimmed.starts_with("openapi:")
                    || trimmed.starts_with("swagger:");
                if looks_importable {
                    return Ok((text, candidate));
                }
            }
            Ok(resp) => {
                first_error.get_or_insert_with(|| anyhow::anyhow!("{candidate}: HTTP {}", resp.status()));
            }
            Err(e) => {
                first_error.get_or_insert_with(|| anyhow::anyhow!("{candidate}: {e}"));
            }
        }
        if first_error.as_ref().is_some_and(|e| e.to_string().contains("error sending request")) {
            break; // server unreachable, no point trying other paths
        }
    }
    Err(first_error.unwrap_or_else(|| anyhow::anyhow!("no OpenAPI / Swagger document found at {url}")))
}
