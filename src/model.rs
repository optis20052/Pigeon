//! Core data model: collections, requests, environments, history.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Method {
    #[default]
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
    HEAD,
    OPTIONS,
}

impl Method {
    pub const ALL: [Method; 7] = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Method::GET => "GET",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::PATCH => "PATCH",
            Method::DELETE => "DELETE",
            Method::HEAD => "HEAD",
            Method::OPTIONS => "OPTIONS",
        }
    }

    pub fn parse(s: &str) -> Method {
        match s.to_ascii_uppercase().as_str() {
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            "DELETE" => Method::DELETE,
            "HEAD" => Method::HEAD,
            "OPTIONS" => Method::OPTIONS,
            _ => Method::GET,
        }
    }

    pub fn index(&self) -> u32 {
        Method::ALL.iter().position(|m| m == self).unwrap_or(0) as u32
    }

    /// CSS class used to color the method label.
    pub fn css_class(&self) -> &'static str {
        match self {
            Method::GET => "method-get",
            Method::POST => "method-post",
            Method::PUT => "method-put",
            Method::PATCH => "method-patch",
            Method::DELETE => "method-delete",
            Method::HEAD => "method-head",
            Method::OPTIONS => "method-options",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyValue {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub description: String,
}

fn yes() -> bool {
    true
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { enabled: true, key: key.into(), value: value.into(), description: String::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RawLanguage {
    #[default]
    Json,
    Text,
    Xml,
    Html,
    JavaScript,
}

impl RawLanguage {
    pub const ALL: [RawLanguage; 5] =
        [RawLanguage::Json, RawLanguage::Text, RawLanguage::Xml, RawLanguage::Html, RawLanguage::JavaScript];

    pub fn label(&self) -> &'static str {
        match self {
            RawLanguage::Json => "JSON",
            RawLanguage::Text => "Text",
            RawLanguage::Xml => "XML",
            RawLanguage::Html => "HTML",
            RawLanguage::JavaScript => "JavaScript",
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            RawLanguage::Json => "application/json",
            RawLanguage::Text => "text/plain",
            RawLanguage::Xml => "application/xml",
            RawLanguage::Html => "text/html",
            RawLanguage::JavaScript => "application/javascript",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormField {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub key: String,
    pub value: String,
    /// When true, `value` is a path to a file on disk.
    #[serde(default)]
    pub is_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Body {
    #[default]
    None,
    Raw { language: RawLanguage, content: String },
    UrlEncoded { fields: Vec<KeyValue> },
    FormData { fields: Vec<FormField> },
    Binary { path: String },
    GraphQl { query: String, variables: String },
}

impl Body {
    pub fn mode_index(&self) -> u32 {
        match self {
            Body::None => 0,
            Body::Raw { .. } => 1,
            Body::UrlEncoded { .. } => 2,
            Body::FormData { .. } => 3,
            Body::Binary { .. } => 4,
            Body::GraphQl { .. } => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ApiKeyLocation {
    #[default]
    Header,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Auth {
    /// Inherit auth from the parent folder / collection / project.
    #[default]
    Inherit,
    None,
    Bearer { token: String },
    Basic { username: String, password: String },
    ApiKey { key: String, value: String, location: ApiKeyLocation },
}

impl Auth {
    pub fn index(&self) -> u32 {
        match self {
            Auth::Inherit => 0,
            Auth::None => 1,
            Auth::Bearer { .. } => 2,
            Auth::Basic { .. } => 3,
            Auth::ApiKey { .. } => 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestSettings {
    #[serde(default = "yes")]
    pub follow_redirects: bool,
    #[serde(default = "yes")]
    pub verify_tls: bool,
    /// Timeout in milliseconds, 0 = none.
    #[serde(default)]
    pub timeout_ms: u64,
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self { follow_redirects: true, verify_tls: true, timeout_ms: 0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub method: Method,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub params: Vec<KeyValue>,
    /// Values for `:name` segments in the URL path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_vars: Vec<KeyValue>,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub settings: RequestSettings,
    #[serde(default)]
    pub description: String,
    /// Simple declarative tests run against the response.
    #[serde(default)]
    pub tests: Vec<Assertion>,
    /// OpenAPI operation this request was imported from (`GET /users/{id}`), used when syncing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "Untitled Request".into(),
            method: Method::GET,
            url: String::new(),
            params: vec![],
            path_vars: vec![],
            headers: vec![],
            body: Body::None,
            auth: Auth::Inherit,
            settings: RequestSettings::default(),
            description: String::new(),
            tests: vec![],
            operation: None,
        }
    }
}

/// A declarative test: `<source> <op> <expected>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Assertion {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// e.g. `status`, `header.Content-Type`, `json.data[0].id`, `body`, `time`
    pub source: String,
    pub op: AssertOp,
    pub expected: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssertOp {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    LessThan,
    GreaterThan,
    Exists,
    Matches,
}

impl AssertOp {
    pub const ALL: [AssertOp; 8] = [
        AssertOp::Equals,
        AssertOp::NotEquals,
        AssertOp::Contains,
        AssertOp::NotContains,
        AssertOp::LessThan,
        AssertOp::GreaterThan,
        AssertOp::Exists,
        AssertOp::Matches,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AssertOp::Equals => "equals",
            AssertOp::NotEquals => "not equals",
            AssertOp::Contains => "contains",
            AssertOp::NotContains => "not contains",
            AssertOp::LessThan => "less than",
            AssertOp::GreaterThan => "greater than",
            AssertOp::Exists => "exists",
            AssertOp::Matches => "matches regex",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Request(Request),
    Folder(Folder),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Item::Request(r) => &r.id,
            Item::Folder(f) => &f.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Item::Request(r) => &r.name,
            Item::Folder(f) => &f.name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Folder {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub description: String,
    /// Label color key (see `ui::colors`), inherited as a tint by everything inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Folder {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: new_id(), name: name.into(), items: vec![], auth: Auth::Inherit, description: String::new(), color: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collection {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub variables: Vec<KeyValue>,
    #[serde(default)]
    pub description: String,
    /// OpenAPI spec location (URL or file path) the collection was imported from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openapi_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            items: vec![],
            auth: Auth::Inherit,
            variables: vec![],
            description: String::new(),
            openapi_source: None,
            color: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Environment {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub variables: Vec<KeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub request: Request,
    pub status: Option<u16>,
    pub elapsed_ms: u128,
}

/// How far a collection/folder color reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorScope {
    /// The folder and everything inside it, including subfolders (unless they have their own color).
    #[default]
    Cascade,
    /// The folder and the requests directly inside it.
    Direct,
    /// Only the folder's own row.
    RowOnly,
}

impl ColorScope {
    pub const ALL: [ColorScope; 3] = [ColorScope::Cascade, ColorScope::Direct, ColorScope::RowOnly];

    pub fn label(&self) -> &'static str {
        match self {
            ColorScope::Cascade => "Folder and everything inside",
            ColorScope::Direct => "Folder and its own requests",
            ColorScope::RowOnly => "Folder row only",
        }
    }
}

/// A project's icon: a themed symbolic icon (optionally tinted) or a custom image file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectIcon {
    Symbolic {
        name: String,
        /// Palette key from `ui::colors`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    /// Image copied into Pigeon's data dir.
    Image { path: String },
}

/// App-wide preferences (not per project).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub color_scope: ColorScope,
    /// App icon variant key (see `ui::app_icon`).
    #[serde(default = "default_app_icon")]
    pub app_icon: String,
}

fn default_app_icon() -> String {
    "dusk".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self { color_scope: ColorScope::default(), app_icon: default_app_icon() }
    }
}

/// A project: everything persisted to disk for one isolated workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default = "default_project_name")]
    pub name: String,
    /// Project-wide auth, inherited by collections set to "Inherit".
    #[serde(default)]
    pub auth: Auth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProjectIcon>,
    #[serde(default)]
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(default)]
    pub active_environment: Option<String>,
    #[serde(default)]
    pub globals: Vec<KeyValue>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

fn default_project_name() -> String {
    "Default".into()
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: default_project_name(),
            auth: Auth::Inherit,
            icon: None,
            collections: vec![],
            environments: vec![],
            active_environment: None,
            globals: vec![],
            history: vec![],
        }
    }
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    /// What "Inherit" resolves to for `item_id` (a request, folder or collection),
    /// looking only at its ancestors: (auth, human-readable source).
    pub fn inherited_auth(&self, collection: Option<&str>, item_id: &str) -> (Auth, String) {
        let mut chain: Vec<(&str, Auth, String)> = vec![("", self.auth.clone(), format!("project “{}”", self.name))];
        if let Some(c) = collection.and_then(|cid| self.collections.iter().find(|c| c.id == cid)) {
            chain.push((&c.id, c.auth.clone(), format!("collection “{}”", c.name)));
            for f in folder_path(&c.items, item_id) {
                chain.push((&f.id, f.auth.clone(), format!("folder “{}”", f.name)));
            }
        }
        chain
            .into_iter()
            .filter(|(id, _, _)| *id != item_id)
            .rev()
            .find(|(_, a, _)| *a != Auth::Inherit)
            .map(|(_, a, s)| (a, s))
            .unwrap_or((Auth::None, "nowhere".into()))
    }

    /// Color of the nearest colored ancestor of `item_id` (or the item itself, if it is a folder).
    pub fn tint_for(&self, collection: Option<&str>, item_id: &str, scope: ColorScope) -> Option<String> {
        let c = self.collections.iter().find(|c| Some(c.id.as_str()) == collection)?;
        let path = folder_path(&c.items, item_id);
        match scope {
            ColorScope::Cascade => path.iter().rev().find_map(|f| f.color.clone()).or_else(|| c.color.clone()),
            // only the immediate parent's color
            ColorScope::Direct => match path.last() {
                Some(parent) => parent.color.clone(),
                None => c.color.clone(),
            },
            ColorScope::RowOnly => None,
        }
    }

    /// Ids of the folders enclosing `item_id` within a collection, outermost first.
    pub fn ancestor_ids(&self, collection: &str, item_id: &str) -> Vec<String> {
        self.collections
            .iter()
            .find(|c| c.id == collection)
            .map(|c| folder_path(&c.items, item_id).iter().map(|f| f.id.clone()).collect())
            .unwrap_or_default()
    }

    /// Sets the color of a collection or folder. Returns false if `id` wasn't found.
    pub fn set_color(&mut self, id: &str, color: Option<String>) -> bool {
        for c in self.collections.iter_mut() {
            if c.id == id {
                c.color = color;
                return true;
            }
            if let Some(f) = find_folder_mut(&mut c.items, id) {
                f.color = color;
                return true;
            }
        }
        false
    }

    /// Auth actually sent for a request.
    pub fn effective_auth(&self, collection: Option<&str>, req: &Request) -> Auth {
        if req.auth != Auth::Inherit { req.auth.clone() } else { self.inherited_auth(collection, &req.id).0 }
    }
}

/// Folders enclosing `id` (including `id` itself if it is a folder), outermost first.
fn folder_path<'a>(items: &'a [Item], id: &str) -> Vec<&'a Folder> {
    fn walk<'a>(items: &'a [Item], id: &str, path: &mut Vec<&'a Folder>) -> bool {
        for item in items {
            match item {
                Item::Request(r) if r.id == id => return true,
                Item::Folder(f) => {
                    path.push(f);
                    if f.id == id || walk(&f.items, id, path) {
                        return true;
                    }
                    path.pop();
                }
                _ => {}
            }
        }
        false
    }
    let mut path = vec![];
    walk(items, id, &mut path);
    path
}

// ----- tree helpers -----

pub fn find_request<'a>(items: &'a [Item], id: &str) -> Option<&'a Request> {
    for item in items {
        match item {
            Item::Request(r) if r.id == id => return Some(r),
            Item::Folder(f) => {
                if let Some(r) = find_request(&f.items, id) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn find_request_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Request> {
    for item in items {
        match item {
            Item::Request(r) if r.id == id => return Some(r),
            Item::Folder(f) => {
                if let Some(r) = find_request_mut(&mut f.items, id) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn find_folder_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Folder> {
    for item in items {
        if let Item::Folder(f) = item {
            if f.id == id {
                return Some(f);
            }
            if let Some(found) = find_folder_mut(&mut f.items, id) {
                return Some(found);
            }
        }
    }
    None
}

/// Removes an item anywhere in the tree, returning it.
pub fn remove_item(items: &mut Vec<Item>, id: &str) -> Option<Item> {
    if let Some(pos) = items.iter().position(|i| i.id() == id) {
        return Some(items.remove(pos));
    }
    for item in items.iter_mut() {
        if let Item::Folder(f) = item {
            if let Some(removed) = remove_item(&mut f.items, id) {
                return Some(removed);
            }
        }
    }
    None
}

/// Renames an item anywhere in the tree.
pub fn rename_item(items: &mut [Item], id: &str, name: &str) -> bool {
    for item in items {
        if item.id() == id {
            match item {
                Item::Request(r) => r.name = name.to_string(),
                Item::Folder(f) => f.name = name.to_string(),
            }
            return true;
        }
        if let Item::Folder(f) = item {
            if rename_item(&mut f.items, id, name) {
                return true;
            }
        }
    }
    false
}

/// Deep-clones an item giving it (and all children) fresh ids.
pub fn duplicate_item(item: &Item) -> Item {
    match item {
        Item::Request(r) => {
            let mut r = r.clone();
            r.id = new_id();
            r.name = format!("{} Copy", r.name);
            Item::Request(r)
        }
        Item::Folder(f) => {
            let mut nf = f.clone();
            nf.id = new_id();
            nf.name = format!("{} Copy", f.name);
            nf.items = f.items.iter().map(|i| {
                let mut d = duplicate_item(i);
                // children keep their original names
                match (&mut d, i) {
                    (Item::Request(r), Item::Request(o)) => r.name = o.name.clone(),
                    (Item::Folder(r), Item::Folder(o)) => r.name = o.name.clone(),
                    _ => {}
                }
                d
            }).collect();
            Item::Folder(nf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_inheritance_chain() {
        let mut ws = Workspace::new("P");
        ws.auth = Auth::Bearer { token: "project".into() };
        let req = Request { id: "r".into(), ..Default::default() };
        let mut folder = Folder::new("F");
        folder.id = "f".into();
        folder.items.push(Item::Request(req.clone()));
        let mut c = Collection::new("C");
        c.id = "c".into();
        c.items.push(Item::Folder(folder));
        ws.collections.push(c);

        // everything inherits -> project auth
        assert_eq!(ws.effective_auth(Some("c"), &req), Auth::Bearer { token: "project".into() });
        assert!(ws.inherited_auth(Some("c"), "r").1.contains("project"));
        // drafts (no collection) still get project auth
        assert_eq!(ws.effective_auth(None, &req), Auth::Bearer { token: "project".into() });

        // folder overrides
        if let Item::Folder(f) = &mut ws.collections[0].items[0] {
            f.auth = Auth::Basic { username: "u".into(), password: "p".into() };
        }
        assert!(matches!(ws.effective_auth(Some("c"), &req), Auth::Basic { .. }));
        // a folder's own hint ignores its own auth
        assert!(ws.inherited_auth(Some("c"), "f").1.contains("project"));

        // color tint: nearest colored ancestor wins
        assert_eq!(ws.tint_for(Some("c"), "r", ColorScope::Cascade), None);
        assert!(ws.set_color("c", Some("blue".into())));
        assert_eq!(ws.tint_for(Some("c"), "r", ColorScope::Cascade).as_deref(), Some("blue"));
        assert!(ws.set_color("f", Some("orange".into())));
        assert_eq!(ws.tint_for(Some("c"), "r", ColorScope::Cascade).as_deref(), Some("orange"));
        assert!(!ws.set_color("missing", None));
        // request "r" sits directly in folder "f"
        assert_eq!(ws.tint_for(Some("c"), "r", ColorScope::Direct).as_deref(), Some("orange"));
        assert!(ws.set_color("f", None));
        assert_eq!(ws.tint_for(Some("c"), "r", ColorScope::Direct), None);
        assert_eq!(ws.tint_for(Some("c"), "r", ColorScope::RowOnly), None);

        // explicit request auth wins
        let own = Request { auth: Auth::None, ..req };
        assert_eq!(ws.effective_auth(Some("c"), &own), Auth::None);
    }
}
