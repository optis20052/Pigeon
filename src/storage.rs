//! Persistence under `~/.local/share/pigeon/`:
//! - `projects.json`: the project index and which project is active
//! - `projects/<id>.json`: one file per project (collections, environments, history, ...)
//! - `sessions/<id>.json`: open tabs per project
//! - `window.json`: window geometry

use crate::model::{ProjectIcon, Settings, Workspace};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_HISTORY: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    /// Mirrors the project's icon so the switcher can draw it without loading the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProjectIcon>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectIndex {
    pub active: String,
    pub projects: Vec<ProjectRef>,
}

pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("pigeon");
    // the app used to be called Courier: move its data over once
    let legacy = base.join("courier");
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, &dir);
    }
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn sub_dir(name: &str) -> PathBuf {
    let dir = data_dir().join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn index_path() -> PathBuf {
    data_dir().join("projects.json")
}

fn project_path(id: &str) -> PathBuf {
    sub_dir("projects").join(format!("{id}.json"))
}

pub fn session_path(project_id: &str) -> PathBuf {
    sub_dir("sessions").join(format!("{project_id}.json"))
}

pub fn window_path() -> PathBuf {
    data_dir().join("window.json")
}

fn write_atomic(path: &PathBuf, text: &str) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Loads the project index, creating a default project (and migrating the
/// single-workspace layout of earlier versions) when needed.
pub fn load_index() -> ProjectIndex {
    if let Some(idx) = std::fs::read_to_string(index_path()).ok().and_then(|t| serde_json::from_str::<ProjectIndex>(&t).ok()) {
        if !idx.projects.is_empty() {
            return idx;
        }
    }
    let legacy = data_dir().join("workspace.json");
    let ws = match std::fs::read_to_string(&legacy) {
        Ok(text) => serde_json::from_str::<Workspace>(&text).unwrap_or_default(),
        Err(_) => Workspace::default(),
    };
    save(&ws);
    let legacy_session = data_dir().join("session.json");
    if legacy_session.exists() {
        let _ = std::fs::rename(&legacy_session, session_path(&ws.id));
    }
    if legacy.exists() {
        let _ = std::fs::rename(&legacy, legacy.with_extension("json.migrated"));
    }
    let idx = ProjectIndex { active: ws.id.clone(), projects: vec![ProjectRef { id: ws.id.clone(), name: ws.name.clone(), icon: None }] };
    save_index(&idx);
    idx
}

pub fn save_index(idx: &ProjectIndex) {
    if let Ok(text) = serde_json::to_string_pretty(idx) {
        write_atomic(&index_path(), &text);
    }
}

pub fn load_project(id: &str) -> Workspace {
    let path = project_path(id);
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Workspace>(&text) {
            Ok(mut ws) => {
                ws.id = id.to_string();
                ws
            }
            Err(e) => {
                eprintln!("pigeon: failed to parse {}: {e}", path.display());
                // keep a backup so the user doesn't lose data
                let _ = std::fs::copy(&path, path.with_extension("json.bak"));
                Workspace { id: id.to_string(), ..Default::default() }
            }
        },
        Err(_) => Workspace { id: id.to_string(), ..Default::default() },
    }
}

pub fn save(ws: &Workspace) {
    let mut ws = ws.clone();
    ws.history.truncate(MAX_HISTORY);
    match serde_json::to_string_pretty(&ws) {
        Ok(text) => write_atomic(&project_path(&ws.id), &text),
        Err(e) => eprintln!("pigeon: failed to serialize project: {e}"),
    }
}

pub fn delete_project(id: &str) {
    let _ = std::fs::remove_file(project_path(id));
    let _ = std::fs::remove_file(session_path(id));
    remove_project_images(id);
}

fn icons_dir() -> PathBuf {
    sub_dir("icons")
}

fn remove_project_images(project_id: &str) {
    if let Ok(entries) = std::fs::read_dir(icons_dir()) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(project_id) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Copies a user-chosen image into Pigeon's data dir (so it survives the original moving)
/// and returns the stored path. A new file name per upload avoids stale texture caches.
/// Stored image paths are resolved by file name inside the icons dir, so they keep working
/// if the data dir moves (e.g. the Courier -> Pigeon rename).
pub fn resolve_project_image(path: &str) -> PathBuf {
    let p = std::path::Path::new(path);
    match p.file_name() {
        Some(name) if !p.exists() => icons_dir().join(name),
        _ => p.to_path_buf(),
    }
}

pub fn store_project_image(project_id: &str, source: &std::path::Path) -> std::io::Result<PathBuf> {
    let ext = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
    remove_project_images(project_id);
    let dest = icons_dir().join(format!("{project_id}-{}.{ext}", chrono::Utc::now().timestamp_millis()));
    std::fs::copy(source, &dest)?;
    Ok(dest)
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn load_settings() -> Settings {
    std::fs::read_to_string(settings_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_settings(settings: &Settings) {
    if let Ok(text) = serde_json::to_string_pretty(settings) {
        write_atomic(&settings_path(), &text);
    }
}
