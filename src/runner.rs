//! Request lifecycle + collection runner (mirrors postman-runtime's item/waterfall commands).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::http::{CookieJar, HttpResponse};
use crate::model::{get_script, Collection, Item, Request};
use crate::prepare;
use crate::script::{self, ScriptRun, TestResult};
use crate::settings::Settings;
use crate::vars::{Scope, VarContext};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
    pub time: String,
}

impl ConsoleEntry {
    pub fn new(level: &str, text: impl Into<String>) -> Self {
        ConsoleEntry { level: level.into(), text: text.into(), time: chrono::Local::now().format("%H:%M:%S%.3f").to_string() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Execution {
    pub name: String,
    pub id: String,
    pub response: Option<HttpResponse>,
    pub error: Option<String>,
    pub tests: Vec<TestResult>,
    pub console: Vec<ConsoleEntry>,
    pub script_errors: Vec<String>,
    pub next: Option<Option<String>>,
    pub skipped: bool,
    pub visualizer: Option<Value>,
    pub method: String,
    pub url: String,
}

impl Execution {
    pub fn passed(&self) -> usize {
        self.tests.iter().filter(|t| t.passed && !t.skipped).count()
    }
    pub fn failed(&self) -> usize {
        self.tests.iter().filter(|t| !t.passed && !t.skipped).count()
    }
}

/// Where the request lives (for inheriting scripts/auth/settings)
pub struct Location<'a> {
    pub collection: Option<&'a Collection>,
    pub path: Vec<usize>,
}

fn parent_scripts(loc: &Location, listen: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    if let Some(c) = loc.collection {
        let s = get_script(&c.event, listen);
        if !s.trim().is_empty() {
            out.push((format!("collection \"{}\"", c.info.name), s));
        }
        if !loc.path.is_empty() {
            let chain = c.chain(&loc.path);
            for it in chain.iter().take(chain.len().saturating_sub(1)) {
                let s = get_script(&it.event, listen);
                if !s.trim().is_empty() {
                    out.push((format!("folder \"{}\"", it.name), s));
                }
            }
        }
    }
    out
}

fn effective_auth(loc: &Location, own: Option<&crate::model::Auth>) -> Option<crate::model::Auth> {
    if let Some(a) = own {
        if a.kind != "inherit" {
            return Some(a.clone());
        }
    }
    match loc.collection {
        Some(c) => {
            if loc.path.is_empty() {
                c.auth.clone().filter(|a| a.kind != "inherit")
            } else {
                c.effective_auth(&loc.path, None)
            }
        }
        None => None,
    }
}

fn behavior(loc: &Location, item: &Item) -> Map<String, Value> {
    let mut m = match loc.collection {
        Some(c) if !loc.path.is_empty() => {
            let mut p = loc.path.clone();
            p.pop();
            let mut b = c.protocol_profile_behavior.clone().unwrap_or_default();
            if !p.is_empty() {
                for (k, v) in c.behavior(&p) {
                    b.insert(k, v);
                }
            }
            b
        }
        Some(c) => c.protocol_profile_behavior.clone().unwrap_or_default(),
        None => Map::new(),
    };
    if let Some(b) = &item.protocol_profile_behavior {
        for (k, v) in b {
            m.insert(k.clone(), v.clone());
        }
    }
    m
}

pub fn cookies_for(jar: &Option<CookieJar>, url: &str) -> Value {
    let Some(jar) = jar else { return json!([]) };
    let Ok(u) = url::Url::parse(url) else { return json!([]) };
    let store = jar.lock().unwrap();
    Value::Array(store.matches(&u).iter().map(|c| json!({"name": c.name(), "value": c.value(), "domain": u.host_str(), "path": c.path().unwrap_or("/")})).collect())
}

pub struct IterInfo {
    pub iteration: usize,
    pub count: usize,
}

/// Execute one request item through its full lifecycle.
pub fn execute_item(item: &Item, loc: &Location, vars: &mut VarContext, settings: &Settings, jar: Option<CookieJar>, it: &IterInfo) -> Execution {
    let mut ex = Execution { name: item.name.clone(), id: item.id().to_string(), ..Default::default() };
    let Some(req0) = &item.request else {
        ex.error = Some("Item has no request".into());
        return ex;
    };
    let mut req: Request = req0.clone();
    let path_names: Vec<String> = match loc.collection {
        Some(c) if !loc.path.is_empty() => c.chain(&loc.path).iter().map(|i| i.name.clone()).collect(),
        _ => vec![item.name.clone()],
    };
    let info = |event: &str| json!({"eventName": event, "iteration": it.iteration, "iterationCount": it.count, "requestName": item.name, "requestId": item.id(), "path": path_names, "current": item.name});

    // ---- pre-request
    let mut pre = parent_scripts(loc, "prerequest");
    let own = get_script(&item.event, "prerequest");
    if !own.trim().is_empty() {
        pre.push((format!("request \"{}\"", item.name), own));
    }
    if script::has_code(&pre) {
        let before = script::request_json(&req, &item.name, item.id());
        let pre_url = vars.resolve(&req.url.raw);
        let r = script::run(
            ScriptRun { event: "prerequest", scripts: &pre, request: before.clone(), response: None, cookies: cookies_for(&jar, &pre_url), info: info("prerequest"), settings, jar: jar.clone() },
            vars,
        );
        for (l, t) in r.console {
            ex.console.push(ConsoleEntry::new(&l, t));
        }
        ex.tests.extend(r.tests);
        if let Some(after) = &r.request {
            script::apply_request_changes(&mut req, &normalize_req(&before), &normalize_req(after));
        }
        if r.next.is_some() {
            ex.next = r.next.clone();
        }
        if !r.errors.is_empty() {
            for e in &r.errors {
                ex.console.push(ConsoleEntry::new("error", e.clone()));
            }
            ex.script_errors = r.errors.clone();
            ex.error = Some(format!("There was an error in evaluating the Pre-request Script: {}", r.errors.join("; ")));
            return ex;
        }
        if r.skip {
            ex.skipped = true;
            ex.console.push(ConsoleEntry::new("info", format!("Request \"{}\" skipped by pm.execution.skipRequest()", item.name)));
            return ex;
        }
    }

    // ---- send
    let auth = effective_auth(loc, req.auth.as_ref());
    let beh = behavior(loc, item);
    let prepared = match prepare::prepare(&req, auth.as_ref(), vars, &beh, settings, jar.clone()) {
        Ok(p) => p,
        Err(e) => {
            ex.console.push(ConsoleEntry::new("error", e.clone()));
            ex.error = Some(e);
            return ex;
        }
    };
    ex.method = prepared.req.method.clone();
    ex.url = prepared.req.url.clone();
    let resp = match prepare::send(&prepared) {
        Ok(r) => r,
        Err(e) => {
            ex.console.push(ConsoleEntry::new("error", format!("{} {}\nError: {}", prepared.req.method, prepared.req.url, e)));
            ex.error = Some(e);
            return ex;
        }
    };
    {
        let mut t = format!("{} {}  →  {} {}  ·  {:.0} ms  ·  {}\n", resp.sent.method, resp.sent.url, resp.status, resp.reason, resp.timings.total_ms, human_size(resp.size_total()));
        for (k, v) in &resp.redirects {
            t.push_str(&format!("  ↳ redirect {k} from {v}\n"));
        }
        t.push_str("Request Headers\n");
        for (k, v) in &resp.sent.headers {
            t.push_str(&format!("  {k}: {v}\n"));
        }
        if !prepared.req.body.is_empty() {
            let b = String::from_utf8_lossy(&prepared.req.body);
            t.push_str(&format!("Request Body\n  {}\n", truncate(&b, 4000)));
        }
        t.push_str("Response Headers\n");
        for (k, v) in &resp.headers {
            t.push_str(&format!("  {k}: {v}\n"));
        }
        let ct = resp.content_type();
        if ct.contains("json") || ct.contains("text") || ct.contains("xml") || ct.contains("html") {
            t.push_str(&format!("Response Body\n  {}", truncate(&resp.text(), 4000)));
        }
        ex.console.push(ConsoleEntry::new("net", t));
    }

    // ---- tests
    let mut tests = parent_scripts(loc, "test");
    let own = get_script(&item.event, "test");
    if !own.trim().is_empty() {
        tests.push((format!("request \"{}\"", item.name), own));
    }
    if script::has_code(&tests) {
        let r = script::run(
            ScriptRun {
                event: "test",
                scripts: &tests,
                request: script::request_json(&req, &item.name, item.id()),
                response: Some(script::response_json(&resp)),
                cookies: cookies_for(&jar, &resp.sent.url),
                info: info("test"),
                settings,
                jar: jar.clone(),
            },
            vars,
        );
        for (l, t) in r.console {
            ex.console.push(ConsoleEntry::new(&l, t));
        }
        ex.tests.extend(r.tests);
        if r.next.is_some() {
            ex.next = r.next;
        }
        if r.visualizer.is_some() {
            ex.visualizer = r.visualizer;
        }
        for e in &r.errors {
            ex.console.push(ConsoleEntry::new("error", format!("Test script error: {e}")));
        }
        ex.script_errors.extend(r.errors);
    }
    ex.response = Some(resp);
    ex
}

fn normalize_req(v: &Value) -> Value {
    let mut out = Map::new();
    for k in ["method", "url", "header", "body", "auth"] {
        if let Some(x) = v.get(k) {
            let x = if k == "header" {
                Value::Array(
                    x.as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|h| json!({"key": h.get("key").cloned().unwrap_or(Value::Null), "value": h.get("value").cloned().unwrap_or(json!("")), "disabled": h.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false)}))
                        .collect(),
                )
            } else if k == "body" {
                let mut b = Map::new();
                for bk in ["mode", "raw", "urlencoded", "formdata", "graphql", "file"] {
                    if let Some(bv) = x.get(bk) {
                        let bv = if bk == "urlencoded" || bk == "formdata" {
                            Value::Array(bv.as_array().cloned().unwrap_or_default().into_iter().map(|h| json!({"key": h.get("key"), "value": h.get("value").cloned().unwrap_or(json!("")), "disabled": h.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false), "type": h.get("type"), "src": h.get("src")})).collect())
                        } else {
                            bv.clone()
                        };
                        if !(bv.is_array() && bv.as_array().unwrap().is_empty()) {
                            b.insert(bk.into(), bv);
                        }
                    }
                }
                Value::Object(b)
            } else {
                x.clone()
            };
            out.insert(k.into(), x);
        }
    }
    Value::Object(out)
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut e = n;
        while !s.is_char_boundary(e) {
            e -= 1;
        }
        format!("{}… ({} more bytes)", &s[..e], s.len() - e)
    }
}

pub fn human_size(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.2} KB", n as f64 / 1024.0)
    } else {
        format!("{:.2} MB", n as f64 / 1024.0 / 1024.0)
    }
}

// ------------------------------------------------------------------ data files

pub fn parse_data_file(path: &str) -> Result<Vec<Map<String, Value>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read data file {path}: {e}"))?;
    parse_data(&text, path.to_lowercase().ends_with(".json"))
}

pub fn parse_data(text: &str, json_hint: bool) -> Result<Vec<Map<String, Value>>, String> {
    let t = text.trim_start_matches('\u{feff}').trim();
    if json_hint || t.starts_with('[') {
        let v: Value = serde_json::from_str(t).map_err(|e| format!("Invalid JSON data file: {e}"))?;
        let arr = v.as_array().ok_or("JSON data file must contain an array of objects")?;
        return Ok(arr.iter().map(|o| o.as_object().cloned().unwrap_or_default()).collect());
    }
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(t.as_bytes());
    let headers: Vec<String> = rdr.headers().map_err(|e| e.to_string())?.iter().map(|s| s.to_string()).collect();
    let mut out = vec![];
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let mut m = Map::new();
        for (i, h) in headers.iter().enumerate() {
            m.insert(h.clone(), Value::String(rec.get(i).unwrap_or("").to_string()));
        }
        out.push(m);
    }
    Ok(out)
}

// ------------------------------------------------------------------ collection runner

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub iterations: usize,
    pub delay_ms: u64,
    pub data: Vec<Map<String, Value>>,
    pub bail: bool,
    /// item ids to run (in this order); None = all
    pub selected: Option<Vec<String>>,
    /// run only within this folder path
    pub folder: Option<Vec<usize>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemResult {
    pub iteration: usize,
    pub name: String,
    pub id: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub reason: String,
    pub time_ms: f64,
    pub size: usize,
    pub tests: Vec<TestResult>,
    pub error: Option<String>,
    pub skipped: bool,
    #[serde(skip)]
    pub response_body: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunSummary {
    pub iterations: usize,
    pub requests: usize,
    pub failed_requests: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    pub tests_skipped: usize,
    pub total_time_ms: f64,
    pub avg_response_ms: f64,
    pub results: Vec<ItemResult>,
    pub stopped: bool,
}

pub enum RunEvent {
    IterationStart(usize),
    Item(ItemResult, Vec<ConsoleEntry>),
    Done(RunSummary, VarContext),
}

/// Runs a collection. `vars` holds environment/globals/collection scopes at start.
pub fn run_collection(coll: &Collection, mut vars: VarContext, opts: &RunOptions, settings: &Settings, jar: Option<CookieJar>, cancel: Arc<AtomicBool>, mut cb: impl FnMut(RunEvent)) -> RunSummary {
    let start = std::time::Instant::now();
    let all: Vec<(Vec<usize>, &Item)> = coll
        .requests()
        .into_iter()
        .filter(|(p, _)| match &opts.folder {
            Some(f) => p.starts_with(f),
            None => true,
        })
        .collect();
    let seq: Vec<(Vec<usize>, &Item)> = match &opts.selected {
        Some(ids) => ids.iter().filter_map(|id| all.iter().find(|(_, i)| i.id() == id).cloned()).collect(),
        None => all.clone(),
    };
    let iterations = if opts.iterations == 0 { opts.data.len().max(1) } else { opts.iterations };
    let mut summary = RunSummary { iterations, ..Default::default() };
    let mut total_resp = 0.0;
    'outer: for iter in 0..iterations {
        if cancel.load(Ordering::Relaxed) {
            summary.stopped = true;
            break;
        }
        cb(RunEvent::IterationStart(iter));
        vars.data = opts.data.get(iter % opts.data.len().max(1)).map(Scope::from_map).unwrap_or_default();
        let mut idx = 0usize;
        let mut guard = 0usize;
        while idx < seq.len() {
            guard += 1;
            if guard > 100_000 || cancel.load(Ordering::Relaxed) {
                summary.stopped = true;
                break 'outer;
            }
            let (path, item) = &seq[idx];
            let loc = Location { collection: Some(coll), path: path.clone() };
            let ex = execute_item(item, &loc, &mut vars, settings, jar.clone(), &IterInfo { iteration: iter, count: iterations });
            let r = ItemResult {
                iteration: iter,
                name: item.name.clone(),
                id: item.id().to_string(),
                method: ex.method.clone(),
                url: ex.url.clone(),
                status: ex.response.as_ref().map(|r| r.status).unwrap_or(0),
                reason: ex.response.as_ref().map(|r| r.reason.clone()).unwrap_or_default(),
                time_ms: ex.response.as_ref().map(|r| r.timings.total_ms).unwrap_or(0.0),
                size: ex.response.as_ref().map(|r| r.size_total()).unwrap_or(0),
                tests: ex.tests.clone(),
                error: ex.error.clone().or_else(|| if ex.script_errors.is_empty() { None } else { Some(ex.script_errors.join("; ")) }),
                skipped: ex.skipped,
                response_body: ex.response.as_ref().map(|r| truncate(&r.text(), 20000)).unwrap_or_default(),
            };
            summary.requests += 1;
            if r.error.is_some() && ex.response.is_none() && !ex.skipped {
                summary.failed_requests += 1;
            }
            summary.tests_passed += ex.passed();
            summary.tests_failed += ex.failed();
            summary.tests_skipped += ex.tests.iter().filter(|t| t.skipped).count();
            total_resp += r.time_ms;
            let bail_now = opts.bail && (r.error.is_some() || ex.failed() > 0);
            cb(RunEvent::Item(r.clone(), ex.console.clone()));
            summary.results.push(r);
            if bail_now {
                summary.stopped = true;
                break 'outer;
            }
            // setNextRequest
            match &ex.next {
                Some(None) => break,
                Some(Some(name)) => match seq.iter().position(|(_, i)| &i.name == name || i.id() == name) {
                    Some(p) => idx = p,
                    None => {
                        cb(RunEvent::Item(ItemResult { iteration: iter, name: format!("setNextRequest(\"{name}\")"), error: Some(format!("Request \"{name}\" not found; stopping iteration")), ..Default::default() }, vec![]));
                        break;
                    }
                },
                None => idx += 1,
            }
            if opts.delay_ms > 0 && idx < seq.len() {
                std::thread::sleep(std::time::Duration::from_millis(opts.delay_ms));
            }
        }
    }
    summary.total_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    summary.avg_response_ms = if summary.requests > 0 { total_resp / summary.requests as f64 } else { 0.0 };
    cb(RunEvent::Done(summary.clone(), vars));
    summary
}
