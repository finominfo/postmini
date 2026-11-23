//! Local mock server: serves a collection's saved examples (like Postman mock servers,
//! but on localhost). Supports x-mock-response-name / x-mock-response-code headers,
//! path variables (`:id`) and {{var}} wildcards.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::model::{value_to_string, Collection};

#[derive(Clone, Debug)]
struct Route {
    method: String,
    segs: Vec<String>,
    query: Vec<(String, String)>,
    name: String,
    code: u16,
    status: String,
    headers: Vec<(String, String)>,
    body: String,
}

pub struct MockServer {
    pub port: u16,
    pub routes: usize,
    stop: Arc<AtomicBool>,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn path_of(raw: &str) -> (String, Vec<(String, String)>) {
    let raw = raw.split('#').next().unwrap_or("");
    let (base, q) = match raw.find('?') {
        Some(i) => (&raw[..i], Some(&raw[i + 1..])),
        None => (raw, None),
    };
    let rest = if let Some(i) = base.find("://") {
        let r = &base[i + 3..];
        r.find('/').map(|j| &r[j..]).unwrap_or("/")
    } else if base.starts_with("{{") {
        match base.find("}}") {
            Some(i) => &base[i + 2..],
            None => base,
        }
    } else if base.starts_with('/') {
        base
    } else {
        base.find('/').map(|j| &base[j..]).unwrap_or("/")
    };
    let query = q
        .map(|q| q.split('&').filter(|p| !p.is_empty()).map(|p| match p.split_once('=') { Some((a, b)) => (a.to_string(), b.to_string()), None => (p.to_string(), String::new()) }).collect())
        .unwrap_or_default();
    (if rest.is_empty() { "/".into() } else { rest.to_string() }, query)
}

fn segs(p: &str) -> Vec<String> {
    p.split('/').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
}

fn build_routes(c: &Collection) -> Vec<Route> {
    let mut out = vec![];
    for (_, item) in c.requests() {
        for ex in &item.response {
            let orig = ex.get("originalRequest");
            let (method, raw) = match orig {
                Some(o) => (
                    o.get("method").map(value_to_string).unwrap_or_else(|| "GET".into()),
                    o.get("url").map(|u| match u {
                        Value::String(s) => s.clone(),
                        o => o.get("raw").map(value_to_string).unwrap_or_default(),
                    }).unwrap_or_default(),
                ),
                None => (item.request.as_ref().map(|r| r.method.clone()).unwrap_or_else(|| "GET".into()), item.request.as_ref().map(|r| r.url.raw.clone()).unwrap_or_default()),
            };
            let (p, query) = path_of(&raw);
            let headers: Vec<(String, String)> = ex
                .get("header")
                .and_then(|h| h.as_array())
                .map(|a| a.iter().filter_map(|h| Some((h.get("key")?.as_str()?.to_string(), h.get("value").map(value_to_string).unwrap_or_default()))).collect())
                .unwrap_or_default();
            let code = ex.get("code").and_then(|c| c.as_u64()).unwrap_or(200) as u16;
            out.push(Route {
                method: method.to_uppercase(),
                segs: segs(&p),
                query,
                name: ex.get("name").map(value_to_string).unwrap_or_default(),
                code,
                status: ex.get("status").map(value_to_string).filter(|s| !s.is_empty()).unwrap_or_else(|| crate::http::status_text(code).to_string()),
                headers,
                body: ex.get("body").map(value_to_string).unwrap_or_default(),
            });
        }
    }
    out
}

fn seg_match(pattern: &str, actual: &str) -> Option<u32> {
    if pattern == actual {
        Some(3)
    } else if pattern.starts_with(':') || (pattern.starts_with("{{") && pattern.ends_with("}}")) {
        Some(1)
    } else {
        None
    }
}

fn find_route<'a>(routes: &'a [Route], method: &str, path: &str, query: &[(String, String)], headers: &[(String, String)]) -> Option<&'a Route> {
    let hget = |k: &str| headers.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let want_name = hget("x-mock-response-name");
    let want_code: Option<u16> = hget("x-mock-response-code").and_then(|c| c.parse().ok());
    let actual = segs(path);
    let mut best: Option<(u32, &Route)> = None;
    for r in routes {
        if r.method != method || r.segs.len() != actual.len() {
            continue;
        }
        if let Some(n) = &want_name {
            if &r.name != n {
                continue;
            }
        }
        if let Some(c) = want_code {
            if r.code != c {
                continue;
            }
        }
        let mut score = 0;
        let mut ok = true;
        for (p, a) in r.segs.iter().zip(actual.iter()) {
            match seg_match(p, a) {
                Some(s) => score += s,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // query params matching adds score
        for (k, v) in &r.query {
            if query.iter().any(|(a, b)| a == k && b == v) {
                score += 2;
            }
        }
        // prefer 2xx when no code requested
        if want_code.is_none() && (200..300).contains(&r.code) {
            score += 1;
        }
        if best.map(|(s, _)| score > s).unwrap_or(true) {
            best = Some((score, r));
        }
    }
    best.map(|(_, r)| r)
}

pub fn start(c: &Collection, port: u16, delay_ms: u64) -> Result<MockServer, String> {
    let routes = build_routes(c);
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("Cannot bind port {port}: {e}"))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let log = Arc::new(Mutex::new(Vec::new()));
    let n = routes.len();
    let (stop2, log2) = (stop.clone(), log.clone());
    std::thread::Builder::new()
        .name("mock".into())
        .spawn(move || {
            let routes = Arc::new(routes);
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        let routes = routes.clone();
                        let log = log2.clone();
                        std::thread::spawn(move || handle(s, &routes, &log, delay_ms));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(50)),
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(MockServer { port, routes: n, stop, log })
}

fn handle(s: std::net::TcpStream, routes: &[Route], log: &Mutex<Vec<String>>, delay_ms: u64) {
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut w = match s.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut rd = BufReader::new(s);
    let mut line = String::new();
    if rd.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = vec![];
    loop {
        let mut l = String::new();
        if rd.read_line(&mut l).is_err() {
            break;
        }
        let l = l.trim_end().to_string();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let cl: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; cl.min(50 * 1024 * 1024)];
    let _ = rd.read_exact(&mut body);
    let (path, query) = path_of(&target);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
    let cors = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\nAccess-Control-Allow-Methods: *\r\n";
    if method == "OPTIONS" {
        let _ = write!(w, "HTTP/1.1 204 No Content\r\n{cors}Content-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    let (code, status, hs, b) = match find_route(routes, &method, &path, &query, &headers) {
        Some(r) => (r.code, r.status.clone(), r.headers.clone(), r.body.clone()),
        None => (
            404,
            "Not Found".to_string(),
            vec![("Content-Type".to_string(), "application/json".to_string())],
            serde_json::json!({"error": {"name": "mockRequestNotFoundError", "message": format!("Double check your method and the request path and try again. No example found for {method} {path}")}}).to_string(),
        ),
    };
    if let Ok(mut l) = log.lock() {
        l.push(format!("{} {} {} → {}", chrono::Local::now().format("%H:%M:%S"), method, target, code));
        if l.len() > 500 {
            l.remove(0);
        }
    }
    let mut out = format!("HTTP/1.1 {code} {status}\r\n{cors}");
    for (k, v) in &hs {
        let lk = k.to_lowercase();
        if lk == "content-length" || lk == "transfer-encoding" || lk == "content-encoding" || lk == "connection" {
            continue;
        }
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!("x-srv-trace: postmini-mock\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()));
    let _ = w.write_all(out.as_bytes());
    if method != "HEAD" {
        let _ = w.write_all(b.as_bytes());
    }
    let _ = w.flush();
}

#[cfg(test)]
mod tests {
    #[test]
    fn mock_serves_examples() {
        let mut c = crate::model::Collection::new("m");
        let mut it = crate::model::Item::new_request("u", crate::model::Request { url: crate::model::Url::from_raw("{{baseUrl}}/users/:id"), ..Default::default() });
        it.response.push(serde_json::json!({"name": "ok", "code": 200, "originalRequest": {"method": "GET", "url": {"raw": "{{baseUrl}}/users/:id"}}, "header": [{"key": "Content-Type", "value": "application/json"}], "body": "{\"id\":1}"}));
        it.response.push(serde_json::json!({"name": "missing", "code": 404, "originalRequest": {"method": "GET", "url": "{{baseUrl}}/users/:id"}, "body": "nope"}));
        c.item.push(it);
        let m = super::start(&c, 0, 0).unwrap();
        let get = |extra: Vec<(String, String)>| {
            let r = crate::http::HttpRequest { method: "GET".into(), url: format!("{}/users/42", m.url()), headers: extra, body: vec![] };
            crate::http::execute(&r, &crate::http::HttpOptions::default()).unwrap()
        };
        let r = get(vec![]);
        assert_eq!(r.status, 200);
        assert_eq!(r.text(), "{\"id\":1}");
        let r = get(vec![("x-mock-response-code".into(), "404".into())]);
        assert_eq!(r.status, 404);
        let r = get(vec![("x-mock-response-name".into(), "missing".into())]);
        assert_eq!(r.text(), "nope");
    }
}
