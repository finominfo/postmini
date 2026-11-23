//! Turns a collection Request (with {{variables}}) into a concrete HttpRequest,
//! like postman-runtime's request-helpers-presend + core-body-builder.

use serde_json::{Map, Value};

use crate::auth;
use crate::http::{self, CookieJar, HttpOptions, HttpRequest, HttpResponse};
use crate::model::{Auth, Request, KV};
use crate::settings::Settings;
use crate::vars::VarContext;

pub struct Prepared {
    pub req: HttpRequest,
    pub opts: HttpOptions,
    pub auth: Option<Auth>,
    pub form: Vec<(String, String)>,
}

fn bval(m: &Map<String, Value>, k: &str) -> Option<bool> {
    m.get(k).and_then(|v| v.as_bool())
}

pub fn content_type_for_lang(lang: &str) -> &'static str {
    match lang {
        "json" => "application/json",
        "xml" => "application/xml",
        "html" => "text/html",
        "javascript" => "application/javascript",
        _ => "text/plain",
    }
}

pub fn mime_for_path(p: &str) -> &'static str {
    let ext = p.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "js" => "application/javascript",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

pub fn resolve_file_path(p: &str, settings: &Settings) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(p);
    if path.is_absolute() || settings.working_dir.is_empty() {
        path
    } else {
        std::path::Path::new(&settings.working_dir).join(path)
    }
}

/// Resolve URL with variables and :path variables
pub fn resolve_url(r: &Request, vars: &VarContext) -> String {
    let raw = vars.resolve(&r.url.raw);
    if r.url.variable.is_empty() {
        return raw;
    }
    let (base, rest) = match raw.find(['?', '#']) {
        Some(i) => (raw[..i].to_string(), raw[i..].to_string()),
        None => (raw.clone(), String::new()),
    };
    let proto_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let path_start = base[proto_end..].find('/').map(|i| i + proto_end).unwrap_or(base.len());
    let (head, path) = base.split_at(path_start);
    let new_path: Vec<String> = path
        .split('/')
        .map(|seg| {
            if let Some(name) = seg.strip_prefix(':') {
                if let Some(v) = r.url.variable.iter().find(|v| v.key == name && !v.disabled) {
                    return vars.resolve(&v.value);
                }
            }
            seg.to_string()
        })
        .collect();
    format!("{}{}{}", head, new_path.join("/"), rest)
}

fn enabled(list: &[KV]) -> impl Iterator<Item = &KV> {
    list.iter().filter(|k| !k.disabled && !(k.key.is_empty() && k.value.is_empty() && k.src.is_none()))
}

pub fn resolve_auth(a: &Auth, vars: &VarContext) -> Auth {
    let mut out = a.clone();
    out.params = a
        .params
        .iter()
        .map(|(k, v)| {
            let nv = match v {
                Value::String(s) => Value::String(vars.resolve(s)),
                o => o.clone(),
            };
            (k.clone(), nv)
        })
        .collect();
    out
}

pub fn http_options(settings: &Settings, behavior: &Map<String, Value>, jar: Option<CookieJar>) -> HttpOptions {
    HttpOptions {
        timeout_ms: settings.timeout_ms,
        verify_ssl: bval(behavior, "strictSSL").unwrap_or(settings.verify_ssl),
        follow_redirects: bval(behavior, "followRedirects").unwrap_or(settings.follow_redirects),
        max_redirects: behavior.get("maxRedirects").and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(settings.max_redirects),
        follow_original_method: bval(behavior, "followOriginalHttpMethod").unwrap_or(false),
        follow_auth_header: bval(behavior, "followAuthorizationHeader").unwrap_or(false),
        remove_referer_on_redirect: bval(behavior, "removeRefererHeaderOnRedirect").unwrap_or(false),
        disable_cookies: bval(behavior, "disableCookies").unwrap_or(false),
        disable_url_encoding: bval(behavior, "disableUrlEncoding").unwrap_or(false),
        proxy: settings.effective_proxy(),
        client_certs: settings.client_certs.clone(),
        ca_file: if settings.ca_file.is_empty() { None } else { Some(settings.ca_file.clone()) },
        max_response_bytes: if settings.max_response_mb == 0 { usize::MAX } else { (settings.max_response_mb as usize) * 1024 * 1024 },
        jar,
    }
}

pub fn prepare(r: &Request, auth_def: Option<&Auth>, vars: &VarContext, behavior: &Map<String, Value>, settings: &Settings, jar: Option<CookieJar>) -> Result<Prepared, String> {
    let trim = |s: String| if settings.trim_keys_values { s.trim().to_string() } else { s };
    let url = resolve_url(r, vars);
    let method = { let m = vars.resolve(&r.method).trim().to_uppercase(); if m.is_empty() { "GET".to_string() } else { m } };
    let mut headers: Vec<(String, String)> = enabled(&r.header).filter(|h| !h.key.trim().is_empty()).map(|h| (trim(vars.resolve(&h.key)), trim(vars.resolve(&h.value)))).collect();
    let has = |hs: &Vec<(String, String)>, k: &str| hs.iter().any(|(h, _)| h.eq_ignore_ascii_case(k));
    let mut body: Vec<u8> = Vec::new();
    let mut form: Vec<(String, String)> = Vec::new();

    let prune = matches!(method.as_str(), "GET" | "HEAD" | "COPY" | "PURGE" | "UNLOCK") && !bval(behavior, "disableBodyPruning").unwrap_or(false);
    if let Some(b) = &r.body {
        if !b.disabled && !prune {
            match b.mode.as_str() {
                "raw" => {
                    let raw = vars.resolve(b.raw.as_deref().unwrap_or(""));
                    if !raw.is_empty() && !has(&headers, "content-type") {
                        headers.push(("Content-Type".into(), content_type_for_lang(&b.raw_language()).into()));
                    }
                    body = raw.into_bytes();
                }
                "urlencoded" => {
                    for kv in enabled(&b.urlencoded) {
                        form.push((trim(vars.resolve(&kv.key)), trim(vars.resolve(&kv.value))));
                    }
                    body = form.iter().map(|(k, v)| format!("{}={}", auth::pct(k), auth::pct(v))).collect::<Vec<_>>().join("&").into_bytes();
                    if !has(&headers, "content-type") {
                        headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
                    }
                }
                "formdata" => {
                    let boundary = format!("--------------------------{}", &uuid::Uuid::new_v4().simple().to_string()[..24]);
                    for kv in enabled(&b.formdata) {
                        let name = vars.resolve(&kv.key);
                        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                        if kv.is_file() {
                            let paths: Vec<String> = match &kv.src {
                                Some(Value::Array(a)) => a.iter().map(crate::model::value_to_string).collect(),
                                Some(Value::String(s)) => vec![s.clone()],
                                _ => vec![],
                            };
                            if paths.is_empty() {
                                body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n").as_bytes());
                                continue;
                            }
                            for (i, p) in paths.iter().enumerate() {
                                if i > 0 {
                                    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                                }
                                let path = resolve_file_path(p, settings);
                                let data = std::fs::read(&path).map_err(|e| format!("Cannot read form-data file {}: {e}", path.display()))?;
                                let fname = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                                let ct = kv.content_type.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| mime_for_path(&fname).to_string());
                                body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\nContent-Type: {ct}\r\n\r\n").as_bytes());
                                body.extend_from_slice(&data);
                                body.extend_from_slice(b"\r\n");
                            }
                        } else {
                            let val = vars.resolve(&kv.value);
                            body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes());
                            if let Some(ct) = kv.content_type.as_ref().filter(|c| !c.is_empty()) {
                                body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
                            }
                            body.extend_from_slice(b"\r\n");
                            body.extend_from_slice(val.as_bytes());
                            body.extend_from_slice(b"\r\n");
                        }
                    }
                    if !body.is_empty() {
                        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                    }
                    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
                    headers.push(("Content-Type".into(), format!("multipart/form-data; boundary={boundary}")));
                }
                "file" => {
                    let src = b.file_src();
                    if !src.is_empty() {
                        let path = resolve_file_path(&src, settings);
                        body = std::fs::read(&path).map_err(|e| format!("Cannot read binary body file {}: {e}", path.display()))?;
                        if !has(&headers, "content-type") {
                            headers.push(("Content-Type".into(), mime_for_path(&src).into()));
                        }
                    } else if let Some(c) = b.file.as_ref().and_then(|f| f.get("content")).and_then(|c| c.as_str()) {
                        body = c.as_bytes().to_vec();
                    }
                }
                "graphql" => {
                    if let Some(g) = &b.graphql {
                        let q = vars.resolve(&g.query);
                        let vs = vars.resolve(&g.variables);
                        let variables: Value = if vs.trim().is_empty() { Value::Object(Map::new()) } else { serde_json::from_str(&vs).map_err(|e| format!("GraphQL variables are not valid JSON: {e}"))? };
                        body = serde_json::json!({ "query": q, "variables": variables }).to_string().into_bytes();
                        if !has(&headers, "content-type") {
                            headers.push(("Content-Type".into(), "application/json".into()));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // default headers (like Postman's "auto-generated headers")
    if !has(&headers, "user-agent") && !settings.user_agent.is_empty() {
        headers.push(("User-Agent".into(), settings.user_agent.clone()));
    }
    if !has(&headers, "accept") {
        headers.push(("Accept".into(), "*/*".into()));
    }
    if settings.send_no_cache && !has(&headers, "cache-control") {
        headers.push(("Cache-Control".into(), "no-cache".into()));
    }
    if settings.send_postman_token && !has(&headers, "postman-token") {
        headers.push(("Postman-Token".into(), uuid::Uuid::new_v4().to_string()));
    }
    if !has(&headers, "accept-encoding") {
        headers.push(("Accept-Encoding".into(), "gzip, deflate, br".into()));
    }

    let mut req = HttpRequest { method, url, headers, body };
    let resolved_auth = auth_def.map(|a| resolve_auth(a, vars));
    if let Some(a) = &resolved_auth {
        auth::apply(a, &mut req, &form)?;
    }
    let opts = http_options(settings, behavior, jar);
    Ok(Prepared { req, opts, auth: resolved_auth, form })
}

/// Send a prepared request; handles the Digest challenge/response round-trip.
pub fn send(p: &Prepared) -> Result<HttpResponse, String> {
    let resp = http::execute(&p.req, &p.opts)?;
    if let Some(a) = &p.auth {
        if a.kind == "digest" && resp.status == 401 && !a.get_bool("disableRetryRequest") {
            if let Some(ch) = resp.headers.iter().find(|(k, v)| k.eq_ignore_ascii_case("www-authenticate") && v.to_lowercase().starts_with("digest")).map(|(_, v)| v.clone()) {
                let a2 = auth::digest_from_challenge(a, &ch);
                let mut r2 = p.req.clone();
                let h = auth::digest_header(&a2, &r2.method, &r2.url, &r2.body, None)?;
                auth::set_header(&mut r2, "Authorization", &h);
                let mut resp2 = http::execute(&r2, &p.opts)?;
                resp2.timings.total_ms += resp.timings.total_ms;
                return Ok(resp2);
            }
        }
    }
    Ok(resp)
}
