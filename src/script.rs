//! Script sandbox: runs pre-request / test scripts in QuickJS with a Postman-compatible `pm` API.
//! A runtime is created per execution and dropped afterwards, so it costs no memory when idle.

use std::time::{Duration, Instant};

use rquickjs::{Coerced, Context, Function, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{CookieJar, HttpResponse};
use crate::settings::Settings;
use crate::vars::{dynamic_variable, VarContext};

const PRELUDE: &str = include_str!("../js/prelude.js");

fn lib_source(name: &str) -> Option<&'static str> {
    Some(match name {
        "chai" => include_str!("../js/lib/chai.min.js"),
        "lodash" => include_str!("../js/lib/lodash.min.js"),
        "crypto-js" => include_str!("../js/lib/crypto-js.min.js"),
        "moment" => include_str!("../js/lib/moment.min.js"),
        "ajv" => include_str!("../js/lib/ajv.min.js"),
        "tv4" => include_str!("../js/lib/tv4.min.js"),
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ScriptResult {
    pub tests: Vec<TestResult>,
    pub console: Vec<(String, String)>,
    pub errors: Vec<String>,
    /// Some(None) => setNextRequest(null) i.e. stop the run
    pub next: Option<Option<String>>,
    pub skip: bool,
    pub request: Option<Value>,
    pub visualizer: Option<Value>,
}

pub struct ScriptRun<'a> {
    pub event: &'a str,
    pub scripts: &'a [(String, String)],
    pub request: Value,
    pub response: Option<Value>,
    pub cookies: Value,
    pub info: Value,
    pub settings: &'a Settings,
    pub jar: Option<CookieJar>,
}

/// Response representation passed to scripts (pm.response)
pub fn response_json(r: &HttpResponse) -> Value {
    let headers: Vec<Value> = r.headers.iter().map(|(k, v)| json!({"key": k, "value": v})).collect();
    let text = r.text();
    let ct = r.content_type();
    let binary = !(ct.is_empty() || ct.contains("json") || ct.contains("text") || ct.contains("xml") || ct.contains("javascript") || ct.contains("html") || ct.contains("x-www-form-urlencoded"));
    let mut v = json!({
        "code": r.status,
        "status": r.reason,
        "header": headers,
        "body": text,
        "responseTime": r.timings.total_ms.round() as i64,
        "responseSize": r.body.len(),
        "cookies": parse_set_cookies(r),
    });
    if binary {
        use base64::Engine;
        v["bodyBase64"] = Value::String(base64::engine::general_purpose::STANDARD.encode(&r.body));
    }
    v
}

pub fn parse_set_cookies(r: &HttpResponse) -> Value {
    let host = crate::http::parse_url(&r.sent.url, false).map(|u| u.host).unwrap_or_default();
    let list: Vec<Value> = r
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(_, v)| cookie::Cookie::parse(v.clone()).ok())
        .map(|c| {
            json!({
                "name": c.name(), "value": c.value(),
                "domain": c.domain().map(|d| d.to_string()).unwrap_or_else(|| host.clone()),
                "path": c.path().unwrap_or("/"),
                "expires": c.expires_datetime().map(|t| t.to_string()),
                "httpOnly": c.http_only().unwrap_or(false), "secure": c.secure().unwrap_or(false)
            })
        })
        .collect();
    Value::Array(list)
}

fn vars_json(v: &VarContext) -> Value {
    json!({
        "locals": v.locals.to_json(),
        "data": v.data.to_json(),
        "environment": v.environment.to_json(),
        "envName": v.env_name,
        "collection": v.collection.to_json(),
        "globals": v.globals.to_json(),
    })
}

fn native_send(def: &str, settings: &Settings, jar: Option<CookieJar>) -> String {
    let r: crate::model::Request = match serde_json::from_str(def) {
        Ok(r) => r,
        Err(e) => return json!({"error": format!("Invalid request: {e}")}).to_string(),
    };
    let empty = VarContext::default();
    let auth = r.auth.clone();
    let prepared = match crate::prepare::prepare(&r, auth.as_ref(), &empty, &serde_json::Map::new(), settings, jar) {
        Ok(p) => p,
        Err(e) => return json!({ "error": e }).to_string(),
    };
    match crate::prepare::send(&prepared) {
        Ok(resp) => response_json(&resp).to_string(),
        Err(e) => json!({ "error": e }).to_string(),
    }
}

fn native_jar(op: &str, args: &str, jar: &Option<CookieJar>) -> String {
    let Some(jar) = jar else { return json!({"error": "Cookie jar is not available"}).to_string() };
    let a: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let url_s = a.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
    let url_s = if url_s.contains("://") { url_s } else { format!("https://{url_s}") };
    let Ok(u) = url::Url::parse(&url_s) else { return json!({"error": format!("Invalid URL: {url_s}")}).to_string() };
    let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let mut store = jar.lock().unwrap();
    match op {
        "get" => {
            let v = store.get_request_values(&u).find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
            json!({ "result": v }).to_string()
        }
        "getAll" => {
            let all: Vec<Value> = store.matches(&u).iter().map(|c| json!({"name": c.name(), "value": c.value(), "path": c.path().unwrap_or("/"), "domain": u.host_str(), "httpOnly": c.http_only().unwrap_or(false), "secure": c.secure().unwrap_or(false)})).collect();
            json!({ "result": all }).to_string()
        }
        "set" => {
            let value = a.get("value").and_then(|n| n.as_str()).unwrap_or("");
            let mut s = format!("{name}={value}");
            if let Some(p) = a.get("path").and_then(|p| p.as_str()) {
                s.push_str(&format!("; Path={p}"));
            } else {
                s.push_str("; Path=/");
            }
            if let Some(d) = a.get("domain").and_then(|p| p.as_str()) {
                s.push_str(&format!("; Domain={d}"));
            }
            if a.get("httpOnly").and_then(|p| p.as_bool()).unwrap_or(false) {
                s.push_str("; HttpOnly");
            }
            if a.get("secure").and_then(|p| p.as_bool()).unwrap_or(false) {
                s.push_str("; Secure");
            }
            match store.parse(&s, &u) {
                Ok(_) => json!({"result": {"name": name, "value": value}}).to_string(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        }
        "unset" | "clear" => {
            let host = u.host_str().unwrap_or("").to_string();
            let victims: Vec<(String, String, String)> = store
                .matches(&u)
                .iter()
                .filter(|c| op == "clear" || c.name() == name)
                .map(|c| {
                    let d = match &c.domain {
                        cookie_store::CookieDomain::HostOnly(h) | cookie_store::CookieDomain::Suffix(h) => h.clone(),
                        _ => host.clone(),
                    };
                    (d, c.path().unwrap_or("/").to_string(), c.name().to_string())
                })
                .collect();
            for (d, p, n) in victims {
                store.remove(&d, &p, &n);
            }
            json!({"result": null}).to_string()
        }
        _ => json!({"error": "unknown op"}).to_string(),
    }
}

fn js_err(ctx: &rquickjs::Ctx, e: rquickjs::Error) -> String {
    if let rquickjs::Error::Exception = e {
        let v = ctx.catch();
        if let Some(ex) = v.as_exception() {
            let msg = ex.message().unwrap_or_default();
            let name: String = ex.get::<_, Option<String>>("name").ok().flatten().unwrap_or_else(|| "Error".into());
            if msg.contains("interrupted") {
                return "Script execution timed out".into();
            }
            return format!("{name}: {msg}");
        }
        return v.get::<Coerced<String>>().map(|c| c.0).unwrap_or_else(|_| "Unknown error".into());
    }
    e.to_string()
}

pub fn has_code(scripts: &[(String, String)]) -> bool {
    scripts.iter().any(|(_, c)| {
        c.lines().any(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//")
        })
    })
}

struct NativeEnv {
    settings: Settings,
    jar: Option<CookieJar>,
}

/// A QuickJS runtime + context with the prelude and libraries loaded, reused for
/// all script executions on the same thread (like postman-sandbox's persistent context).
struct Engine {
    rt: Runtime,
    ctx: Context,
    deadline: Arc<AtomicU64>,
    epoch: Instant,
    env: Rc<RefCell<NativeEnv>>,
}

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

impl Engine {
    fn new() -> Result<Engine, String> {
        let rt = Runtime::new().map_err(|e| format!("Cannot create JS runtime: {e}"))?;
        rt.set_memory_limit(256 * 1024 * 1024);
        rt.set_max_stack_size(2 * 1024 * 1024);
        let epoch = Instant::now();
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let d2 = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || epoch.elapsed().as_millis() as u64 > d2.load(Ordering::Relaxed))));
        let ctx = Context::full(&rt).map_err(|e| format!("Cannot create JS context: {e}"))?;
        let env = Rc::new(RefCell::new(NativeEnv { settings: Settings::default(), jar: None }));
        let e2 = env.clone();
        let e3 = env.clone();
        let d3 = deadline.clone();
        ctx.with(|ctx| -> Result<(), String> {
            let g = ctx.globals();
            let m = |e: rquickjs::Error| e.to_string();
            g.set("__dyn", Function::new(ctx.clone(), |name: String| -> Option<String> { dynamic_variable(&name) }).map_err(m)?).map_err(m)?;
            g.set("__lib", Function::new(ctx.clone(), |name: String| -> Option<String> { lib_source(&name).map(|s| s.to_string()) }).map_err(m)?).map_err(m)?;
            g.set(
                "__sleep",
                Function::new(ctx.clone(), move |ms: f64| {
                    let left = d3.load(Ordering::Relaxed).saturating_sub(epoch.elapsed().as_millis() as u64);
                    std::thread::sleep(Duration::from_millis((ms.max(0.0) as u64).min(left)));
                })
                .map_err(m)?,
            )
            .map_err(m)?;
            g.set(
                "__send",
                Function::new(ctx.clone(), move |def: String| -> String {
                    let (s, j) = {
                        let e = e2.borrow();
                        (e.settings.clone(), e.jar.clone())
                    };
                    native_send(&def, &s, j)
                })
                .map_err(m)?,
            )
            .map_err(m)?;
            g.set(
                "__jar",
                Function::new(ctx.clone(), move |op: String, args: String| -> String {
                    let j = e3.borrow().jar.clone();
                    native_jar(&op, &args, &j)
                })
                .map_err(m)?,
            )
            .map_err(m)?;
            ctx.eval::<(), _>(PRELUDE).map_err(|e| format!("prelude: {}", js_err(&ctx, e)))?;
            ctx.eval::<(), _>(
                "globalThis.__base = new Set(Object.getOwnPropertyNames(globalThis));\
                 globalThis.__clean = function () { Object.getOwnPropertyNames(globalThis).forEach(function (k) { if (!__base.has(k) && k.indexOf('__') !== 0) { try { delete globalThis[k]; } catch (e) {} } }); };",
            )
            .map_err(|e| js_err(&ctx, e))?;
            Ok(())
        })?;
        Ok(Engine { rt, ctx, deadline, epoch, env })
    }

    fn drain(&self) {
        loop {
            while self.rt.is_job_pending() {
                if self.timed_out() {
                    return;
                }
                let _ = self.rt.execute_pending_job();
            }
            let more = self.ctx.with(|ctx| ctx.eval::<bool, _>("__runNextTimer()").unwrap_or(false));
            if !more || self.timed_out() {
                break;
            }
        }
    }

    fn timed_out(&self) -> bool {
        self.epoch.elapsed().as_millis() as u64 > self.deadline.load(Ordering::Relaxed)
    }
}

/// Execute scripts and write back variable changes into `vars`.
pub fn run(run: ScriptRun, vars: &mut VarContext) -> ScriptResult {
    let mut result = ScriptResult::default();
    if !has_code(run.scripts) {
        return result;
    }
    ENGINE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            match Engine::new() {
                Ok(e) => *slot = Some(e),
                Err(e) => {
                    result.errors.push(e);
                    return;
                }
            }
        }
        let engine = slot.as_ref().unwrap();
        let ok = run_in(engine, &run, vars, &mut result);
        engine.deadline.store(u64::MAX, Ordering::Relaxed);
        if !ok {
            // timed out / broken: start fresh next time
            *slot = None;
        }
    });
    result
}

fn run_in(engine: &Engine, run: &ScriptRun, vars: &mut VarContext, result: &mut ScriptResult) -> bool {
    let timeout_ms = if run.settings.script_timeout_ms == 0 { 600_000 } else { run.settings.script_timeout_ms };
    engine.deadline.store(engine.epoch.elapsed().as_millis() as u64 + timeout_ms, Ordering::Relaxed);
    {
        let mut env = engine.env.borrow_mut();
        env.settings = run.settings.clone();
        env.jar = run.jar.clone();
    }
    let ctx_json = json!({
        "vars": vars_json(vars),
        "request": run.request,
        "response": run.response,
        "cookies": run.cookies,
        "info": run.info,
    })
    .to_string();
    let setup = engine.ctx.with(|ctx| -> Result<(), String> {
        ctx.eval::<(), _>("__clean();").map_err(|e| js_err(&ctx, e))?;
        ctx.globals().set("__ctxJson", ctx_json.as_str()).map_err(|e| e.to_string())?;
        ctx.eval::<(), _>("globalThis.__ctx = JSON.parse(globalThis.__ctxJson); delete globalThis.__ctxJson; __setup(globalThis);").map_err(|e| js_err(&ctx, e))?;
        Ok(())
    });
    if let Err(e) = setup {
        result.errors.push(e);
        return false;
    }
    let mut healthy = true;
    for (name, code) in run.scripts {
        if code.trim().is_empty() {
            continue;
        }
        let wrapped = format!("(async function () {{\n{}\n}})().catch(__reportError);", code);
        let r = engine.ctx.with(|ctx| ctx.eval::<(), _>(wrapped.as_str()).map_err(|e| js_err(&ctx, e)));
        if let Err(e) = r {
            result.errors.push(format!("{e} (in {} {})", name, run.event));
        }
        engine.drain();
        if engine.timed_out() {
            result.errors.push(format!("Script execution timed out after {timeout_ms} ms"));
            healthy = false;
            break;
        }
    }
    if !healthy {
        engine.deadline.store(u64::MAX, Ordering::Relaxed);
    }
    let out: Result<String, String> = engine.ctx.with(|ctx| ctx.eval::<String, _>("__finish()").map_err(|e| js_err(&ctx, e)));
    match out {
        Ok(s) => {
            let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
            if let Some(t) = v.get("tests") {
                result.tests = serde_json::from_value(t.clone()).unwrap_or_default();
            }
            if let Some(Value::Array(c)) = v.get("console") {
                for e in c {
                    if let Some(a) = e.as_array() {
                        result.console.push((a.first().and_then(|x| x.as_str()).unwrap_or("log").into(), a.get(1).and_then(|x| x.as_str()).unwrap_or("").into()));
                    }
                }
            }
            if let Some(Value::Array(es)) = v.get("errors") {
                for e in es {
                    result.errors.push(e.as_str().unwrap_or("").to_string());
                }
            }
            result.skip = v.get("skip").and_then(|s| s.as_bool()).unwrap_or(false);
            if let Some(n) = v.get("next") {
                result.next = Some(n.as_str().map(|s| s.to_string()));
            }
            if let Some(vz) = v.get("visualizer").filter(|v| !v.is_null()) {
                result.visualizer = Some(vz.clone());
            }
            result.request = v.get("request").cloned();
            if let Some(vs) = v.get("vars") {
                vars.locals.update_from_json(&vs["locals"]);
                vars.environment.update_from_json(&vs["environment"]);
                vars.collection.update_from_json(&vs["collection"]);
                vars.globals.update_from_json(&vs["globals"]);
            }
        }
        Err(e) => {
            result.errors.push(format!("finish: {e}"));
            healthy = false;
        }
    }
    // release per-run memory (response bodies etc.)
    let _ = engine.ctx.with(|ctx| ctx.eval::<(), _>("globalThis.__ctx = undefined; globalThis.pm = undefined;"));
    engine.rt.run_gc();
    healthy
}

/// Apply a request modified by a pre-request script back to the model.
pub fn apply_request_changes(r: &mut crate::model::Request, before: &Value, after: &Value) {
    if before == after {
        return;
    }
    if before.get("method") != after.get("method") {
        if let Some(m) = after.get("method").and_then(|m| m.as_str()) {
            r.method = m.to_string();
        }
    }
    if before.get("url") != after.get("url") {
        if let Some(u) = after.get("url").and_then(|m| m.as_str()) {
            r.url.raw = u.to_string();
            r.url.sync_from_raw();
        }
    }
    if before.get("header") != after.get("header") {
        if let Some(h) = after.get("header") {
            r.header = serde_json::from_value(h.clone()).unwrap_or_default();
        }
    }
    if before.get("body") != after.get("body") {
        if let Some(b) = after.get("body") {
            let mut nb: crate::model::Body = serde_json::from_value(b.clone()).unwrap_or_default();
            if let Some(old) = &r.body {
                if nb.options.is_none() {
                    nb.options = old.options.clone();
                }
            }
            r.body = Some(nb);
        }
    }
    if before.get("auth") != after.get("auth") {
        if let Some(a) = after.get("auth") {
            r.auth = serde_json::from_value(a.clone()).ok();
        }
    }
}

/// Request JSON as seen by scripts (pm.request)
pub fn request_json(r: &crate::model::Request, name: &str, id: &str) -> Value {
    let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("url".into(), Value::String(r.url.raw.clone()));
        m.insert("name".into(), Value::String(name.into()));
        m.insert("id".into(), Value::String(id.into()));
        m.insert("pathVariables".into(), json!(r.url.variable.iter().map(|v| json!({"key": v.key, "value": v.value})).collect::<Vec<_>>()));
    }
    v
}
