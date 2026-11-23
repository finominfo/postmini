//! Authorization helpers, modelled on postman-runtime's authorizers.

use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use base64::Engine;
use hmac::{Hmac, Mac};
use md5::Md5;
use rand::Rng;
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::http::HttpRequest;
use crate::model::Auth;

const RFC3986: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

pub fn pct(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, RFC3986).to_string()
}

pub fn pct_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned()
}

fn hmac_bytes(alg: &str, key: &[u8], data: &[u8]) -> Vec<u8> {
    match alg {
        "sha1" => {
            let mut m = Hmac::<Sha1>::new_from_slice(key).unwrap();
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        "sha384" => {
            let mut m = Hmac::<Sha384>::new_from_slice(key).unwrap();
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        "sha512" => {
            let mut m = Hmac::<Sha512>::new_from_slice(key).unwrap();
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        "md5" => {
            let mut m = Hmac::<Md5>::new_from_slice(key).unwrap();
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        _ => {
            let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
    }
}

fn sha256_hex(d: &[u8]) -> String {
    hex::encode(Sha256::digest(d))
}

fn nonce(n: usize) -> String {
    let r = &mut rand::thread_rng();
    (0..n).map(|_| r.sample(rand::distributions::Alphanumeric) as char).collect()
}

pub fn set_header(req: &mut HttpRequest, k: &str, v: &str) {
    req.headers.retain(|(h, _)| !h.eq_ignore_ascii_case(k));
    req.headers.push((k.to_string(), v.to_string()));
}

pub fn add_query(url: &str, k: &str, v: &str) -> String {
    let (base, hash) = match url.find('#') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, ""),
    };
    let sep = if base.contains('?') { if base.ends_with('?') || base.ends_with('&') { "" } else { "&" } } else { "?" };
    format!("{base}{sep}{}={}{hash}", pct(k), pct(v))
}

fn has_header(req: &HttpRequest, k: &str) -> bool {
    req.headers.iter().any(|(h, _)| h.eq_ignore_ascii_case(k))
}

/// Split URL into (base without query, query pairs decoded)
fn split_url(url: &str) -> (String, Vec<(String, String)>) {
    let no_hash = url.split('#').next().unwrap_or("");
    match no_hash.find('?') {
        Some(i) => {
            let q = &no_hash[i + 1..];
            let pairs = q
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| match p.find('=') {
                    Some(j) => (pct_decode(&p[..j]), pct_decode(&p[j + 1..])),
                    None => (pct_decode(p), String::new()),
                })
                .collect();
            (no_hash[..i].to_string(), pairs)
        }
        None => (no_hash.to_string(), vec![]),
    }
}

/// Apply auth that does not need a challenge. `form` = urlencoded body params (for OAuth1).
pub fn apply(auth: &Auth, req: &mut HttpRequest, form: &[(String, String)]) -> Result<(), String> {
    match auth.kind.as_str() {
        "noauth" | "inherit" | "" => Ok(()),
        "basic" => {
            let v = format!("{}:{}", auth.get("username"), auth.get("password"));
            set_header(req, "Authorization", &format!("Basic {}", B64.encode(v)));
            Ok(())
        }
        "bearer" => {
            let t = auth.get("token");
            if !t.is_empty() {
                set_header(req, "Authorization", &format!("Bearer {t}"));
            }
            Ok(())
        }
        "apikey" => {
            let k = auth.get("key");
            let v = auth.get("value");
            if k.is_empty() {
                return Ok(());
            }
            if auth.get_or("in", "header") == "query" {
                req.url = add_query(&req.url, &k, &v);
            } else {
                set_header(req, &k, &v);
            }
            Ok(())
        }
        "oauth2" => {
            let token = auth.get("accessToken");
            if token.is_empty() {
                return Ok(());
            }
            if auth.get_or("addTokenTo", "header") == "queryParams" {
                req.url = add_query(&req.url, "access_token", &token);
            } else {
                let prefix = auth.get_or("headerPrefix", "Bearer");
                let v = if prefix.is_empty() { token } else { format!("{prefix} {token}") };
                set_header(req, "Authorization", &v);
            }
            Ok(())
        }
        "jwt" => apply_jwt(auth, req),
        "digest" => {
            // when realm & nonce are already known compute directly, otherwise wait for challenge
            if !auth.get("realm").is_empty() && !auth.get("nonce").is_empty() {
                let h = digest_header(auth, &req.method, &req.url, &req.body, None)?;
                set_header(req, "Authorization", &h);
            }
            Ok(())
        }
        "oauth1" => apply_oauth1(auth, req, form),
        "hawk" => apply_hawk(auth, req),
        "awsv4" => apply_aws(auth, req),
        "edgegrid" => apply_edgegrid(auth, req),
        "ntlm" => Err("NTLM authentication is not supported by postmini (requires connection-bound handshake)".into()),
        "asap" => Err("ASAP authentication (RS256) is not supported by postmini".into()),
        other => Err(format!("Unknown auth type: {other}")),
    }
}

// ------------------------------------------------------------------ JWT

fn apply_jwt(auth: &Auth, req: &mut HttpRequest) -> Result<(), String> {
    let alg = auth.get_or("algorithm", "HS256").to_uppercase();
    let h = match alg.as_str() {
        "HS256" => "sha256",
        "HS384" => "sha384",
        "HS512" => "sha512",
        other => return Err(format!("JWT algorithm {other} not supported (use HS256/HS384/HS512)")),
    };
    let mut header: serde_json::Map<String, Value> = serde_json::from_str(&auth.get_or("header", "{}")).unwrap_or_default();
    header.insert("alg".into(), Value::String(alg.clone()));
    header.entry("typ").or_insert(Value::String("JWT".into()));
    let payload_s = auth.get_or("payload", "{}");
    let payload: Value = serde_json::from_str(if payload_s.trim().is_empty() { "{}" } else { &payload_s }).map_err(|e| format!("JWT payload is not valid JSON: {e}"))?;
    let secret_s = auth.get("secret");
    let secret = if auth.get_bool("isSecretBase64Encoded") { B64.decode(secret_s.trim()).map_err(|e| e.to_string())? } else { secret_s.into_bytes() };
    let signing_input = format!("{}.{}", B64URL.encode(Value::Object(header).to_string()), B64URL.encode(payload.to_string()));
    let sig = B64URL.encode(hmac_bytes(h, &secret, signing_input.as_bytes()));
    let token = format!("{signing_input}.{sig}");
    if auth.get_or("addTokenTo", "header") == "queryParam" {
        let k = auth.get_or("queryParamKey", "token");
        req.url = add_query(&req.url, &k, &token);
    } else {
        let prefix = auth.get_or("headerPrefix", "Bearer");
        set_header(req, "Authorization", &format!("{} {}", prefix, token).trim().to_string());
    }
    Ok(())
}

// ------------------------------------------------------------------ Digest

pub fn parse_challenge(h: &str) -> Vec<(String, String)> {
    let s = h.trim();
    let s = s.split_once(' ').map(|(_, r)| r).unwrap_or(s);
    let mut out = vec![];
    let mut rest = s;
    while !rest.is_empty() {
        rest = rest.trim_start_matches([',', ' ']);
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_lowercase();
        rest = &rest[eq + 1..];
        let val;
        if let Some(r) = rest.strip_prefix('"') {
            let end = r.find('"').unwrap_or(r.len());
            val = r[..end].to_string();
            rest = if end < r.len() { &r[end + 1..] } else { "" };
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            val = rest[..end].trim().to_string();
            rest = &rest[end..];
        }
        out.push((key, val));
    }
    out
}

/// Update digest params from a WWW-Authenticate challenge
pub fn digest_from_challenge(auth: &Auth, challenge: &str) -> Auth {
    let mut a = auth.clone();
    for (k, v) in parse_challenge(challenge) {
        match k.as_str() {
            "realm" => a.set_str("realm", &v),
            "nonce" => a.set_str("nonce", &v),
            "opaque" => a.set_str("opaque", &v),
            "algorithm" => a.set_str("algorithm", &v),
            "qop" => {
                let q = if v.split(',').any(|x| x.trim() == "auth") { "auth" } else { v.split(',').next().unwrap_or("").trim() };
                a.set_str("qop", q)
            }
            _ => {}
        }
    }
    a
}

pub fn digest_header(auth: &Auth, method: &str, url: &str, body: &[u8], _: Option<()>) -> Result<String, String> {
    let user = auth.get("username");
    let pass = auth.get("password");
    let realm = auth.get("realm");
    let nonce_v = auth.get("nonce");
    let alg = auth.get_or("algorithm", "MD5");
    let qop = auth.get("qop");
    let nc = { let n = auth.get_or("nonceCount", "00000001"); if n.is_empty() { "00000001".to_string() } else { n } };
    let cnonce = { let c = auth.get("clientNonce"); if c.is_empty() { nonce(16).to_lowercase() } else { c } };
    let opaque = auth.get("opaque");
    let u = crate::http::parse_url(url, true)?;
    let uri = u.target.clone();
    let hash = |s: &str| -> String {
        if alg.to_uppercase().starts_with("SHA-256") {
            sha256_hex(s.as_bytes())
        } else if alg.to_uppercase().starts_with("SHA-512-256") {
            let full = Sha512::digest(s.as_bytes());
            hex::encode(&full[..32])
        } else {
            hex::encode(Md5::digest(s.as_bytes()))
        }
    };
    let mut ha1 = hash(&format!("{user}:{realm}:{pass}"));
    if alg.to_lowercase().ends_with("-sess") {
        ha1 = hash(&format!("{ha1}:{nonce_v}:{cnonce}"));
    }
    let ha2 = if qop == "auth-int" {
        let bh = hash(&String::from_utf8_lossy(body));
        hash(&format!("{method}:{uri}:{bh}"))
    } else {
        hash(&format!("{method}:{uri}"))
    };
    let response = if qop.is_empty() { hash(&format!("{ha1}:{nonce_v}:{ha2}")) } else { hash(&format!("{ha1}:{nonce_v}:{nc}:{cnonce}:{qop}:{ha2}")) };
    let mut h = format!("Digest username=\"{user}\", realm=\"{realm}\", nonce=\"{nonce_v}\", uri=\"{uri}\", algorithm=\"{alg}\"");
    if !qop.is_empty() {
        h.push_str(&format!(", qop={qop}, nc={nc}, cnonce=\"{cnonce}\""));
    }
    h.push_str(&format!(", response=\"{response}\""));
    if !opaque.is_empty() {
        h.push_str(&format!(", opaque=\"{opaque}\""));
    }
    Ok(h)
}

// ------------------------------------------------------------------ OAuth 1.0

fn apply_oauth1(auth: &Auth, req: &mut HttpRequest, form: &[(String, String)]) -> Result<(), String> {
    let method = auth.get_or("signatureMethod", "HMAC-SHA1").to_uppercase();
    let ts = { let t = auth.get("timestamp"); if t.is_empty() { chrono::Utc::now().timestamp().to_string() } else { t } };
    let nn = { let n = auth.get("nonce"); if n.is_empty() { nonce(11) } else { n } };
    let version = auth.get_or("version", "1.0");
    let add_empty = auth.get_bool("addEmptyParamsToSign");
    let mut oauth: Vec<(String, String)> = vec![
        ("oauth_consumer_key".into(), auth.get("consumerKey")),
        ("oauth_token".into(), auth.get("token")),
        ("oauth_signature_method".into(), method.clone()),
        ("oauth_timestamp".into(), ts),
        ("oauth_nonce".into(), nn),
        ("oauth_version".into(), version),
        ("oauth_callback".into(), auth.get("callback")),
        ("oauth_verifier".into(), auth.get("verifier")),
    ];
    if auth.get_bool("includeBodyHash") && !req.body.is_empty() && form.is_empty() {
        let bh = match method.as_str() {
            "HMAC-SHA256" => B64.encode(Sha256::digest(&req.body)),
            "HMAC-SHA512" => B64.encode(Sha512::digest(&req.body)),
            _ => B64.encode(Sha1::digest(&req.body)),
        };
        oauth.push(("oauth_body_hash".into(), bh));
    }
    oauth.retain(|(k, v)| !v.is_empty() || (add_empty && k != "oauth_callback" && k != "oauth_verifier"));
    let (base, query) = split_url(&req.url);
    let u = crate::http::parse_url(&base, true)?;
    let base_url = format!("{}://{}{}", u.scheme, u.host_header(), u.target.split('?').next().unwrap_or("/"));
    let mut params: Vec<(String, String)> = oauth.clone();
    params.extend(query.iter().cloned());
    params.extend(form.iter().cloned());
    let mut enc: Vec<(String, String)> = params.iter().map(|(k, v)| (pct(k), pct(v))).collect();
    enc.sort();
    let param_str = enc.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let base_string = format!("{}&{}&{}", req.method.to_uppercase(), pct(&base_url), pct(&param_str));
    let key = format!("{}&{}", pct(&auth.get("consumerSecret")), pct(&auth.get("tokenSecret")));
    let sig = match method.as_str() {
        "HMAC-SHA1" => B64.encode(hmac_bytes("sha1", key.as_bytes(), base_string.as_bytes())),
        "HMAC-SHA256" => B64.encode(hmac_bytes("sha256", key.as_bytes(), base_string.as_bytes())),
        "HMAC-SHA512" => B64.encode(hmac_bytes("sha512", key.as_bytes(), base_string.as_bytes())),
        "PLAINTEXT" => key.clone(),
        other => return Err(format!("OAuth1 signature method {other} not supported")),
    };
    oauth.push(("oauth_signature".into(), sig));
    let add_to = auth.get_or("addParamsToHeader", "true");
    let to_header = add_to == "true" || add_to.is_empty() || auth.get_bool("addParamsToHeader");
    if to_header {
        let realm = auth.get("realm");
        let mut parts = vec![];
        if !realm.is_empty() {
            parts.push(format!("realm=\"{}\"", realm));
        }
        for (k, v) in &oauth {
            parts.push(format!("{}=\"{}\"", pct(k), pct(v)));
        }
        set_header(req, "Authorization", &format!("OAuth {}", parts.join(",")));
    } else if form.is_empty() || req.method == "GET" {
        for (k, v) in &oauth {
            req.url = add_query(&req.url, k, v);
        }
    } else {
        let mut b = String::from_utf8_lossy(&req.body).into_owned();
        for (k, v) in &oauth {
            if !b.is_empty() {
                b.push('&');
            }
            b.push_str(&format!("{}={}", pct(k), pct(v)));
        }
        req.body = b.into_bytes();
        req.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
    }
    Ok(())
}

// ------------------------------------------------------------------ Hawk

fn apply_hawk(auth: &Auth, req: &mut HttpRequest) -> Result<(), String> {
    let id = auth.get("authId");
    let key = auth.get("authKey");
    let alg = auth.get_or("algorithm", "sha256").to_lowercase();
    let alg = if alg.contains("1") && !alg.contains("256") { "sha1" } else { "sha256" };
    let ts = { let t = auth.get("timestamp"); if t.is_empty() { chrono::Utc::now().timestamp().to_string() } else { t } };
    let nn = { let n = auth.get("nonce"); if n.is_empty() { nonce(6) } else { n } };
    let ext = auth.get("extraData");
    let app = auth.get("app");
    let dlg = auth.get("delegation");
    let u = crate::http::parse_url(&req.url, true)?;
    let mut hash = String::new();
    if auth.get_bool("includePayloadHash") {
        let ct = req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.split(';').next().unwrap_or("").trim().to_lowercase()).unwrap_or_default();
        let data = format!("hawk.1.payload\n{}\n{}\n", ct, String::from_utf8_lossy(&req.body));
        hash = if alg == "sha1" { B64.encode(Sha1::digest(data.as_bytes())) } else { B64.encode(Sha256::digest(data.as_bytes())) };
    }
    let mut norm = format!("hawk.1.header\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n", ts, nn, req.method.to_uppercase(), u.target, u.host, u.port, hash, ext.replace('\\', "\\\\").replace('\n', "\\n"));
    if !app.is_empty() {
        norm.push_str(&format!("{}\n{}\n", app, dlg));
    }
    let mac = B64.encode(hmac_bytes(alg, key.as_bytes(), norm.as_bytes()));
    let mut h = format!("Hawk id=\"{id}\", ts=\"{ts}\", nonce=\"{nn}\"");
    if !hash.is_empty() {
        h.push_str(&format!(", hash=\"{hash}\""));
    }
    if !ext.is_empty() {
        h.push_str(&format!(", ext=\"{}\"", ext.replace('\\', "\\\\").replace('"', "\\\"")));
    }
    h.push_str(&format!(", mac=\"{mac}\""));
    if !app.is_empty() {
        h.push_str(&format!(", app=\"{app}\""));
        if !dlg.is_empty() {
            h.push_str(&format!(", dlg=\"{dlg}\""));
        }
    }
    set_header(req, "Authorization", &h);
    Ok(())
}

// ------------------------------------------------------------------ AWS SigV4

fn aws_uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn apply_aws(auth: &Auth, req: &mut HttpRequest) -> Result<(), String> {
    let access = auth.get("accessKey");
    let secret = auth.get("secretKey");
    let region = auth.get_or("region", "us-east-1");
    let region = if region.is_empty() { "us-east-1".to_string() } else { region };
    let service = { let s = auth.get("service"); if s.is_empty() { "execute-api".to_string() } else { s } };
    let token = auth.get("sessionToken");
    let to_query = auth.get_bool("addAuthDataToQuery");
    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let payload_hash = if to_query && service == "s3" { "UNSIGNED-PAYLOAD".to_string() } else { sha256_hex(&req.body) };

    if !to_query {
        set_header(req, "X-Amz-Date", &amz_date);
        if !token.is_empty() {
            set_header(req, "X-Amz-Security-Token", &token);
        }
        if service == "s3" || has_header(req, "x-amz-content-sha256") {
            set_header(req, "X-Amz-Content-Sha256", &payload_hash);
        }
    }
    let (base, mut query) = split_url(&req.url);
    let u = crate::http::parse_url(&base, false)?;
    // signed headers
    let mut hdrs: Vec<(String, String)> = vec![("host".into(), u.host_header())];
    if !to_query {
        for (k, v) in &req.headers {
            let lk = k.to_lowercase();
            if lk == "host" || lk == "authorization" || lk == "content-length" || lk == "user-agent" || lk == "connection" || lk == "expect" || lk.starts_with("postman") || lk == "cache-control" || lk == "accept-encoding" {
                continue;
            }
            hdrs.push((lk, v.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
    }
    hdrs.sort_by(|a, b| a.0.cmp(&b.0));
    hdrs.dedup_by(|a, b| a.0 == b.0);
    let signed_headers = hdrs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    if to_query {
        query.push(("X-Amz-Algorithm".into(), "AWS4-HMAC-SHA256".into()));
        query.push(("X-Amz-Credential".into(), format!("{access}/{scope}")));
        query.push(("X-Amz-Date".into(), amz_date.clone()));
        query.push(("X-Amz-Expires".into(), "86400".into()));
        query.push(("X-Amz-SignedHeaders".into(), signed_headers.clone()));
        if !token.is_empty() {
            query.push(("X-Amz-Security-Token".into(), token.clone()));
        }
    }
    let path = u.target.split('?').next().unwrap_or("/").to_string();
    let path_dec = percent_encoding::percent_decode_str(&path).decode_utf8_lossy().into_owned();
    let canon_uri = if service == "s3" { aws_uri_encode(&path_dec, false) } else { aws_uri_encode(&aws_uri_encode(&path_dec, false), false) };
    let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (aws_uri_encode(k, true), aws_uri_encode(v, true))).collect();
    q.sort();
    let canon_query = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let canon_headers: String = hdrs.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let canonical = format!("{}\n{}\n{}\n{}\n{}\n{}", req.method.to_uppercase(), canon_uri, canon_query, canon_headers, signed_headers, payload_hash);
    let sts = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));
    let k_date = hmac_bytes("sha256", format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_bytes("sha256", &k_date, region.as_bytes());
    let k_service = hmac_bytes("sha256", &k_region, service.as_bytes());
    let k_sign = hmac_bytes("sha256", &k_service, b"aws4_request");
    let signature = hex::encode(hmac_bytes("sha256", &k_sign, sts.as_bytes()));
    if to_query {
        query.push(("X-Amz-Signature".into(), signature));
        let qs = query.iter().map(|(k, v)| format!("{}={}", aws_uri_encode(k, true), aws_uri_encode(v, true))).collect::<Vec<_>>().join("&");
        req.url = format!("{base}?{qs}");
    } else {
        set_header(req, "Authorization", &format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"));
    }
    Ok(())
}

// ------------------------------------------------------------------ Akamai EdgeGrid

fn apply_edgegrid(auth: &Auth, req: &mut HttpRequest) -> Result<(), String> {
    let access = auth.get("accessToken");
    let client = auth.get("clientToken");
    let secret = auth.get("clientSecret");
    let ts = { let t = auth.get("timestamp"); if t.is_empty() { chrono::Utc::now().format("%Y%m%dT%H:%M:%S+0000").to_string() } else { t } };
    let nn = { let n = auth.get("nonce"); if n.is_empty() { uuid::Uuid::new_v4().to_string() } else { n } };
    let to_sign: Vec<String> = auth.get("headersToSign").split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    let mut url = req.url.clone();
    let base_uri = auth.get("baseURL");
    if !base_uri.is_empty() {
        if let Ok(pu) = crate::http::parse_url(&url, true) {
            url = format!("{}{}", base_uri.trim_end_matches('/'), pu.target);
        }
    }
    let u = crate::http::parse_url(&url, true)?;
    let canon_headers = req
        .headers
        .iter()
        .filter(|(k, _)| to_sign.contains(&k.to_lowercase()))
        .map(|(k, v)| format!("{}:{}", k.to_lowercase(), v.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect::<Vec<_>>()
        .join("\t");
    let content_hash = if req.method == "POST" && !req.body.is_empty() {
        let n = req.body.len().min(131072);
        B64.encode(Sha256::digest(&req.body[..n]))
    } else {
        String::new()
    };
    let auth_header = format!("EG1-HMAC-SHA256 client_token={client};access_token={access};timestamp={ts};nonce={nn};");
    let data = format!("{}\t{}\t{}\t{}\t{}\t{}\t{}", req.method.to_uppercase(), u.scheme, u.host, u.target, canon_headers, content_hash, auth_header);
    let signing_key = B64.encode(hmac_bytes("sha256", secret.as_bytes(), ts.as_bytes()));
    let sig = B64.encode(hmac_bytes("sha256", signing_key.as_bytes(), data.as_bytes()));
    set_header(req, "Authorization", &format!("{auth_header}signature={sig}"));
    if !base_uri.is_empty() {
        req.url = url;
    }
    Ok(())
}

// ------------------------------------------------------------------ OAuth 2.0 token retrieval

#[derive(Debug, Clone)]
pub struct TokenResult {
    pub access_token: String,
    pub token_type: String,
    pub refresh_token: String,
    pub expires_in: Option<i64>,
    pub raw: Value,
}

fn pkce_challenge(verifier: &str) -> String {
    B64URL.encode(Sha256::digest(verifier.as_bytes()))
}

fn token_request(auth: &Auth, params: Vec<(String, String)>, opts: &crate::http::HttpOptions) -> Result<TokenResult, String> {
    let url = auth.get("accessTokenUrl");
    if url.is_empty() {
        return Err("Access Token URL is required".into());
    }
    let mut params = params;
    let mut headers = vec![("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string()), ("Accept".to_string(), "application/json".to_string())];
    let cid = auth.get("clientId");
    let csec = auth.get("clientSecret");
    if auth.get_or("client_authentication", "header") == "body" {
        params.push(("client_id".into(), cid));
        if !csec.is_empty() {
            params.push(("client_secret".into(), csec));
        }
    } else if !cid.is_empty() {
        headers.push(("Authorization".into(), format!("Basic {}", B64.encode(format!("{}:{}", pct(&cid), pct(&csec))))));
    }
    let scope = auth.get("scope");
    if !scope.is_empty() && !params.iter().any(|(k, _)| k == "scope") {
        params.push(("scope".into(), scope));
    }
    let aud = auth.get("audience");
    if !aud.is_empty() {
        params.push(("audience".into(), aud));
    }
    let res = auth.get("resource");
    if !res.is_empty() {
        params.push(("resource".into(), res));
    }
    let body = params.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&");
    let req = HttpRequest { method: "POST".into(), url, headers, body: body.into_bytes() };
    let resp = crate::http::execute(&req, opts)?;
    let text = resp.text();
    let raw: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        // some servers answer urlencoded
        let mut m = serde_json::Map::new();
        for p in text.split('&') {
            if let Some((k, v)) = p.split_once('=') {
                m.insert(pct_decode(k), Value::String(pct_decode(v)));
            }
        }
        Value::Object(m)
    });
    if resp.status >= 400 || raw.get("error").is_some() {
        return Err(format!("Token request failed ({}): {}", resp.status, text.chars().take(500).collect::<String>()));
    }
    let access = raw.get("access_token").map(crate::model::value_to_string).unwrap_or_default();
    if access.is_empty() {
        return Err(format!("No access_token in response: {}", text.chars().take(500).collect::<String>()));
    }
    Ok(TokenResult {
        access_token: access,
        token_type: raw.get("token_type").map(crate::model::value_to_string).unwrap_or_else(|| "Bearer".into()),
        refresh_token: raw.get("refresh_token").map(crate::model::value_to_string).unwrap_or_default(),
        expires_in: raw.get("expires_in").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))),
        raw,
    })
}

pub fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Wait on a loopback redirect URI for the authorization response; returns query params.
fn wait_for_redirect(redirect_uri: &str, timeout_s: u64) -> Result<Vec<(String, String)>, String> {
    use std::io::{BufRead, BufReader, Write};
    let u = crate::http::parse_url(redirect_uri, true)?;
    if !(u.host == "localhost" || u.host == "127.0.0.1" || u.host == "::1") {
        return Err(format!("Callback URL must be a loopback address (e.g. http://localhost:5555/callback) so postmini can receive the code. Got {redirect_uri}"));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", u.port)).map_err(|e| format!("Cannot listen on port {}: {e}", u.port))?;
    listener.set_nonblocking(true).ok();
    let start = std::time::Instant::now();
    loop {
        if start.elapsed().as_secs() > timeout_s {
            return Err("Timed out waiting for authorization (5 min)".into());
        }
        match listener.accept() {
            Ok((mut s, _)) => {
                s.set_nonblocking(false).ok();
                let mut rd = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                let mut line = String::new();
                rd.read_line(&mut line).ok();
                let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (_, q) = split_url(&format!("http://x{target}"));
                // implicit flow returns token in fragment: serve page that re-posts fragment as query
                if q.is_empty() {
                    let page = "<html><body><script>if(location.hash){location.replace(location.pathname+'?'+location.hash.substring(1));}else{document.body.innerText='No parameters received.';}</script></body></html>";
                    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
                    continue;
                }
                let page = "<html><body style='font-family:sans-serif'><h2>postmini: authorization complete</h2>You can close this window and return to postmini.</body></html>";
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
                return Ok(q);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Perform an OAuth 2.0 grant ("Get New Access Token")
pub fn oauth2_get_token(auth: &Auth, opts: &crate::http::HttpOptions) -> Result<TokenResult, String> {
    let grant = auth.get_or("grant_type", "authorization_code");
    match grant.as_str() {
        "client_credentials" => token_request(auth, vec![("grant_type".into(), "client_credentials".into())], opts),
        "password_credentials" | "password" => token_request(
            auth,
            vec![("grant_type".into(), "password".into()), ("username".into(), auth.get("username")), ("password".into(), auth.get("password"))],
            opts,
        ),
        "refresh_token" => token_request(auth, vec![("grant_type".into(), "refresh_token".into()), ("refresh_token".into(), auth.get("refreshToken"))], opts),
        "authorization_code" | "authorization_code_with_pkce" | "implicit" => {
            let redirect = { let r = auth.get("redirect_uri"); if r.is_empty() { "http://localhost:5555/callback".to_string() } else { r } };
            let state = { let s = auth.get("state"); if s.is_empty() { nonce(12) } else { s } };
            let pkce = grant == "authorization_code_with_pkce";
            let verifier = { let v = auth.get("code_verifier"); if v.is_empty() { nonce(64) } else { v } };
            let mut url = auth.get("authUrl");
            if url.is_empty() {
                return Err("Auth URL is required".into());
            }
            url = add_query(&url, "response_type", if grant == "implicit" { "token" } else { "code" });
            url = add_query(&url, "client_id", &auth.get("clientId"));
            url = add_query(&url, "redirect_uri", &redirect);
            url = add_query(&url, "state", &state);
            let scope = auth.get("scope");
            if !scope.is_empty() {
                url = add_query(&url, "scope", &scope);
            }
            if pkce {
                let method = auth.get_or("challengeAlgorithm", "S256");
                if method == "plain" {
                    url = add_query(&url, "code_challenge", &verifier);
                    url = add_query(&url, "code_challenge_method", "plain");
                } else {
                    url = add_query(&url, "code_challenge", &pkce_challenge(&verifier));
                    url = add_query(&url, "code_challenge_method", "S256");
                }
            }
            open_browser(&url);
            let params = wait_for_redirect(&redirect, 300)?;
            let get = |k: &str| params.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
            if !get("error").is_empty() {
                return Err(format!("Authorization error: {} {}", get("error"), get("error_description")));
            }
            if get("state") != state && !get("state").is_empty() {
                return Err("State mismatch in authorization response".into());
            }
            if grant == "implicit" {
                let t = get("access_token");
                if t.is_empty() {
                    return Err("No access_token returned".into());
                }
                return Ok(TokenResult { access_token: t, token_type: get("token_type"), refresh_token: String::new(), expires_in: get("expires_in").parse().ok(), raw: Value::Null });
            }
            let code = get("code");
            if code.is_empty() {
                return Err("No authorization code returned".into());
            }
            let mut p = vec![("grant_type".into(), "authorization_code".into()), ("code".into(), code), ("redirect_uri".into(), redirect)];
            if pkce {
                p.push(("code_verifier".into(), verifier));
            }
            token_request(auth, p, opts)
        }
        other => Err(format!("Grant type {other} not supported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digest_rfc2617() {
        // RFC 2617 example
        let mut a = Auth::new("digest");
        a.set_str("username", "Mufasa");
        a.set_str("password", "Circle Of Life");
        a.set_str("realm", "testrealm@host.com");
        a.set_str("nonce", "dcd98b7102dd2f0e8b11d0f600bfb0c093");
        a.set_str("qop", "auth");
        a.set_str("nonceCount", "00000001");
        a.set_str("clientNonce", "0a4f113b");
        a.set_str("opaque", "5ccc069c403ebaf9f0171e9517f40e41");
        let h = digest_header(&a, "GET", "http://host.com/dir/index.html", b"", None).unwrap();
        assert!(h.contains("response=\"6629fae49393a05397450978507c4ef1\""), "{h}");
    }
    #[test]
    fn jwt_hs256() {
        let mut a = Auth::new("jwt");
        a.set_str("algorithm", "HS256");
        a.set_str("secret", "secret");
        a.set_str("payload", "{\"sub\":\"1\"}");
        let mut r = HttpRequest { method: "GET".into(), url: "http://x/".into(), ..Default::default() };
        apply(&a, &mut r, &[]).unwrap();
        assert!(r.headers[0].1.starts_with("Bearer eyJ"));
    }
}
