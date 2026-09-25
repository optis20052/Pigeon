//! Code snippet generation for a prepared request.

use crate::http::{Prepared, PreparedBody};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Curl,
    HttpRaw,
    PythonRequests,
    JavaScriptFetch,
    NodeAxios,
    Go,
    RustReqwest,
}

impl Target {
    pub const ALL: [Target; 7] = [
        Target::Curl,
        Target::HttpRaw,
        Target::PythonRequests,
        Target::JavaScriptFetch,
        Target::NodeAxios,
        Target::Go,
        Target::RustReqwest,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Target::Curl => "cURL",
            Target::HttpRaw => "HTTP",
            Target::PythonRequests => "Python - requests",
            Target::JavaScriptFetch => "JavaScript - fetch",
            Target::NodeAxios => "Node.js - axios",
            Target::Go => "Go - net/http",
            Target::RustReqwest => "Rust - reqwest",
        }
    }
}

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn dq(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

pub fn generate(p: &Prepared, target: Target) -> String {
    match target {
        Target::Curl => curl(p),
        Target::HttpRaw => http_raw(p),
        Target::PythonRequests => python(p),
        Target::JavaScriptFetch => fetch(p),
        Target::NodeAxios => axios(p),
        Target::Go => go(p),
        Target::RustReqwest => rust(p),
    }
}

fn curl(p: &Prepared) -> String {
    let mut out = format!("curl --location --request {} {}", p.method.as_str(), sq(&p.url));
    for (k, v) in &p.headers {
        out += &format!(" \\\n  --header {}", sq(&format!("{k}: {v}")));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Text { content, .. } => out += &format!(" \\\n  --data-raw {}", sq(content)),
        PreparedBody::UrlEncoded(pairs) => {
            for (k, v) in pairs {
                out += &format!(" \\\n  --data-urlencode {}", sq(&format!("{k}={v}")));
            }
        }
        PreparedBody::Multipart(fields) => {
            for (k, v, file) in fields {
                let val = if *file { format!("{k}=@\"{v}\"") } else { format!("{k}={v}") };
                out += &format!(" \\\n  --form {}", sq(&val));
            }
        }
        PreparedBody::File(path) => out += &format!(" \\\n  --data-binary {}", sq(&format!("@{path}"))),
    }
    out
}

fn http_raw(p: &Prepared) -> String {
    let (path, host) = match url::Url::parse(&p.url) {
        Ok(u) => {
            let mut path = u.path().to_string();
            if let Some(q) = u.query() {
                path += "?";
                path += q;
            }
            (path, u.host_str().map(|h| match u.port() {
                Some(port) => format!("{h}:{port}"),
                None => h.to_string(),
            }).unwrap_or_default())
        }
        Err(_) => (p.url.clone(), String::new()),
    };
    let mut out = format!("{} {} HTTP/1.1\nHost: {}\n", p.method.as_str(), path, host);
    for (k, v) in &p.headers {
        out += &format!("{k}: {v}\n");
    }
    match &p.body {
        PreparedBody::Text { content, .. } => out += &format!("Content-Length: {}\n\n{}", content.len(), content),
        PreparedBody::UrlEncoded(pairs) => {
            let body = pairs.iter().map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))).collect::<Vec<_>>().join("&");
            out += &format!("Content-Type: application/x-www-form-urlencoded\n\n{body}");
        }
        PreparedBody::Multipart(_) => out += "Content-Type: multipart/form-data; boundary=----PigeonBoundary\n\n…",
        PreparedBody::File(path) => out += &format!("\n<file contents: {path}>"),
        PreparedBody::None => {}
    }
    out
}

fn python(p: &Prepared) -> String {
    let mut out = String::from("import requests\n\n");
    out += &format!("url = {}\n", dq(&p.url));
    out += "headers = {\n";
    for (k, v) in &p.headers {
        out += &format!("    {}: {},\n", dq(k), dq(v));
    }
    out += "}\n";
    let body_arg = match &p.body {
        PreparedBody::None => "",
        PreparedBody::Text { content, .. } => {
            out += &format!("payload = {}\n", dq(content));
            ", data=payload"
        }
        PreparedBody::UrlEncoded(pairs) => {
            out += "payload = {\n";
            for (k, v) in pairs {
                out += &format!("    {}: {},\n", dq(k), dq(v));
            }
            out += "}\n";
            ", data=payload"
        }
        PreparedBody::Multipart(fields) => {
            out += "payload = {\n";
            for (k, v, f) in fields.iter().filter(|f| !f.2) {
                let _ = f;
                out += &format!("    {}: {},\n", dq(k), dq(v));
            }
            out += "}\nfiles = [\n";
            for (k, v, _) in fields.iter().filter(|f| f.2) {
                out += &format!("    ({}, open({}, 'rb')),\n", dq(k), dq(v));
            }
            out += "]\n";
            ", data=payload, files=files"
        }
        PreparedBody::File(path) => {
            out += &format!("payload = open({}, 'rb')\n", dq(path));
            ", data=payload"
        }
    };
    out += &format!("\nresponse = requests.request({}, url, headers=headers{body_arg})\n\nprint(response.text)\n", dq(p.method.as_str()));
    out
}

fn js_headers(p: &Prepared, indent: &str) -> String {
    let mut out = String::from("{\n");
    for (k, v) in &p.headers {
        out += &format!("{indent}  {}: {},\n", dq(k), dq(v));
    }
    out + indent + "}"
}

fn fetch(p: &Prepared) -> String {
    let body = match &p.body {
        PreparedBody::None => None,
        PreparedBody::Text { content, .. } => Some(dq(content)),
        PreparedBody::UrlEncoded(pairs) => Some(format!(
            "new URLSearchParams({{\n{}  }})",
            pairs.iter().map(|(k, v)| format!("    {}: {},\n", dq(k), dq(v))).collect::<String>()
        )),
        PreparedBody::Multipart(_) => Some("formData".into()),
        PreparedBody::File(_) => Some("fileInput.files[0]".into()),
    };
    let mut out = String::new();
    if let PreparedBody::Multipart(fields) = &p.body {
        out += "const formData = new FormData();\n";
        for (k, v, f) in fields {
            if *f {
                out += &format!("formData.append({}, fileInput.files[0], {});\n", dq(k), dq(v));
            } else {
                out += &format!("formData.append({}, {});\n", dq(k), dq(v));
            }
        }
        out += "\n";
    }
    out += &format!("const response = await fetch({}, {{\n  method: {},\n  headers: {},\n", dq(&p.url), dq(p.method.as_str()), js_headers(p, "  "));
    if let Some(b) = body {
        out += &format!("  body: {b},\n");
    }
    out += "});\n\nconsole.log(await response.text());\n";
    out
}

fn axios(p: &Prepared) -> String {
    let data = match &p.body {
        PreparedBody::None => None,
        PreparedBody::Text { content, .. } => Some(dq(content)),
        PreparedBody::UrlEncoded(pairs) => Some(format!("new URLSearchParams({})", dq(&pairs.iter().map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))).collect::<Vec<_>>().join("&")))),
        PreparedBody::Multipart(_) => Some("formData".into()),
        PreparedBody::File(path) => Some(format!("fs.createReadStream({})", dq(path))),
    };
    let mut out = String::from("const axios = require('axios');\n\n");
    out += &format!("const response = await axios.request({{\n  method: {},\n  url: {},\n  headers: {},\n", dq(&p.method.as_str().to_lowercase()), dq(&p.url), js_headers(p, "  "));
    if let Some(d) = data {
        out += &format!("  data: {d},\n");
    }
    out += "});\n\nconsole.log(response.data);\n";
    out
}

fn go(p: &Prepared) -> String {
    let body = match &p.body {
        PreparedBody::Text { content, .. } => format!("strings.NewReader({})", go_str(content)),
        PreparedBody::UrlEncoded(pairs) => format!("strings.NewReader({})", go_str(&pairs.iter().map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))).collect::<Vec<_>>().join("&"))),
        _ => "nil".into(),
    };
    let mut out = String::from("package main\n\nimport (\n\t\"fmt\"\n\t\"io\"\n\t\"net/http\"\n\t\"strings\"\n)\n\nfunc main() {\n");
    out += &format!("\treq, err := http.NewRequest({}, {}, {})\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n", go_str(p.method.as_str()), go_str(&p.url), body);
    for (k, v) in &p.headers {
        out += &format!("\treq.Header.Add({}, {})\n", go_str(k), go_str(v));
    }
    out += "\n\tres, err := http.DefaultClient.Do(req)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\tdefer res.Body.Close()\n\n\tbody, _ := io.ReadAll(res.Body)\n\tfmt.Println(string(body))\n}\n";
    out
}

fn go_str(s: &str) -> String {
    if !s.contains('`') && s.contains('\n') { format!("`{s}`") } else { dq(s) }
}

fn rust(p: &Prepared) -> String {
    let mut out = String::from("#[tokio::main]\nasync fn main() -> Result<(), Box<dyn std::error::Error>> {\n    let client = reqwest::Client::new();\n");
    out += &format!("    let response = client\n        .request(reqwest::Method::{}, {})\n", p.method.as_str(), dq(&p.url));
    for (k, v) in &p.headers {
        out += &format!("        .header({}, {})\n", dq(k), dq(v));
    }
    match &p.body {
        PreparedBody::Text { content, .. } => out += &format!("        .body({})\n", dq(content)),
        PreparedBody::UrlEncoded(pairs) => {
            out += &format!("        .form(&[{}])\n", pairs.iter().map(|(k, v)| format!("({}, {})", dq(k), dq(v))).collect::<Vec<_>>().join(", "));
        }
        PreparedBody::File(path) => out += &format!("        .body(std::fs::read({})?)\n", dq(path)),
        _ => {}
    }
    out += "        .send()\n        .await?;\n\n    println!(\"{}\", response.text().await?);\n    Ok(())\n}\n";
    out
}
