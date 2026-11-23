//! Importers: cURL commands, Postman v1 collections, OpenAPI 3 / Swagger 2.

use serde_json::{json, Map, Value};

use crate::model::{new_id, value_to_string, Auth, Body, Collection, Environment, GraphQl, Item, Request, Url, Variable, KV};

// ------------------------------------------------------------------ cURL

fn tokenize(cmd: &str) -> Vec<String> {
    // Windows cmd "Copy as cURL (cmd)" uses ^ as escape and ^" for quotes
    let windows = cmd.contains("^\"") || cmd.contains("^\n") || cmd.contains("^\r\n");
    let mut s = cmd.replace("\r\n", "\n");
    if windows {
        s = s.replace("^\n", " ").replace("^^", "\u{1}").replace("^\"", "\u{2}").replace('^', "").replace('\u{1}', "^");
    }
    s = s.replace("\\\n", " ");
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\u{2}' => {
                // windows quoted section: until next \u{2}
                has = true;
                i += 1;
                while i < chars.len() && chars[i] != '\u{2}' {
                    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\u{2}' {
                        cur.push('"');
                        i += 2;
                        continue;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '\'' => {
                has = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '\'' => {
                has = true;
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        match chars[i] {
                            'n' => cur.push('\n'),
                            't' => cur.push('\t'),
                            'r' => cur.push('\r'),
                            '\\' => cur.push('\\'),
                            '\'' => cur.push('\''),
                            '"' => cur.push('"'),
                            'x' if i + 2 < chars.len() => {
                                let h: String = chars[i + 1..i + 3].iter().collect();
                                if let Ok(b) = u8::from_str_radix(&h, 16) {
                                    cur.push(b as char);
                                }
                                i += 2;
                            }
                            'u' if i + 4 < chars.len() => {
                                let h: String = chars[i + 1..i + 5].iter().collect();
                                if let Some(ch) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                                    cur.push(ch);
                                }
                                i += 4;
                            }
                            o => {
                                cur.push('\\');
                                cur.push(o)
                            }
                        }
                    } else {
                        cur.push(chars[i]);
                    }
                    i += 1;
                }
            }
            '"' => {
                has = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '"' | '\\' | '$' | '`') {
                        i += 1;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '\\' if i + 1 < chars.len() => {
                has = true;
                i += 1;
                cur.push(chars[i]);
            }
            c if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                has = true;
                cur.push(c)
            }
        }
        i += 1;
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn is_curl(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("curl ") || t.starts_with("curl.exe ") || t.starts_with("curl\n")
}

pub fn parse_curl(cmd: &str) -> Result<Request, String> {
    let toks = tokenize(cmd.trim());
    if toks.is_empty() || !(toks[0] == "curl" || toks[0] == "curl.exe") {
        return Err("Not a curl command".into());
    }
    let mut method: Option<String> = None;
    let mut headers: Vec<KV> = vec![];
    let mut data: Vec<String> = vec![];
    let mut data_urlencode = false;
    let mut form: Vec<KV> = vec![];
    let mut url = String::new();
    let mut get = false;
    let mut auth: Option<Auth> = None;
    let mut upload: Option<String> = None;
    let mut i = 1;
    let takes_arg = ["-o", "--output", "-m", "--max-time", "--connect-timeout", "-x", "--proxy", "--cacert", "--cert", "--key", "-w", "--write-out", "-c", "--cookie-jar", "-r", "--range", "--retry", "-E", "--limit-rate", "-Y", "-y", "--proxy-user", "-U", "--resolve", "--interface", "--max-redirs"];
    while i < toks.len() {
        let t = toks[i].clone();
        let mut next = || {
            i += 1;
            toks.get(i).cloned().unwrap_or_default()
        };
        // support --flag=value
        let (flag, inline) = if t.starts_with("--") && t.contains('=') {
            let (a, b) = t.split_once('=').unwrap();
            (a.to_string(), Some(b.to_string()))
        } else {
            (t.clone(), None)
        };
        let mut arg = |inline: &Option<String>| inline.clone().unwrap_or_else(&mut next);
        match flag.as_str() {
            "-X" | "--request" => method = Some(arg(&inline).to_uppercase()),
            "-H" | "--header" => {
                let h = arg(&inline);
                if let Some((k, v)) = h.split_once(':') {
                    headers.push(KV::new(k.trim(), v.trim()));
                } else if let Some(k) = h.strip_suffix(';') {
                    headers.push(KV::new(k.trim(), ""));
                }
            }
            "-d" | "--data" | "--data-ascii" | "--data-raw" | "--data-binary" | "--json" => {
                let d = arg(&inline);
                if flag == "--json" {
                    headers.push(KV::new("Content-Type", "application/json"));
                    headers.push(KV::new("Accept", "application/json"));
                }
                if let Some(f) = d.strip_prefix('@').filter(|_| flag != "--data-raw") {
                    data.push(std::fs::read_to_string(f).unwrap_or_default());
                } else {
                    data.push(d);
                }
            }
            "--data-urlencode" => {
                data_urlencode = true;
                let d = arg(&inline);
                let enc = match d.split_once('=') {
                    Some((k, v)) => format!("{}={}", k, crate::auth::pct(v)),
                    None => crate::auth::pct(&d),
                };
                data.push(enc);
            }
            "-F" | "--form" | "--form-string" => {
                let f = arg(&inline);
                if let Some((k, v)) = f.split_once('=') {
                    if let Some(path) = v.strip_prefix('@').filter(|_| flag != "--form-string") {
                        let (p, ct) = match path.split_once(";type=") {
                            Some((p, ct)) => (p.to_string(), Some(ct.to_string())),
                            None => (path.split(';').next().unwrap_or("").to_string(), None),
                        };
                        form.push(KV { key: k.into(), kind: Some("file".into()), src: Some(Value::String(p)), content_type: ct, ..Default::default() });
                    } else {
                        let v = v.strip_prefix('<').map(|p| std::fs::read_to_string(p).unwrap_or_default()).unwrap_or_else(|| v.to_string());
                        form.push(KV { key: k.into(), value: v, kind: Some("text".into()), ..Default::default() });
                    }
                }
            }
            "-u" | "--user" => {
                let u = arg(&inline);
                let (user, pass) = u.split_once(':').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((u, String::new()));
                let mut a = Auth::new("basic");
                a.set_str("username", &user);
                a.set_str("password", &pass);
                auth = Some(a);
            }
            "--digest" => {
                if let Some(a) = &mut auth {
                    let mut d = Auth::new("digest");
                    d.set_str("username", &a.get("username"));
                    d.set_str("password", &a.get("password"));
                    *a = d;
                }
            }
            "--oauth2-bearer" => {
                let mut a = Auth::new("bearer");
                a.set_str("token", &arg(&inline));
                auth = Some(a);
            }
            "-b" | "--cookie" => {
                let c = arg(&inline);
                headers.push(KV::new("Cookie", &c));
            }
            "-A" | "--user-agent" => {
                let a = arg(&inline);
                headers.push(KV::new("User-Agent", &a));
            }
            "-e" | "--referer" => {
                let a = arg(&inline);
                headers.push(KV::new("Referer", &a));
            }
            "-G" | "--get" => get = true,
            "-I" | "--head" => method = Some("HEAD".into()),
            "-T" | "--upload-file" => upload = Some(arg(&inline)),
            "--url" => url = arg(&inline),
            "-k" | "--insecure" | "-L" | "--location" | "--compressed" | "-s" | "--silent" | "-v" | "--verbose" | "-i" | "--include" | "-S" | "--show-error" | "-f" | "--fail" | "--http1.1" | "--http2" | "-g" | "--globoff" | "-N" | "--no-buffer" => {}
            f if takes_arg.contains(&f) => {
                if inline.is_none() {
                    next();
                }
            }
            f if f.starts_with('-') && f.len() > 2 && !f.starts_with("--") => {
                // combined short flags like -sSL ; if ends with X/H/d take next
                let last = f.chars().last().unwrap();
                if matches!(last, 'X' | 'H' | 'd' | 'u' | 'o' | 'F' | 'b' | 'A' | 'e' | 'm' | 'x') {
                    let v = next();
                    match last {
                        'X' => method = Some(v.to_uppercase()),
                        'H' => {
                            if let Some((k, vv)) = v.split_once(':') {
                                headers.push(KV::new(k.trim(), vv.trim()));
                            }
                        }
                        'd' => data.push(v),
                        _ => {}
                    }
                }
            }
            f if f.starts_with('-') => {}
            _ => {
                if url.is_empty() {
                    url = t.clone();
                }
            }
        }
        i += 1;
    }
    if url.is_empty() {
        return Err("No URL found in curl command".into());
    }
    let ct = headers.iter().find(|h| h.key.eq_ignore_ascii_case("content-type")).map(|h| h.value.to_lowercase()).unwrap_or_default();
    let mut body: Option<Body> = None;
    if get && !data.is_empty() {
        let q = data.join("&");
        url = if url.contains('?') { format!("{url}&{q}") } else { format!("{url}?{q}") };
        data.clear();
    }
    if !form.is_empty() {
        body = Some(Body { mode: "formdata".into(), formdata: form, ..Default::default() });
        headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type") || !h.value.to_lowercase().starts_with("multipart/form-data"));
    } else if let Some(f) = upload {
        body = Some(Body { mode: "file".into(), file: Some(json!({ "src": f })), ..Default::default() });
        if method.is_none() {
            method = Some("PUT".into());
        }
    } else if !data.is_empty() {
        let raw = data.join("&");
        let looks_form = !raw.trim_start().starts_with('{') && !raw.trim_start().starts_with('[') && raw.split('&').all(|p| p.contains('=') && !p.contains(' ') && !p.contains('\n'));
        if ct.contains("x-www-form-urlencoded") || (ct.is_empty() && looks_form) || data_urlencode {
            let kvs = raw
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| match p.split_once('=') {
                    Some((k, v)) => KV::new(&crate::auth::pct_decode(k), &crate::auth::pct_decode(v)),
                    None => KV::new(&crate::auth::pct_decode(p), ""),
                })
                .collect();
            body = Some(Body { mode: "urlencoded".into(), urlencoded: kvs, ..Default::default() });
        } else {
            let mut b = Body { mode: "raw".into(), raw: Some(raw.clone()), ..Default::default() };
            let lang = if ct.contains("json") || serde_json::from_str::<Value>(&raw).is_ok() {
                "json"
            } else if ct.contains("xml") {
                "xml"
            } else if ct.contains("html") {
                "html"
            } else if ct.contains("javascript") {
                "javascript"
            } else {
                "text"
            };
            b.set_raw_language(lang);
            // GraphQL detection
            if lang == "json" {
                if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                    if let Some(q) = v.get("query").and_then(|q| q.as_str()) {
                        if v.as_object().map(|o| o.keys().all(|k| k == "query" || k == "variables" || k == "operationName")).unwrap_or(false) {
                            b = Body { mode: "graphql".into(), graphql: Some(GraphQl { query: q.into(), variables: v.get("variables").map(|x| serde_json::to_string_pretty(x).unwrap_or_default()).unwrap_or_default() }), ..Default::default() };
                        }
                    }
                }
            }
            body = Some(b);
        }
    }
    let method = method.unwrap_or_else(|| if body.is_some() { "POST".into() } else { "GET".into() });
    Ok(Request { method, header: headers, body, url: Url::from_raw(&url), auth, description: None, extra: Map::new() })
}

// ------------------------------------------------------------------ Postman v1

pub fn convert_v1(v: &Value) -> Result<Collection, String> {
    let mut c = Collection::new(&v.get("name").map(value_to_string).unwrap_or_else(|| "Imported".into()));
    let reqs: Vec<&Value> = v.get("requests").and_then(|r| r.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    let conv = |r: &Value| -> Item {
        let mut req = Request { method: r.get("method").map(value_to_string).unwrap_or_else(|| "GET".into()).to_uppercase(), url: Url::from_raw(&r.get("url").map(value_to_string).unwrap_or_default()), ..Default::default() };
        let hs = r.get("headers").map(value_to_string).unwrap_or_default();
        req.header = hs.lines().filter_map(|l| l.split_once(':').map(|(k, v)| KV::new(k.trim(), v.trim()))).collect();
        match r.get("dataMode").and_then(|d| d.as_str()) {
            Some("raw") => req.body = Some(Body { mode: "raw".into(), raw: r.get("rawModeData").map(value_to_string), ..Default::default() }),
            Some("urlencoded") | Some("params") => {
                let list: Vec<KV> = r
                    .get("data")
                    .and_then(|d| d.as_array())
                    .map(|a| a.iter().map(|x| KV { key: x.get("key").map(value_to_string).unwrap_or_default(), value: x.get("value").map(value_to_string).unwrap_or_default(), disabled: x.get("enabled").and_then(|e| e.as_bool()) == Some(false), kind: x.get("type").map(value_to_string), ..Default::default() }).collect())
                    .unwrap_or_default();
                if r.get("dataMode").and_then(|d| d.as_str()) == Some("params") {
                    req.body = Some(Body { mode: "formdata".into(), formdata: list, ..Default::default() });
                } else {
                    req.body = Some(Body { mode: "urlencoded".into(), urlencoded: list, ..Default::default() });
                }
            }
            _ => {}
        }
        let mut it = Item::new_request(&r.get("name").map(value_to_string).unwrap_or_else(|| "Request".into()), req);
        let pre = r.get("preRequestScript").map(value_to_string).unwrap_or_default();
        if !pre.is_empty() {
            crate::model::set_script(&mut it.event, "prerequest", &pre);
        }
        let tests = r.get("tests").map(value_to_string).unwrap_or_default();
        if !tests.is_empty() {
            crate::model::set_script(&mut it.event, "test", &tests);
        }
        it
    };
    let find = |id: &str| reqs.iter().find(|r| r.get("id").map(value_to_string).as_deref() == Some(id)).copied();
    let mut used = std::collections::HashSet::new();
    if let Some(folders) = v.get("folders").and_then(|f| f.as_array()) {
        for f in folders {
            let mut folder = Item::new_folder(&f.get("name").map(value_to_string).unwrap_or_default());
            let mut ch = vec![];
            for id in f.get("order").and_then(|o| o.as_array()).cloned().unwrap_or_default() {
                let id = value_to_string(&id);
                if let Some(r) = find(&id) {
                    used.insert(id);
                    ch.push(conv(r));
                }
            }
            folder.item = Some(ch);
            c.item.push(folder);
        }
    }
    let order: Vec<String> = v.get("order").and_then(|o| o.as_array()).map(|a| a.iter().map(value_to_string).collect()).unwrap_or_else(|| reqs.iter().filter_map(|r| r.get("id").map(value_to_string)).collect());
    for id in order {
        if used.contains(&id) {
            continue;
        }
        if let Some(r) = find(&id) {
            c.item.push(conv(r));
        }
    }
    Ok(c)
}

// ------------------------------------------------------------------ OpenAPI / Swagger

fn resolve_ref<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
    let mut cur = v;
    for _ in 0..10 {
        match cur.get("$ref").and_then(|r| r.as_str()) {
            Some(r) if r.starts_with("#/") => {
                let mut t = root;
                for p in r[2..].split('/') {
                    let p = p.replace("~1", "/").replace("~0", "~");
                    t = match t.get(&p) {
                        Some(x) => x,
                        None => return cur,
                    };
                }
                cur = t;
            }
            _ => break,
        }
    }
    cur
}

fn sample_from_schema(root: &Value, s: &Value, depth: usize) -> Value {
    let s = resolve_ref(root, s);
    if depth > 8 {
        return Value::Null;
    }
    if let Some(e) = s.get("example") {
        return e.clone();
    }
    if let Some(e) = s.get("default") {
        return e.clone();
    }
    if let Some(e) = s.get("enum").and_then(|e| e.as_array()).and_then(|a| a.first()) {
        return e.clone();
    }
    for k in ["allOf"] {
        if let Some(list) = s.get(k).and_then(|a| a.as_array()) {
            let mut m = Map::new();
            for sub in list {
                if let Value::Object(o) = sample_from_schema(root, sub, depth + 1) {
                    m.extend(o);
                }
            }
            return Value::Object(m);
        }
    }
    for k in ["oneOf", "anyOf"] {
        if let Some(first) = s.get(k).and_then(|a| a.as_array()).and_then(|a| a.first()) {
            return sample_from_schema(root, first, depth + 1);
        }
    }
    let ty = s.get("type").and_then(|t| t.as_str()).unwrap_or(if s.get("properties").is_some() { "object" } else { "string" });
    match ty {
        "object" => {
            let mut m = Map::new();
            if let Some(props) = s.get("properties").and_then(|p| p.as_object()) {
                for (k, v) in props {
                    m.insert(k.clone(), sample_from_schema(root, v, depth + 1));
                }
            }
            Value::Object(m)
        }
        "array" => Value::Array(vec![s.get("items").map(|i| sample_from_schema(root, i, depth + 1)).unwrap_or(Value::Null)]),
        "integer" => json!(0),
        "number" => json!(0.0),
        "boolean" => json!(true),
        _ => {
            let f = s.get("format").and_then(|f| f.as_str()).unwrap_or("");
            Value::String(match f {
                "date-time" => "2024-01-01T00:00:00Z".into(),
                "date" => "2024-01-01".into(),
                "uuid" => "00000000-0000-0000-0000-000000000000".into(),
                "email" => "user@example.com".into(),
                "uri" | "url" => "https://example.com".into(),
                _ => "<string>".into(),
            })
        }
    }
}

pub fn is_openapi(v: &Value) -> bool {
    v.get("openapi").is_some() || v.get("swagger").is_some()
}

pub fn parse_any_doc(text: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(text).or_else(|_| serde_yaml::from_str::<Value>(text).map_err(|e| format!("Not valid JSON or YAML: {e}")))
}

pub fn import_openapi(root: &Value) -> Result<Collection, String> {
    let title = root.get("info").and_then(|i| i.get("title")).map(value_to_string).unwrap_or_else(|| "OpenAPI import".into());
    let mut c = Collection::new(&title);
    if let Some(d) = root.get("info").and_then(|i| i.get("description")).and_then(|d| d.as_str()) {
        c.info.description = Some(crate::model::Description(d.into()));
    }
    let swagger2 = root.get("swagger").is_some();
    let base = if swagger2 {
        let scheme = root.get("schemes").and_then(|s| s.as_array()).and_then(|a| a.first()).map(value_to_string).unwrap_or_else(|| "https".into());
        format!("{}://{}{}", scheme, root.get("host").map(value_to_string).unwrap_or_else(|| "localhost".into()), root.get("basePath").map(value_to_string).unwrap_or_default())
    } else {
        let s = root.get("servers").and_then(|s| s.as_array()).and_then(|a| a.first());
        let mut u = s.and_then(|s| s.get("url")).map(value_to_string).unwrap_or_else(|| "/".into());
        if let Some(vars) = s.and_then(|s| s.get("variables")).and_then(|v| v.as_object()) {
            for (k, v) in vars {
                u = u.replace(&format!("{{{k}}}"), &v.get("default").map(value_to_string).unwrap_or_default());
            }
        }
        u
    };
    c.variable.push(Variable { key: "baseUrl".into(), value: Value::String(base.trim_end_matches('/').to_string()), ..Default::default() });
    let paths = root.get("paths").and_then(|p| p.as_object()).ok_or("OpenAPI document has no paths")?;
    let mut folders: Vec<Item> = vec![];
    for (path, ops) in paths {
        let ops_obj = match ops.as_object() {
            Some(o) => o,
            None => continue,
        };
        let common: Vec<Value> = ops.get("parameters").and_then(|p| p.as_array()).cloned().unwrap_or_default();
        for (m, op) in ops_obj {
            let method = m.to_uppercase();
            if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" | "TRACE") {
                continue;
            }
            let pm_path = {
                let mut out = String::new();
                for seg in path.split('/') {
                    if !out.is_empty() || path.starts_with('/') {
                        out.push('/');
                    }
                    if seg.starts_with('{') && seg.ends_with('}') {
                        out.push(':');
                        out.push_str(&seg[1..seg.len() - 1]);
                    } else {
                        out.push_str(seg);
                    }
                }
                out.replacen("//", "/", 1)
            };
            let mut req = Request { method: method.clone(), ..Default::default() };
            let mut query = vec![];
            let mut pathvars = vec![];
            let mut params: Vec<Value> = common.clone();
            params.extend(op.get("parameters").and_then(|p| p.as_array()).cloned().unwrap_or_default());
            let mut body_param: Option<Value> = None;
            let mut form_params: Vec<KV> = vec![];
            for p in &params {
                let p = resolve_ref(root, p);
                let name = p.get("name").map(value_to_string).unwrap_or_default();
                let ex = p.get("example").cloned().or_else(|| p.get("schema").map(|s| sample_from_schema(root, s, 0))).or_else(|| p.get("default").cloned()).map(|v| value_to_string(&v)).unwrap_or_default();
                let ex = if ex == "<string>" { String::new() } else { ex };
                let desc = p.get("description").map(value_to_string).filter(|d| !d.is_empty()).map(crate::model::Description);
                match p.get("in").and_then(|i| i.as_str()).unwrap_or("") {
                    "query" => query.push(KV { key: name, value: ex, description: desc, disabled: !p.get("required").and_then(|r| r.as_bool()).unwrap_or(false), ..Default::default() }),
                    "header" => req.header.push(KV { key: name, value: ex, description: desc, ..Default::default() }),
                    "path" => pathvars.push(KV { key: name, value: ex, description: desc, ..Default::default() }),
                    "body" => body_param = p.get("schema").cloned(),
                    "formData" => form_params.push(KV { key: name, value: ex, kind: Some(if p.get("type").and_then(|t| t.as_str()) == Some("file") { "file".into() } else { "text".into() }), ..Default::default() }),
                    _ => {}
                }
            }
            let enabled_q: Vec<String> = query.iter().filter(|q| !q.disabled).map(|q| format!("{}={}", q.key, q.value)).collect();
            let mut raw = format!("{{{{baseUrl}}}}{}", pm_path);
            if !enabled_q.is_empty() {
                raw.push('?');
                raw.push_str(&enabled_q.join("&"));
            }
            req.url = Url::from_raw(&raw);
            req.url.query = query;
            req.url.variable = pathvars;
            // body
            if let Some(schema) = body_param {
                let sample = sample_from_schema(root, &schema, 0);
                let mut b = Body { mode: "raw".into(), raw: Some(serde_json::to_string_pretty(&sample).unwrap_or_default()), ..Default::default() };
                b.set_raw_language("json");
                req.body = Some(b);
                req.header.push(KV::new("Content-Type", "application/json"));
            } else if !form_params.is_empty() {
                req.body = Some(Body { mode: "formdata".into(), formdata: form_params, ..Default::default() });
            } else if let Some(rb) = op.get("requestBody") {
                let rb = resolve_ref(root, rb);
                if let Some(content) = rb.get("content").and_then(|c| c.as_object()) {
                    if let Some((ct, media)) = content.iter().find(|(k, _)| k.contains("json")).or_else(|| content.iter().next()) {
                        let sample = media.get("example").cloned().or_else(|| media.get("examples").and_then(|e| e.as_object()).and_then(|e| e.values().next()).map(|e| resolve_ref(root, e).get("value").cloned().unwrap_or(Value::Null))).unwrap_or_else(|| media.get("schema").map(|s| sample_from_schema(root, s, 0)).unwrap_or(Value::Null));
                        if ct.contains("x-www-form-urlencoded") || ct.contains("multipart") {
                            let kvs: Vec<KV> = sample.as_object().map(|o| o.iter().map(|(k, v)| KV { key: k.clone(), value: value_to_string(v), kind: if ct.contains("multipart") { Some("text".into()) } else { None }, ..Default::default() }).collect()).unwrap_or_default();
                            req.body = Some(if ct.contains("multipart") { Body { mode: "formdata".into(), formdata: kvs, ..Default::default() } } else { Body { mode: "urlencoded".into(), urlencoded: kvs, ..Default::default() } });
                        } else {
                            let text = if ct.contains("json") { serde_json::to_string_pretty(&sample).unwrap_or_default() } else { value_to_string(&sample) };
                            let mut b = Body { mode: "raw".into(), raw: Some(text), ..Default::default() };
                            b.set_raw_language(if ct.contains("json") { "json" } else if ct.contains("xml") { "xml" } else { "text" });
                            req.body = Some(b);
                            req.header.push(KV::new("Content-Type", ct));
                        }
                    }
                }
            }
            let name = op.get("summary").map(value_to_string).filter(|s| !s.is_empty()).or_else(|| op.get("operationId").map(value_to_string)).unwrap_or_else(|| format!("{method} {path}"));
            if let Some(d) = op.get("description").and_then(|d| d.as_str()) {
                req.description = Some(crate::model::Description(d.into()));
            }
            let mut item = Item::new_request(&name, req);
            // saved example responses
            if let Some(resps) = op.get("responses").and_then(|r| r.as_object()) {
                for (code, r) in resps {
                    let r = resolve_ref(root, r);
                    let codeint: u16 = code.parse().unwrap_or(200);
                    let body = r.get("content").and_then(|c| c.as_object()).and_then(|c| c.iter().find(|(k, _)| k.contains("json")).or_else(|| c.iter().next())).map(|(_, m)| m.get("example").cloned().unwrap_or_else(|| m.get("schema").map(|s| sample_from_schema(root, s, 0)).unwrap_or(Value::Null)));
                    let body = body.or_else(|| r.get("schema").map(|s| sample_from_schema(root, s, 0)));
                    if let Some(b) = body {
                        item.response.push(json!({
                            "name": r.get("description").map(value_to_string).unwrap_or_else(|| code.clone()),
                            "originalRequest": item.request,
                            "status": crate::http::status_text(codeint),
                            "code": codeint,
                            "_postman_previewlanguage": "json",
                            "header": [{"key": "Content-Type", "value": "application/json"}],
                            "body": serde_json::to_string_pretty(&b).unwrap_or_default()
                        }));
                    }
                }
            }
            let tag = op.get("tags").and_then(|t| t.as_array()).and_then(|a| a.first()).map(value_to_string);
            match tag {
                Some(t) => {
                    if let Some(f) = folders.iter_mut().find(|f| f.name == t) {
                        f.item.as_mut().unwrap().push(item);
                    } else {
                        let mut f = Item::new_folder(&t);
                        f.item = Some(vec![item]);
                        folders.push(f);
                    }
                }
                None => c.item.push(item),
            }
        }
    }
    let mut all = folders;
    all.append(&mut c.item);
    c.item = all;
    // security
    let sec_schemes = root.get("components").and_then(|c| c.get("securitySchemes")).or_else(|| root.get("securityDefinitions")).and_then(|s| s.as_object());
    if let Some(ss) = sec_schemes {
        if let Some((_, s)) = ss.iter().next() {
            let ty = s.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let scheme = s.get("scheme").and_then(|t| t.as_str()).unwrap_or("");
            let mut a = None;
            if ty == "http" && scheme.eq_ignore_ascii_case("bearer") || ty == "oauth2" || ty == "openIdConnect" {
                let mut x = Auth::new("bearer");
                x.set_str("token", "{{bearerToken}}");
                a = Some(x);
            } else if ty == "http" && scheme.eq_ignore_ascii_case("basic") || ty == "basic" {
                let mut x = Auth::new("basic");
                x.set_str("username", "{{username}}");
                x.set_str("password", "{{password}}");
                a = Some(x);
            } else if ty == "apiKey" {
                let mut x = Auth::new("apikey");
                x.set_str("key", &s.get("name").map(value_to_string).unwrap_or_default());
                x.set_str("value", "{{apiKey}}");
                x.set_str("in", if s.get("in").and_then(|i| i.as_str()) == Some("query") { "query" } else { "header" });
                a = Some(x);
            }
            c.auth = a;
        }
    }
    c.ensure_ids();
    Ok(c)
}

/// What an import text turned out to be
pub enum Imported {
    Collection(Collection),
    Environment(Environment),
    Request(Request),
}

pub fn import_text(text: &str) -> Result<Imported, String> {
    let t = text.trim();
    if is_curl(t) {
        return parse_curl(t).map(Imported::Request);
    }
    let v = parse_any_doc(t)?;
    if is_openapi(&v) {
        return import_openapi(&v).map(Imported::Collection);
    }
    if v.get("info").is_some() || v.get("requests").is_some() || v.get("collection").is_some() {
        return crate::model::parse_collection(t).or_else(|_| crate::model::parse_collection(&v.to_string())).map(Imported::Collection);
    }
    if v.get("values").is_some() || v.get("environment").is_some() {
        return Environment::parse(&v.to_string()).map(Imported::Environment);
    }
    // a bare URL?
    if !t.contains('\n') && (t.starts_with("http://") || t.starts_with("https://")) {
        return Ok(Imported::Request(Request { url: Url::from_raw(t), ..Default::default() }));
    }
    let _ = new_id;
    Err("Unrecognized format. Supported: Postman collection v1/v2.0/v2.1, Postman environment, OpenAPI 3.x / Swagger 2.0 (JSON/YAML), cURL".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curl_basic() {
        let r = parse_curl(r#"curl -X POST 'https://api.x.com/v1/items?a=1' -H 'Content-Type: application/json' -H "Authorization: Bearer abc" --data-raw '{"name":"x"}' --compressed"#).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url.raw, "https://api.x.com/v1/items?a=1");
        assert_eq!(r.header.len(), 2);
        assert_eq!(r.body.as_ref().unwrap().raw.as_deref(), Some("{\"name\":\"x\"}"));
        assert_eq!(r.body.as_ref().unwrap().raw_language(), "json");
    }
    #[test]
    fn curl_windows() {
        let r = parse_curl("curl ^\"https://x.com/a^\" ^\n  -H ^\"accept: */*^\" ^\n  --data-raw ^\"^{^\\^\"a^\\^\":1^}^\"").unwrap();
        assert_eq!(r.url.raw, "https://x.com/a");
        assert_eq!(r.body.unwrap().raw.unwrap(), "{\"a\":1}");
    }
    #[test]
    fn curl_form() {
        let r = parse_curl("curl -u user:pw -F 'file=@/tmp/a.txt' -F name=joe http://h/up").unwrap();
        assert_eq!(r.auth.unwrap().get("username"), "user");
        assert_eq!(r.body.unwrap().formdata.len(), 2);
    }
}

#[cfg(test)]
mod oa_tests {
    #[test]
    fn openapi() {
        let t = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/petstore.yaml")).unwrap();
        let super::Imported::Collection(c) = super::import_text(&t).unwrap() else { panic!() };
        assert_eq!(c.info.name, "Petstore");
        let reqs = c.requests();
        assert_eq!(reqs.len(), 3);
        let post = reqs.iter().find(|(_, i)| i.name == "Create pet").unwrap().1;
        let body = post.request.as_ref().unwrap().body.as_ref().unwrap().raw.clone().unwrap();
        assert!(body.contains("\"name\": \"Rex\""), "{body}");
        let byid = reqs.iter().find(|(_, i)| i.name == "showPetById").unwrap().1;
        assert_eq!(byid.request.as_ref().unwrap().url.raw, "{{baseUrl}}/pets/:petId");
        assert_eq!(byid.request.as_ref().unwrap().url.variable[0].value, "42");
        assert_eq!(c.auth.as_ref().unwrap().kind, "apikey");
        let _ = serde_json::to_string_pretty(&c).unwrap();
    }
}
