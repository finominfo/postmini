//! Workspace persistence. Everything is stored as plain Postman-compatible JSON files:
//!   <data>/collections/<id>.postman_collection.json
//!   <data>/environments/<id>.postman_environment.json
//!   <data>/globals.json, history.json, settings.json, state.json, cookies.json
//! Data dir: `postmini-data` next to the executable (portable mode) if it exists,
//! else $POSTMINI_HOME, else the OS data dir (%APPDATA%\postmini on Windows).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::http::CookieJar;
use crate::model::{Collection, Environment, Request};
use crate::settings::Settings;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct HistoryEntry {
    pub id: String,
    pub time: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub time_ms: f64,
    pub request: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct State {
    pub active_env: Option<String>,
    #[serde(default)]
    pub open_tabs: Vec<String>,
    #[serde(default)]
    pub expanded: Vec<String>,
}

pub fn data_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join("postmini-data");
            if p.is_dir() {
                return p;
            }
        }
    }
    if let Ok(h) = std::env::var("POSTMINI_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("postmini")
}

pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

pub fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' || c == '.' { c } else { '_' }).collect();
    let n = n.trim().to_string();
    if n.is_empty() {
        "untitled".into()
    } else {
        n
    }
}

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(dir.join("collections"));
        let _ = std::fs::create_dir_all(dir.join("environments"));
        Store { dir }
    }

    fn coll_path(&self, id: &str) -> PathBuf {
        self.dir.join("collections").join(format!("{}.postman_collection.json", safe_name(id)))
    }
    fn env_path(&self, id: &str) -> PathBuf {
        self.dir.join("environments").join(format!("{}.postman_environment.json", safe_name(id)))
    }

    pub fn load_collections(&self) -> Vec<Collection> {
        let mut out = vec![];
        if let Ok(rd) = std::fs::read_dir(self.dir.join("collections")) {
            for e in rd.flatten() {
                if e.path().extension().map(|x| x == "json").unwrap_or(false) {
                    if let Ok(t) = std::fs::read_to_string(e.path()) {
                        if let Ok(c) = crate::model::parse_collection(&t) {
                            if out.iter().any(|x: &Collection| x.id() == c.id()) {
                                continue;
                            }
                            // files dropped in manually (other name / missing ids): normalize to <id>.postman_collection.json
                            let canon = self.coll_path(c.id());
                            if canon != e.path() {
                                if self.save_collection(&c).is_ok() {
                                    let _ = std::fs::remove_file(e.path());
                                }
                            }
                            out.push(c);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.info.name.to_lowercase().cmp(&b.info.name.to_lowercase()));
        out
    }
    pub fn save_collection(&self, c: &Collection) -> Result<(), String> {
        let s = serde_json::to_string_pretty(c).map_err(|e| e.to_string())?;
        write_atomic(&self.coll_path(c.id()), s.as_bytes()).map_err(|e| e.to_string())
    }
    pub fn delete_collection(&self, id: &str) {
        let _ = std::fs::rename(self.coll_path(id), self.dir.join(format!("deleted-{}.json", safe_name(id))));
    }

    pub fn load_environments(&self) -> Vec<Environment> {
        let mut out = vec![];
        if let Ok(rd) = std::fs::read_dir(self.dir.join("environments")) {
            for e in rd.flatten() {
                if let Ok(t) = std::fs::read_to_string(e.path()) {
                    if let Ok(env) = Environment::parse(&t) {
                        out.push(env);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }
    pub fn save_environment(&self, e: &Environment) -> Result<(), String> {
        let mut e = e.clone();
        e.values.retain(|v| !v.key.is_empty());
        let s = serde_json::to_string_pretty(&e).map_err(|e| e.to_string())?;
        write_atomic(&self.env_path(&e.id), s.as_bytes()).map_err(|e| e.to_string())
    }
    pub fn delete_environment(&self, id: &str) {
        let _ = std::fs::remove_file(self.env_path(id));
    }

    pub fn load_globals(&self) -> Environment {
        std::fs::read_to_string(self.dir.join("globals.json"))
            .ok()
            .and_then(|t| Environment::parse(&t).ok())
            .map(|mut g| {
                g.scope = "globals".into();
                g
            })
            .unwrap_or_else(|| {
                let mut g = Environment::new("Globals");
                g.scope = "globals".into();
                g
            })
    }
    pub fn save_globals(&self, g: &Environment) {
        let mut g = g.clone();
        g.values.retain(|v| !v.key.is_empty());
        if let Ok(s) = serde_json::to_string_pretty(&g) {
            let _ = write_atomic(&self.dir.join("globals.json"), s.as_bytes());
        }
    }

    pub fn load<T: for<'de> Deserialize<'de> + Default>(&self, name: &str) -> T {
        std::fs::read_to_string(self.dir.join(name)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save<T: Serialize>(&self, name: &str, v: &T) {
        if let Ok(s) = serde_json::to_string(v) {
            let _ = write_atomic(&self.dir.join(name), s.as_bytes());
        }
    }
    pub fn load_settings(&self) -> Settings {
        std::fs::read_to_string(self.dir.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save_settings(&self, s: &Settings) {
        if let Ok(t) = serde_json::to_string_pretty(s) {
            let _ = write_atomic(&self.dir.join("settings.json"), t.as_bytes());
        }
    }
    pub fn jar(&self) -> CookieJar {
        crate::http::load_jar(&self.dir.join("cookies.json"))
    }
    pub fn save_jar(&self, jar: &CookieJar) {
        crate::http::save_jar(jar, &self.dir.join("cookies.json"));
    }
}
