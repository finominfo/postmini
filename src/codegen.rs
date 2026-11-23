//! Code snippet generation (Postman "Code" panel).

use crate::model::{Request, Url};
use crate::settings::Settings;
use crate::vars::VarContext;

pub enum CodeBody {
    None,
    Raw(String),
    UrlEncoded(Vec<(String, String)>),
    Multipart(Vec<(String, String, bool)>),
    File(String),
}

pub struct CodeInput {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: CodeBody,
}

pub const LANGS: &[&str] = &[
    "cURL", "cURL (Windows cmd)", "PowerShell - Invoke-RestMethod", "HTTPie", "wget", "HTTP (raw)", "JavaScript - Fetch", "NodeJs - Axios", "Python - Requests", "Python - http.client", "Go - Native", "Java - OkHttp", "C# - HttpClient", "PHP - cURL", "Ruby - Net::HTTP", "Rust - reqwest", "Swift - URLSession", "Kotlin - OkHttp", "Dart - http",
];

/// Build codegen input: variables resolved, auth applied, no auto-generated headers.
pub fn input_for(r: &Request, auth: Option<&crate::model::Auth>, vars: &VarContext, settings: &Settings) -> Result<CodeInput, String> {
    let mut s = settings.clone();
    s.user_agent.clear();
    s.send_no_cache = false;
    s.send_postman_token = false;
    // keep body structure for multipart/form instead of raw bytes
    let mut r2 = r.clone();
    let structured = r.body.as_ref().filter(|b| !b.disabled).map(|b| b.mode.clone()).unwrap_or_default();
    if structured == "formdata" || structured == "file" {
        r2.body = None;
    }
    let p = crate::prepare::prepare(&r2, auth, vars, &serde_json::Map::new(), &s, None)?;
    let user_has = |k: &str| r.header.iter().any(|h| !h.disabled && h.key.eq_ignore_ascii_case(k));
    let headers: Vec<(String, String)> = p
        .req
        .headers
        .iter()
        .filter(|(k, v)| !((k.eq_ignore_ascii_case("accept") && v == "*/*" && !user_has("accept")) || (k.eq_ignore_ascii_case("accept-encoding") && !user_has("accept-encoding"))))
        .cloned()
        .collect();
    let body = match structured.as_str() {
        "formdata" => {
            let b = r.body.as_ref().unwrap();
            CodeBody::Multipart(b.formdata.iter().filter(|k| !k.disabled && !k.key.is_empty()).map(|k| if k.is_file() { (vars.resolve(&k.key), k.src_path(), true) } else { (vars.resolve(&k.key), vars.resolve(&k.value), false) }).collect())
        }
        "file" => CodeBody::File(r.body.as_ref().unwrap().file_src()),
        "urlencoded" if !p.req.body.is_empty() => CodeBody::UrlEncoded(p.form.clone()),
        _ if !p.req.body.is_empty() => CodeBody::Raw(String::from_utf8_lossy(&p.req.body).into_owned()),
        _ => CodeBody::None,
    };
    Ok(CodeInput { method: p.req.method, url: p.req.url, headers, body })
}

fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn dq(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}
fn sq_php(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}
fn kotlin_str(s: &str) -> String {
    dq(s).replace('$', "\\$")
}
fn raw_body_string(b: &CodeBody) -> String {
    match b {
        CodeBody::Raw(s) => s.clone(),
        CodeBody::UrlEncoded(f) => f.iter().map(|(k, v)| format!("{}={}", crate::auth::pct(k), crate::auth::pct(v))).collect::<Vec<_>>().join("&"),
        _ => String::new(),
    }
}
fn has_body(b: &CodeBody) -> bool {
    !matches!(b, CodeBody::None)
}
fn win_cmd(s: &str) -> String {
    // Windows cmd quoting compatible with "Copy as cURL (cmd)"
    let mut o = String::from("^\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\^\""),
            '%' | '&' | '|' | '<' | '>' | '^' | '(' | ')' | '!' => {
                o.push('^');
                o.push(c)
            }
            '\n' => o.push_str("^\n\n"),
            c => o.push(c),
        }
    }
    o.push_str("^\"");
    o
}

pub fn generate(lang: &str, i: &CodeInput) -> String {
    let m = &i.method;
    let url = &i.url;
    let mut o = String::new();
    match lang {
        "cURL" => {
            let implicit = m == "GET" && !has_body(&i.body) || m == "POST" && has_body(&i.body);
            o.push_str(&format!("curl --location{} {}", if implicit { String::new() } else { format!(" --request {m}") }, sh(url)));
            for (k, v) in &i.headers {
                o.push_str(&format!(" \\\n--header {}", sh(&format!("{k}: {v}"))));
            }
            match &i.body {
                CodeBody::Raw(s) => o.push_str(&format!(" \\\n--data {}", sh(s))),
                CodeBody::UrlEncoded(f) => {
                    for (k, v) in f {
                        o.push_str(&format!(" \\\n--data-urlencode {}", sh(&format!("{k}={v}"))));
                    }
                }
                CodeBody::Multipart(f) => {
                    for (k, v, file) in f {
                        o.push_str(&format!(" \\\n--form {}", sh(&if *file { format!("{k}=@\"{v}\"") } else { format!("{k}=\"{v}\"") })));
                    }
                }
                CodeBody::File(p) => o.push_str(&format!(" \\\n--data-binary {}", sh(&format!("@{p}")))),
                CodeBody::None => {}
            }
        }
        "cURL (Windows cmd)" => {
            o.push_str(&format!("curl --location --request {m} {}", win_cmd(url)));
            for (k, v) in &i.headers {
                o.push_str(&format!(" ^\n  --header {}", win_cmd(&format!("{k}: {v}"))));
            }
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => o.push_str(&format!(" ^\n  --data-raw {}", win_cmd(&raw_body_string(&i.body)))),
                CodeBody::Multipart(f) => {
                    for (k, v, file) in f {
                        o.push_str(&format!(" ^\n  --form {}", win_cmd(&if *file { format!("{k}=@{v}") } else { format!("{k}={v}") })));
                    }
                }
                CodeBody::File(p) => o.push_str(&format!(" ^\n  --data-binary {}", win_cmd(&format!("@{p}")))),
                CodeBody::None => {}
            }
        }
        "PowerShell - Invoke-RestMethod" => {
            let ps = |s: &str| format!("'{}'", s.replace('\'', "''"));
            o.push_str("$headers = New-Object \"System.Collections.Generic.Dictionary[[String],[String]]\"\n");
            for (k, v) in &i.headers {
                if k.eq_ignore_ascii_case("content-type") && matches!(i.body, CodeBody::Multipart(_)) {
                    continue;
                }
                o.push_str(&format!("$headers.Add({}, {})\n", ps(k), ps(v)));
            }
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => {
                    o.push_str(&format!("\n$body = @\"\n{}\n\"@\n", raw_body_string(&i.body)));
                    o.push_str(&format!("\n$response = Invoke-RestMethod {} -Method {} -Headers $headers -Body $body\n", ps(url), m));
                }
                CodeBody::Multipart(f) => {
                    o.push_str("\n$form = @{\n");
                    for (k, v, file) in f {
                        if *file {
                            o.push_str(&format!("    {} = Get-Item -Path {}\n", ps(k), ps(v)));
                        } else {
                            o.push_str(&format!("    {} = {}\n", ps(k), ps(v)));
                        }
                    }
                    o.push_str("}\n");
                    o.push_str(&format!("\n$response = Invoke-RestMethod {} -Method {} -Headers $headers -Form $form\n", ps(url), m));
                }
                CodeBody::File(p) => o.push_str(&format!("\n$response = Invoke-RestMethod {} -Method {} -Headers $headers -InFile {}\n", ps(url), m, ps(p))),
                CodeBody::None => o.push_str(&format!("\n$response = Invoke-RestMethod {} -Method {} -Headers $headers\n", ps(url), m)),
            }
            o.push_str("$response | ConvertTo-Json -Depth 10\n");
        }
        "HTTPie" => {
            match &i.body {
                CodeBody::Raw(s) => o.push_str(&format!("printf {} | ", sh(s))),
                CodeBody::File(p) => o.push_str(&format!("cat {} | ", sh(p))),
                _ => {}
            }
            o.push_str(&format!("http --follow --timeout 3600 {}", m));
            if matches!(i.body, CodeBody::UrlEncoded(_)) {
                o = o.replacen("http ", "http --form ", 1);
            }
            if matches!(i.body, CodeBody::Multipart(_)) {
                o = o.replacen("http ", "http --multipart ", 1);
            }
            o.push_str(&format!(" {}", sh(url)));
            for (k, v) in &i.headers {
                o.push_str(&format!(" \\\n {}", sh(&format!("{k}:{v}"))));
            }
            match &i.body {
                CodeBody::UrlEncoded(f) => {
                    for (k, v) in f {
                        o.push_str(&format!(" \\\n {}", sh(&format!("{k}={v}"))));
                    }
                }
                CodeBody::Multipart(f) => {
                    for (k, v, file) in f {
                        o.push_str(&format!(" \\\n {}", sh(&if *file { format!("{k}@{v}") } else { format!("{k}={v}") })));
                    }
                }
                _ => {}
            }
        }
        "wget" => {
            o.push_str(&format!("wget --no-check-certificate --quiet \\\n  --method {m} \\\n  --timeout=0"));
            for (k, v) in &i.headers {
                o.push_str(&format!(" \\\n  --header {}", sh(&format!("{k}: {v}"))));
            }
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => o.push_str(&format!(" \\\n  --body-data {}", sh(&raw_body_string(&i.body)))),
                CodeBody::File(p) => o.push_str(&format!(" \\\n  --body-file {}", sh(p))),
                _ => {}
            }
            o.push_str(&format!(" \\\n   {} \\\n  -O -", sh(url)));
        }
        "HTTP (raw)" => {
            let u = crate::http::parse_url(url, true);
            let (target, host) = u.as_ref().map(|u| (u.target.clone(), u.host_header())).unwrap_or((url.clone(), String::new()));
            o.push_str(&format!("{m} {target} HTTP/1.1\nHost: {host}\n"));
            for (k, v) in &i.headers {
                o.push_str(&format!("{k}: {v}\n"));
            }
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => {
                    let b = raw_body_string(&i.body);
                    o.push_str(&format!("Content-Length: {}\n\n{}", b.len(), b));
                }
                CodeBody::Multipart(f) => {
                    let bd = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
                    o.push_str(&format!("Content-Type: multipart/form-data; boundary={bd}\n\n"));
                    for (k, v, file) in f {
                        if *file {
                            o.push_str(&format!("--{bd}\nContent-Disposition: form-data; name=\"{k}\"; filename=\"{}\"\nContent-Type: <Content-Type header here>\n\n(data)\n", v.rsplit(['/', '\\']).next().unwrap_or(v)));
                        } else {
                            o.push_str(&format!("--{bd}\nContent-Disposition: form-data; name=\"{k}\"\n\n{v}\n"));
                        }
                    }
                    o.push_str(&format!("--{bd}--\n"));
                }
                CodeBody::File(p) => o.push_str(&format!("\n<file contents here: {p}>")),
                CodeBody::None => {}
            }
        }
        "JavaScript - Fetch" => {
            o.push_str("const myHeaders = new Headers();\n");
            for (k, v) in &i.headers {
                if k.eq_ignore_ascii_case("content-type") && matches!(i.body, CodeBody::Multipart(_)) {
                    continue;
                }
                o.push_str(&format!("myHeaders.append({}, {});\n", dq(k), dq(v)));
            }
            let body_var = match &i.body {
                CodeBody::Raw(s) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                        o.push_str(&format!("\nconst raw = JSON.stringify({});\n", serde_json::to_string_pretty(&v).unwrap_or_default()));
                    } else {
                        o.push_str(&format!("\nconst raw = {};\n", dq(s)));
                    }
                    Some("raw")
                }
                CodeBody::UrlEncoded(f) => {
                    o.push_str("\nconst urlencoded = new URLSearchParams();\n");
                    for (k, v) in f {
                        o.push_str(&format!("urlencoded.append({}, {});\n", dq(k), dq(v)));
                    }
                    Some("urlencoded")
                }
                CodeBody::Multipart(f) => {
                    o.push_str("\nconst formdata = new FormData();\n");
                    for (k, v, file) in f {
                        if *file {
                            o.push_str(&format!("formdata.append({}, fileInput.files[0], {});\n", dq(k), dq(v)));
                        } else {
                            o.push_str(&format!("formdata.append({}, {});\n", dq(k), dq(v)));
                        }
                    }
                    Some("formdata")
                }
                CodeBody::File(_) => {
                    o.push_str("\nconst file = \"<file contents here>\";\n");
                    Some("file")
                }
                CodeBody::None => None,
            };
            o.push_str(&format!("\nconst requestOptions = {{\n  method: {},\n  headers: myHeaders,\n", dq(m)));
            if let Some(b) = body_var {
                o.push_str(&format!("  body: {b},\n"));
            }
            o.push_str("  redirect: \"follow\"\n};\n\n");
            o.push_str(&format!("fetch({}, requestOptions)\n  .then((response) => response.text())\n  .then((result) => console.log(result))\n  .catch((error) => console.error(error));\n", dq(url)));
        }
        "NodeJs - Axios" => {
            o.push_str("const axios = require('axios');\n");
            let data = match &i.body {
                CodeBody::Raw(s) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                        o.push_str(&format!("let data = JSON.stringify({});\n", serde_json::to_string_pretty(&v).unwrap_or_default()));
                    } else {
                        o.push_str(&format!("let data = {};\n", dq(s)));
                    }
                    true
                }
                CodeBody::UrlEncoded(f) => {
                    o.push_str("const qs = require('qs');\nlet data = qs.stringify({\n");
                    for (k, v) in f {
                        o.push_str(&format!("  {}: {},\n", dq(k), dq(v)));
                    }
                    o.push_str("});\n");
                    true
                }
                CodeBody::Multipart(f) => {
                    o.push_str("const FormData = require('form-data');\nconst fs = require('fs');\nlet data = new FormData();\n");
                    for (k, v, file) in f {
                        if *file {
                            o.push_str(&format!("data.append({}, fs.createReadStream({}));\n", dq(k), dq(v)));
                        } else {
                            o.push_str(&format!("data.append({}, {});\n", dq(k), dq(v)));
                        }
                    }
                    true
                }
                CodeBody::File(p) => {
                    o.push_str(&format!("const fs = require('fs');\nlet data = fs.readFileSync({});\n", dq(p)));
                    true
                }
                CodeBody::None => false,
            };
            o.push_str(&format!("\nlet config = {{\n  method: {},\n  maxBodyLength: Infinity,\n  url: {},\n  headers: {{\n", dq(&m.to_lowercase()), dq(url)));
            for (k, v) in &i.headers {
                if k.eq_ignore_ascii_case("content-type") && matches!(i.body, CodeBody::Multipart(_)) {
                    continue;
                }
                o.push_str(&format!("    {}: {},\n", dq(k), dq(v)));
            }
            if matches!(i.body, CodeBody::Multipart(_)) {
                o.push_str("    ...data.getHeaders()\n");
            }
            o.push_str("  }");
            if data {
                o.push_str(",\n  data : data");
            }
            o.push_str("\n};\n\naxios.request(config)\n.then((response) => {\n  console.log(JSON.stringify(response.data));\n})\n.catch((error) => {\n  console.log(error);\n});\n");
        }
        "Python - Requests" => {
            o.push_str("import requests\n");
            let is_json = matches!(&i.body, CodeBody::Raw(s) if serde_json::from_str::<serde_json::Value>(s).is_ok());
            if is_json {
                o.push_str("import json\n");
            }
            o.push_str(&format!("\nurl = {}\n\n", dq(url)));
            match &i.body {
                CodeBody::Raw(s) => {
                    if is_json {
                        let v: serde_json::Value = serde_json::from_str(s).unwrap();
                        o.push_str(&format!("payload = json.dumps({})\n", py_literal(&v, 0)));
                    } else {
                        o.push_str(&format!("payload = {}\n", dq(s)));
                    }
                }
                CodeBody::UrlEncoded(_) => o.push_str(&format!("payload = {}\n", dq(&raw_body_string(&i.body)))),
                CodeBody::Multipart(f) => {
                    o.push_str("payload = {");
                    o.push_str(&f.iter().filter(|x| !x.2).map(|(k, v, _)| format!("{}: {}", dq(k), dq(v))).collect::<Vec<_>>().join(",\n"));
                    o.push_str("}\nfiles = [\n");
                    for (k, v, _) in f.iter().filter(|x| x.2) {
                        o.push_str(&format!("  ({}, ({}, open({}, 'rb'), 'application/octet-stream')),\n", dq(k), dq(v.rsplit(['/', '\\']).next().unwrap_or(v)), dq(v)));
                    }
                    o.push_str("]\n");
                }
                CodeBody::File(p) => o.push_str(&format!("payload = open({}, 'rb').read()\n", dq(p))),
                CodeBody::None => o.push_str("payload = {}\n"),
            }
            o.push_str("headers = {\n");
            for (k, v) in &i.headers {
                if k.eq_ignore_ascii_case("content-type") && matches!(i.body, CodeBody::Multipart(_)) {
                    continue;
                }
                o.push_str(&format!("  {}: {},\n", dq(k), dq(v)));
            }
            o.push_str("}\n\n");
            let files = if matches!(i.body, CodeBody::Multipart(_)) { ", files=files" } else { "" };
            o.push_str(&format!("response = requests.request({}, url, headers=headers, data=payload{})\n\nprint(response.text)\n", dq(m), files));
        }
        "Python - http.client" => {
            let u = crate::http::parse_url(url, true).ok();
            let (host, target, https, port) = u.map(|u| (u.host.clone(), u.target.clone(), u.scheme == "https", if u.port_explicit { Some(u.port) } else { None })).unwrap_or_default();
            o.push_str("import http.client\n\n");
            o.push_str(&format!("conn = http.client.{}({}{})\n", if https { "HTTPSConnection" } else { "HTTPConnection" }, dq(&host), port.map(|p| format!(", {p}")).unwrap_or_default()));
            o.push_str(&format!("payload = {}\n", dq(&raw_body_string(&i.body))));
            o.push_str("headers = {\n");
            for (k, v) in &i.headers {
                o.push_str(&format!("  {}: {},\n", dq(k), dq(v)));
            }
            o.push_str("}\n");
            o.push_str(&format!("conn.request({}, {}, payload, headers)\nres = conn.getresponse()\ndata = res.read()\nprint(data.decode(\"utf-8\"))\n", dq(m), dq(&target)));
        }
        "Go - Native" => {
            o.push_str("package main\n\nimport (\n  \"fmt\"\n");
            if has_body(&i.body) {
                o.push_str("  \"strings\"\n");
            }
            o.push_str("  \"net/http\"\n  \"io\"\n)\n\nfunc main() {\n\n");
            o.push_str(&format!("  url := {}\n  method := {}\n\n", dq(url), dq(m)));
            if has_body(&i.body) {
                o.push_str(&format!("  payload := strings.NewReader({})\n\n", go_str(&raw_body_string(&i.body))));
                o.push_str("  client := &http.Client {}\n  req, err := http.NewRequest(method, url, payload)\n\n");
            } else {
                o.push_str("  client := &http.Client {}\n  req, err := http.NewRequest(method, url, nil)\n\n");
            }
            o.push_str("  if err != nil {\n    fmt.Println(err)\n    return\n  }\n");
            for (k, v) in &i.headers {
                o.push_str(&format!("  req.Header.Add({}, {})\n", dq(k), dq(v)));
            }
            o.push_str("\n  res, err := client.Do(req)\n  if err != nil {\n    fmt.Println(err)\n    return\n  }\n  defer res.Body.Close()\n\n  body, err := io.ReadAll(res.Body)\n  if err != nil {\n    fmt.Println(err)\n    return\n  }\n  fmt.Println(string(body))\n}\n");
        }
        "Java - OkHttp" | "Kotlin - OkHttp" => {
            let kotlin = lang.starts_with("Kotlin");
            let q = |s: &str| if kotlin { kotlin_str(s) } else { dq(s) };
            let ct = i.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.clone()).unwrap_or_else(|| "text/plain".into());
            if kotlin {
                o.push_str("val client = OkHttpClient()\n");
            } else {
                o.push_str("OkHttpClient client = new OkHttpClient().newBuilder()\n  .build();\n");
            }
            let body_expr = if has_body(&i.body) {
                if kotlin {
                    o.push_str(&format!("val mediaType = {}.toMediaType()\nval body = {}.toRequestBody(mediaType)\n", q(&ct), q(&raw_body_string(&i.body))));
                } else {
                    o.push_str(&format!("MediaType mediaType = MediaType.parse({});\nRequestBody body = RequestBody.create(mediaType, {});\n", q(&ct), q(&raw_body_string(&i.body))));
                }
                "body"
            } else if matches!(m.as_str(), "POST" | "PUT" | "PATCH") {
                if kotlin {
                    o.push_str("val body = \"\".toRequestBody()\n");
                } else {
                    o.push_str("RequestBody body = RequestBody.create(\"\", null);\n");
                }
                "body"
            } else {
                "null"
            };
            if kotlin {
                o.push_str(&format!("val request = Request.Builder()\n  .url({})\n  .method({}, {})\n", q(url), q(m), body_expr));
            } else {
                o.push_str(&format!("Request request = new Request.Builder()\n  .url({})\n  .method({}, {})\n", q(url), q(m), body_expr));
            }
            for (k, v) in &i.headers {
                o.push_str(&format!("  .addHeader({}, {})\n", q(k), q(v)));
            }
            if kotlin {
                o.push_str("  .build()\nval response = client.newCall(request).execute()\n");
            } else {
                o.push_str("  .build();\nResponse response = client.newCall(request).execute();\n");
            }
        }
        "C# - HttpClient" => {
            o.push_str(&format!("var client = new HttpClient();\nvar request = new HttpRequestMessage(HttpMethod.{}, {});\n", cap(m), dq(url)));
            let ct = i.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.clone());
            for (k, v) in &i.headers {
                if k.eq_ignore_ascii_case("content-type") {
                    continue;
                }
                o.push_str(&format!("request.Headers.Add({}, {});\n", dq(k), dq(v)));
            }
            match &i.body {
                CodeBody::Raw(s) => o.push_str(&format!("var content = new StringContent({}, null, {});\nrequest.Content = content;\n", dq(s), dq(ct.as_deref().unwrap_or("text/plain")))),
                CodeBody::UrlEncoded(f) => {
                    o.push_str("var collection = new List<KeyValuePair<string, string>>();\n");
                    for (k, v) in f {
                        o.push_str(&format!("collection.Add(new(\"{}\", {}));\n", k.replace('"', "\\\""), dq(v)));
                    }
                    o.push_str("var content = new FormUrlEncodedContent(collection);\nrequest.Content = content;\n");
                }
                CodeBody::Multipart(f) => {
                    o.push_str("var content = new MultipartFormDataContent();\n");
                    for (k, v, file) in f {
                        if *file {
                            o.push_str(&format!("content.Add(new StreamContent(File.OpenRead({})), {}, {});\n", dq(v), dq(k), dq(v)));
                        } else {
                            o.push_str(&format!("content.Add(new StringContent({}), {});\n", dq(v), dq(k)));
                        }
                    }
                    o.push_str("request.Content = content;\n");
                }
                CodeBody::File(p) => o.push_str(&format!("request.Content = new StreamContent(File.OpenRead({}));\n", dq(p))),
                CodeBody::None => {}
            }
            o.push_str("var response = await client.SendAsync(request);\nresponse.EnsureSuccessStatusCode();\nConsole.WriteLine(await response.Content.ReadAsStringAsync());\n");
        }
        "PHP - cURL" => {
            o.push_str("<?php\n\n$curl = curl_init();\n\ncurl_setopt_array($curl, array(\n");
            o.push_str(&format!("  CURLOPT_URL => {},\n  CURLOPT_RETURNTRANSFER => true,\n  CURLOPT_ENCODING => '',\n  CURLOPT_MAXREDIRS => 10,\n  CURLOPT_TIMEOUT => 0,\n  CURLOPT_FOLLOWLOCATION => true,\n  CURLOPT_HTTP_VERSION => CURL_HTTP_VERSION_1_1,\n  CURLOPT_CUSTOMREQUEST => {},\n", sq_php(url), sq_php(m)));
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => o.push_str(&format!("  CURLOPT_POSTFIELDS => {},\n", sq_php(&raw_body_string(&i.body)))),
                CodeBody::Multipart(f) => {
                    o.push_str("  CURLOPT_POSTFIELDS => array(");
                    o.push_str(&f.iter().map(|(k, v, file)| if *file { format!("{} => new CURLFILE({})", sq_php(k), sq_php(v)) } else { format!("{} => {}", sq_php(k), sq_php(v)) }).collect::<Vec<_>>().join(","));
                    o.push_str("),\n");
                }
                CodeBody::File(p) => o.push_str(&format!("  CURLOPT_POSTFIELDS => file_get_contents({}),\n", sq_php(p))),
                CodeBody::None => {}
            }
            if !i.headers.is_empty() {
                o.push_str("  CURLOPT_HTTPHEADER => array(\n");
                o.push_str(&i.headers.iter().filter(|(k, _)| !(k.eq_ignore_ascii_case("content-type") && matches!(i.body, CodeBody::Multipart(_)))).map(|(k, v)| format!("    {}", sq_php(&format!("{k}: {v}")))).collect::<Vec<_>>().join(",\n"));
                o.push_str("\n  ),\n");
            }
            o.push_str("));\n\n$response = curl_exec($curl);\n\ncurl_close($curl);\necho $response;\n");
        }
        "Ruby - Net::HTTP" => {
            let rb = |s: &str| dq(s).replace("#{", "\\#{");
            o.push_str("require \"uri\"\nrequire \"net/http\"\n\n");
            o.push_str(&format!("url = URI({})\n\n", rb(url)));
            o.push_str(&format!("http = Net::HTTP.new(url.host, url.port);\n{}", if url.starts_with("https") { "http.use_ssl = true\n" } else { "" }));
            o.push_str(&format!("request = Net::HTTP::{}.new(url)\n", cap(m)));
            for (k, v) in &i.headers {
                o.push_str(&format!("request[{}] = {}\n", rb(k), rb(v)));
            }
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => o.push_str(&format!("request.body = {}\n", rb(&raw_body_string(&i.body)))),
                CodeBody::Multipart(f) => {
                    o.push_str("form_data = [");
                    o.push_str(&f.iter().map(|(k, v, file)| if *file { format!("[{}, File.open({})]", rb(k), rb(v)) } else { format!("[{}, {}]", rb(k), rb(v)) }).collect::<Vec<_>>().join(","));
                    o.push_str("]\nrequest.set_form form_data, 'multipart/form-data'\n");
                }
                CodeBody::File(p) => o.push_str(&format!("request.body = File.read({})\n", rb(p))),
                CodeBody::None => {}
            }
            o.push_str("\nresponse = http.request(request)\nputs response.read_body\n");
        }
        "Rust - reqwest" => {
            o.push_str("#[tokio::main]\nasync fn main() -> Result<(), Box<dyn std::error::Error>> {\n    let client = reqwest::Client::builder()\n        .build()?;\n\n");
            o.push_str("    let mut headers = reqwest::header::HeaderMap::new();\n");
            for (k, v) in &i.headers {
                if k.eq_ignore_ascii_case("content-type") && matches!(i.body, CodeBody::Multipart(_)) {
                    continue;
                }
                o.push_str(&format!("    headers.insert({}, {}.parse()?);\n", dq(k), dq(v)));
            }
            let mut call = format!("    let request = client.request(reqwest::Method::from_bytes(b{})?, {})\n        .headers(headers)", dq(m), dq(url));
            match &i.body {
                CodeBody::Raw(_) | CodeBody::UrlEncoded(_) => {
                    o.push_str(&format!("\n    let data = {};\n\n", rust_raw(&raw_body_string(&i.body))));
                    call.push_str("\n        .body(data)");
                }
                CodeBody::Multipart(f) => {
                    o.push_str("\n    let form = reqwest::multipart::Form::new()");
                    for (k, v, file) in f {
                        if *file {
                            o.push_str(&format!("\n        .part({}, reqwest::multipart::Part::bytes(std::fs::read({})?).file_name({}))", dq(k), dq(v), dq(v.rsplit(['/', '\\']).next().unwrap_or(v))));
                        } else {
                            o.push_str(&format!("\n        .text({}, {})", dq(k), dq(v)));
                        }
                    }
                    o.push_str(";\n\n");
                    call.push_str("\n        .multipart(form)");
                }
                CodeBody::File(p) => {
                    o.push_str(&format!("\n    let data = std::fs::read({})?;\n\n", dq(p)));
                    call.push_str("\n        .body(data)");
                }
                CodeBody::None => o.push('\n'),
            }
            o.push_str(&call);
            o.push_str(";\n\n    let response = request.send().await?;\n    let body = response.text().await?;\n\n    println!(\"{}\", body);\n\n    Ok(())\n}\n");
        }
        "Swift - URLSession" => {
            if has_body(&i.body) {
                o.push_str(&format!("let parameters = {}\nlet postData = parameters.data(using: .utf8)\n\n", dq(&raw_body_string(&i.body))));
            }
            o.push_str(&format!("var request = URLRequest(url: URL(string: {})!,timeoutInterval: Double.infinity)\n", dq(url)));
            for (k, v) in &i.headers {
                o.push_str(&format!("request.addValue({}, forHTTPHeaderField: {})\n", dq(v), dq(k)));
            }
            o.push_str(&format!("\nrequest.httpMethod = {}\n", dq(m)));
            if has_body(&i.body) {
                o.push_str("request.httpBody = postData\n");
            }
            o.push_str("\nlet task = URLSession.shared.dataTask(with: request) { data, response, error in \n  guard let data = data else {\n    print(String(describing: error))\n    return\n  }\n  print(String(data: data, encoding: .utf8)!)\n}\n\ntask.resume()\n");
        }
        "Dart - http" => {
            let ds = |s: &str| dq(s).replace('$', "\\$");
            o.push_str("var headers = {\n");
            for (k, v) in &i.headers {
                o.push_str(&format!("  {}: {},\n", ds(k), ds(v)));
            }
            o.push_str("};\n");
            o.push_str(&format!("var request = http.Request({}, Uri.parse({}));\n", ds(m), ds(url)));
            if has_body(&i.body) {
                o.push_str(&format!("request.body = {};\n", ds(&raw_body_string(&i.body))));
            }
            o.push_str("request.headers.addAll(headers);\n\nhttp.StreamedResponse response = await request.send();\n\nif (response.statusCode == 200) {\n  print(await response.stream.bytesToString());\n}\nelse {\n  print(response.reasonPhrase);\n}\n");
        }
        _ => o.push_str("Unsupported language"),
    }
    o
}

fn cap(m: &str) -> String {
    let l = m.to_lowercase();
    let mut c = l.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn go_str(s: &str) -> String {
    if !s.contains('`') {
        format!("`{s}`")
    } else {
        dq(s)
    }
}

fn rust_raw(s: &str) -> String {
    let mut hashes = String::from("#");
    while s.contains(&format!("\"{hashes}")) {
        hashes.push('#');
    }
    format!("r{hashes}\"{s}\"{hashes}")
}

fn py_literal(v: &serde_json::Value, ind: usize) -> String {
    use serde_json::Value;
    let pad = "  ".repeat(ind + 1);
    let end = "  ".repeat(ind);
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True".into() } else { "False".into() },
        Value::Number(n) => n.to_string(),
        Value::String(s) => dq(s),
        Value::Array(a) => {
            if a.is_empty() {
                return "[]".into();
            }
            format!("[\n{}\n{}]", a.iter().map(|x| format!("{pad}{}", py_literal(x, ind + 1))).collect::<Vec<_>>().join(",\n"), end)
        }
        Value::Object(m) => {
            if m.is_empty() {
                return "{}".into();
            }
            format!("{{\n{}\n{}}}", m.iter().map(|(k, x)| format!("{pad}{}: {}", dq(k), py_literal(x, ind + 1))).collect::<Vec<_>>().join(",\n"), end)
        }
    }
}

#[allow(dead_code)]
pub fn url_only(raw: &str) -> Url {
    Url::from_raw(raw)
}
