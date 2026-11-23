//! Minimal, dependency-light HTTP/1.1 client with precise timings (like Postman's
//! network panel): DNS, TCP, TLS, request, first byte, download.
//! Supports HTTP & HTTPS, HTTP/CONNECT & SOCKS5 proxies, redirects, cookie jar,
//! gzip/deflate/brotli, client certificates, custom CA, insecure mode.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};

pub type CookieJar = Arc<Mutex<cookie_store::CookieStore>>;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProxyCfg {
    /// http://user:pass@host:port, socks5://host:port
    pub url: String,
    /// comma separated host patterns that bypass the proxy
    #[serde(default)]
    pub bypass: String,
    /// which schemes to proxy
    #[serde(default = "yes")]
    pub http: bool,
    #[serde(default = "yes")]
    pub https: bool,
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ClientCert {
    /// host or host:port, may start with *.
    pub host: String,
    pub cert_path: String,
    pub key_path: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct HttpOptions {
    pub timeout_ms: u64,
    pub verify_ssl: bool,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub follow_original_method: bool,
    pub follow_auth_header: bool,
    pub remove_referer_on_redirect: bool,
    pub disable_cookies: bool,
    pub disable_url_encoding: bool,
    pub proxy: Option<ProxyCfg>,
    pub client_certs: Vec<ClientCert>,
    pub ca_file: Option<String>,
    pub max_response_bytes: usize,
    pub jar: Option<CookieJar>,
}

impl Default for HttpOptions {
    fn default() -> Self {
        HttpOptions {
            timeout_ms: 0,
            verify_ssl: true,
            follow_redirects: true,
            max_redirects: 10,
            follow_original_method: false,
            follow_auth_header: false,
            remove_referer_on_redirect: false,
            disable_cookies: false,
            disable_url_encoding: false,
            proxy: None,
            client_certs: vec![],
            ca_file: None,
            max_response_bytes: 50 * 1024 * 1024,
            jar: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Timings {
    pub dns_ms: f64,
    pub tcp_ms: f64,
    pub tls_ms: f64,
    pub request_ms: f64,
    pub ttfb_ms: f64,
    pub download_ms: f64,
    pub total_ms: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SentRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_len: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub header_bytes: usize,
    pub wire_body_bytes: usize,
    pub timings: Timings,
    pub remote_addr: String,
    pub local_addr: String,
    pub tls: Option<String>,
    pub sent: SentRequest,
    pub redirects: Vec<(u16, String)>,
    pub truncated: bool,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn content_type(&self) -> String {
        self.header("content-type").unwrap_or("").to_lowercase()
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn size_total(&self) -> usize {
        self.header_bytes + self.wire_body_bytes
    }
}

// ------------------------------------------------------------------ URL parsing

#[derive(Debug, Clone)]
pub struct ParsedUrl {
    pub scheme: String,
    pub userinfo: Option<String>,
    pub host: String,
    pub port: u16,
    pub port_explicit: bool,
    /// path + query (encoded)
    pub target: String,
}

impl ParsedUrl {
    pub fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.host_header())
    }
    pub fn host_header(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port_explicit && self.port != default_port(&self.scheme) {
            format!("{}:{}", h, self.port)
        } else {
            h
        }
    }
    pub fn to_string(&self) -> String {
        format!("{}{}", self.origin(), self.target)
    }
}

fn default_port(scheme: &str) -> u16 {
    match scheme {
        "https" | "wss" => 443,
        _ => 80,
    }
}

const fn is_unreserved_or_ok(b: u8) -> bool {
    // characters that we leave as-is (RFC3986 unreserved + reserved + '%')
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@' | b'/' | b'?' | b'%' | b'[' | b']')
}

pub fn encode_target(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'%' {
            // keep valid escapes, encode lone %
            let valid = i + 2 < bytes.len() + 0 && bytes.get(i + 1).map(|c| c.is_ascii_hexdigit()).unwrap_or(false) && bytes.get(i + 2).map(|c| c.is_ascii_hexdigit()).unwrap_or(false);
            if valid {
                out.push('%');
            } else {
                out.push_str("%25");
            }
        } else if is_unreserved_or_ok(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn parse_url(raw: &str, encode: bool) -> Result<ParsedUrl, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("URL is empty".into());
    }
    let (scheme, rest) = match raw.find("://") {
        Some(i) => (raw[..i].to_lowercase(), &raw[i + 3..]),
        None => ("http".to_string(), raw),
    };
    if !matches!(scheme.as_str(), "http" | "https" | "ws" | "wss") {
        return Err(format!("Unsupported protocol: {scheme}"));
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let mut target = rest[end..].to_string();
    if let Some(h) = target.find('#') {
        target.truncate(h);
    }
    if target.is_empty() || target.starts_with('?') {
        target.insert(0, '/');
    }
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(authority[..i].to_string()), &authority[i + 1..]),
        None => (None, authority),
    };
    let (host, port, explicit) = if let Some(stripped) = hostport.strip_prefix('[') {
        let close = stripped.find(']').ok_or("Invalid IPv6 host")?;
        let host = &stripped[..close];
        let after = &stripped[close + 1..];
        match after.strip_prefix(':') {
            Some(p) => (host.to_string(), p.parse::<u16>().map_err(|_| "Invalid port")?, true),
            None => (host.to_string(), default_port(&scheme), false),
        }
    } else {
        match hostport.rfind(':') {
            Some(i) => {
                let p = &hostport[i + 1..];
                (hostport[..i].to_string(), p.parse::<u16>().map_err(|_| format!("Invalid port: {p}"))?, true)
            }
            None => (hostport.to_string(), default_port(&scheme), false),
        }
    };
    if host.is_empty() {
        return Err("URL has no host".into());
    }
    if host.contains("{{") {
        return Err(format!("Unresolved variable in host: {host}"));
    }
    Ok(ParsedUrl { scheme, userinfo, host: host.to_lowercase(), port, port_explicit: explicit, target: if encode { encode_target(&target) } else { target } })
}

// ------------------------------------------------------------------ TLS

fn root_store(ca_file: &Option<String>) -> Arc<rustls::RootCertStore> {
    static BASE: OnceLock<rustls::RootCertStore> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        let mut store = rustls::RootCertStore::empty();
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        for c in native.certs {
            let _ = store.add(c);
        }
        store
    });
    let mut store = base.clone();
    if let Some(f) = ca_file {
        if !f.is_empty() {
            if let Ok(data) = std::fs::read(f) {
                let mut rd = io::BufReader::new(&data[..]);
                for c in rustls_pemfile::certs(&mut rd).flatten() {
                    let _ = store.add(c);
                }
            }
        }
    }
    Arc::new(store)
}

#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn host_matches(pattern: &str, host: &str, port: u16) -> bool {
    let pattern = pattern.trim().to_lowercase();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    let pattern = pattern.trim_start_matches("https://").trim_start_matches("http://");
    let (ph, pp) = match pattern.rfind(':') {
        Some(i) => (&pattern[..i], pattern[i + 1..].parse::<u16>().ok()),
        None => (pattern, None),
    };
    if let Some(p) = pp {
        if p != port {
            return false;
        }
    }
    if let Some(suffix) = ph.strip_prefix("*.") {
        host.ends_with(&format!(".{suffix}")) || host == suffix
    } else {
        ph == host
    }
}

pub fn tls_config(opts: &HttpOptions, host: &str, port: u16, alpn_http1: bool) -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions().map_err(|e| e.to_string())?;
    let builder = if opts.verify_ssl {
        builder.with_root_certificates(root_store(&opts.ca_file))
    } else {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    };
    let cert = opts.client_certs.iter().find(|c| c.enabled && host_matches(&c.host, host, port));
    let mut cfg = match cert {
        Some(c) => {
            let cert_data = std::fs::read(&c.cert_path).map_err(|e| format!("Client cert {}: {e}", c.cert_path))?;
            let key_data = std::fs::read(if c.key_path.is_empty() { &c.cert_path } else { &c.key_path }).map_err(|e| format!("Client key: {e}"))?;
            let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_data[..]).flatten().collect();
            let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut &key_data[..]).map_err(|e| e.to_string())?.ok_or("No private key found in key file")?;
            builder.with_client_auth_cert(certs, key).map_err(|e| e.to_string())?
        }
        None => builder.with_no_client_auth(),
    };
    if alpn_http1 {
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    }
    Ok(Arc::new(cfg))
}

// ------------------------------------------------------------------ streams

pub enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    pub fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => &s.sock,
        }
    }
    pub fn set_timeout(&self, d: Option<Duration>) {
        let _ = self.tcp().set_read_timeout(d);
        let _ = self.tcp().set_write_timeout(d);
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => match s.read(buf) {
                // servers that close without close_notify
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                r => r,
            },
        }
    }
}
impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

struct Deadline(Option<Instant>);
impl Deadline {
    fn remaining(&self) -> Result<Option<Duration>, String> {
        match self.0 {
            None => Ok(None),
            Some(d) => {
                let now = Instant::now();
                if now >= d {
                    Err("Request timed out".into())
                } else {
                    Ok(Some(d - now))
                }
            }
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn bypass_proxy(cfg: &ProxyCfg, host: &str, port: u16) -> bool {
    cfg.bypass.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).any(|p| p == "*" || host_matches(p, host, port) || (p.starts_with('.') && host.ends_with(p)))
}

/// Connection result with timing info
pub struct Connected {
    pub stream: Stream,
    pub dns_ms: f64,
    pub tcp_ms: f64,
    pub tls_ms: f64,
    pub via_http_proxy: bool,
    pub proxy_auth: Option<String>,
    pub remote: String,
    pub local: String,
    pub tls_info: Option<String>,
}

fn resolve(host: &str, port: u16, deadline: &Deadline) -> Result<Vec<SocketAddr>, String> {
    let _ = deadline.remaining()?;
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().map_err(|e| format!("getaddrinfo ENOTFOUND {host}: {e}"))?.collect();
    if addrs.is_empty() {
        return Err(format!("getaddrinfo ENOTFOUND {host}"));
    }
    Ok(addrs)
}

fn tcp_connect(addrs: &[SocketAddr], deadline: &Deadline) -> Result<TcpStream, String> {
    let mut last = String::new();
    for a in addrs {
        let r = match deadline.remaining()? {
            Some(t) => TcpStream::connect_timeout(a, t),
            None => TcpStream::connect_timeout(a, Duration::from_secs(30)),
        };
        match r {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = format!("connect {} {}", e, a),
        }
    }
    Err(if last.contains("refused") { format!("ECONNREFUSED: {last}") } else { last })
}

fn proxy_for(opts: &HttpOptions, u: &ParsedUrl) -> Option<(ParsedProxy, String)> {
    let p = opts.proxy.as_ref()?;
    if p.url.trim().is_empty() {
        return None;
    }
    let secure = matches!(u.scheme.as_str(), "https" | "wss");
    if (secure && !p.https) || (!secure && !p.http) || bypass_proxy(p, &u.host, u.port) {
        return None;
    }
    parse_proxy(&p.url).map(|pp| (pp, p.url.clone()))
}

pub struct ParsedProxy {
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub user: Option<(String, String)>,
}

fn parse_proxy(s: &str) -> Option<ParsedProxy> {
    let s = s.trim();
    let (kind, rest) = match s.find("://") {
        Some(i) => (s[..i].to_lowercase(), &s[i + 3..]),
        None => ("http".to_string(), s),
    };
    let rest = rest.trim_end_matches('/');
    let (user, hp) = match rest.rfind('@') {
        Some(i) => {
            let ui = &rest[..i];
            let (u, p) = match ui.find(':') {
                Some(j) => (&ui[..j], &ui[j + 1..]),
                None => (ui, ""),
            };
            let dec = |x: &str| percent_encoding::percent_decode_str(x).decode_utf8_lossy().into_owned();
            (Some((dec(u), dec(p))), &rest[i + 1..])
        }
        None => (None, rest),
    };
    let def = if kind.starts_with("socks") { 1080 } else if kind == "https" { 443 } else { 8080 };
    let (host, port) = match hp.rfind(':') {
        Some(i) => (hp[..i].to_string(), hp[i + 1..].parse().unwrap_or(def)),
        None => (hp.to_string(), def),
    };
    Some(ParsedProxy { kind, host, port, user })
}

fn read_line_raw<R: BufRead>(r: &mut R) -> io::Result<String> {
    let mut buf = Vec::new();
    r.read_until(b'\n', &mut buf)?;
    while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn socks5_connect(s: &mut TcpStream, host: &str, port: u16, user: &Option<(String, String)>) -> Result<(), String> {
    let e = |x: io::Error| format!("SOCKS5: {x}");
    let methods: &[u8] = if user.is_some() { &[0x05, 0x02, 0x00, 0x02] } else { &[0x05, 0x01, 0x00] };
    s.write_all(methods).map_err(e)?;
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).map_err(e)?;
    if resp[1] == 0x02 {
        let (u, p) = user.clone().unwrap_or_default();
        let mut m = vec![0x01, u.len() as u8];
        m.extend(u.as_bytes());
        m.push(p.len() as u8);
        m.extend(p.as_bytes());
        s.write_all(&m).map_err(e)?;
        s.read_exact(&mut resp).map_err(e)?;
        if resp[1] != 0 {
            return Err("SOCKS5 authentication failed".into());
        }
    } else if resp[1] != 0 {
        return Err("SOCKS5: no acceptable auth method".into());
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend(host.as_bytes());
    req.extend(port.to_be_bytes());
    s.write_all(&req).map_err(e)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).map_err(e)?;
    if head[1] != 0 {
        return Err(format!("SOCKS5 connect failed (code {})", head[1]));
    }
    let skip = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).map_err(e)?;
            l[0] as usize
        }
        _ => 0,
    } + 2;
    let mut tmp = vec![0u8; skip];
    s.read_exact(&mut tmp).map_err(e)?;
    Ok(())
}

pub fn connect(u: &ParsedUrl, opts: &HttpOptions, timeout_ms: u64, alpn_http1: bool) -> Result<Connected, String> {
    let deadline = Deadline(if timeout_ms > 0 { Some(Instant::now() + Duration::from_millis(timeout_ms)) } else { None });
    connect_dl(u, opts, &deadline, alpn_http1)
}

fn connect_dl(u: &ParsedUrl, opts: &HttpOptions, deadline: &Deadline, alpn_http1: bool) -> Result<Connected, String> {
    let secure = matches!(u.scheme.as_str(), "https" | "wss");
    let proxy = proxy_for(opts, u);
    let t0 = Instant::now();
    let (connect_host, connect_port) = match &proxy {
        Some((p, _)) => (p.host.clone(), p.port),
        None => (u.host.clone(), u.port),
    };
    let addrs = resolve(&connect_host, connect_port, deadline)?;
    let dns_ms = ms(t0.elapsed());
    let t1 = Instant::now();
    let mut tcp = tcp_connect(&addrs, deadline)?;
    let t = deadline.remaining()?.or(Some(Duration::from_secs(120)));
    let _ = tcp.set_read_timeout(t);
    let _ = tcp.set_write_timeout(t);
    let mut via_http_proxy = false;
    let mut proxy_auth = None;
    if let Some((p, _)) = &proxy {
        if p.kind.starts_with("socks") {
            socks5_connect(&mut tcp, &u.host, u.port, &p.user)?;
        } else {
            if let Some((user, pass)) = &p.user {
                proxy_auth = Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))));
            }
            if secure {
                // CONNECT tunnel
                let mut req = format!("CONNECT {h}:{p} HTTP/1.1\r\nHost: {h}:{p}\r\n", h = u.host, p = u.port);
                if let Some(a) = &proxy_auth {
                    req.push_str(&format!("Proxy-Authorization: {a}\r\n"));
                }
                req.push_str("\r\n");
                tcp.write_all(req.as_bytes()).map_err(|e| format!("Proxy: {e}"))?;
                let mut rd = BufReader::new(&mut tcp);
                let status = read_line_raw(&mut rd).map_err(|e| format!("Proxy: {e}"))?;
                loop {
                    let l = read_line_raw(&mut rd).map_err(|e| format!("Proxy: {e}"))?;
                    if l.is_empty() {
                        break;
                    }
                }
                let code: u16 = status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
                if code != 200 {
                    return Err(format!("Proxy CONNECT failed: {status}"));
                }
            } else {
                via_http_proxy = true;
            }
        }
    }
    let tcp_ms = ms(t1.elapsed());
    let remote = tcp.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    let local = tcp.local_addr().map(|a| a.to_string()).unwrap_or_default();
    let mut tls_ms = 0.0;
    let mut tls_info = None;
    let stream = if secure {
        let t2 = Instant::now();
        let cfg = tls_config(opts, &u.host, u.port, alpn_http1)?;
        let name = ServerName::try_from(u.host.clone()).map_err(|e| format!("Invalid server name: {e}"))?;
        let mut conn = rustls::ClientConnection::new(cfg, name).map_err(|e| e.to_string())?;
        while conn.is_handshaking() {
            conn.complete_io(&mut tcp).map_err(|e| {
                let s = e.to_string();
                if s.contains("UnknownIssuer") || s.contains("certificate") {
                    format!("SSL Error: {s} (disable SSL certificate verification in Settings to ignore)")
                } else {
                    format!("TLS handshake failed: {s}")
                }
            })?;
        }
        tls_ms = ms(t2.elapsed());
        tls_info = Some(format!(
            "{:?} {:?}",
            conn.protocol_version().map(|v| format!("{v:?}")).unwrap_or_default(),
            conn.negotiated_cipher_suite().map(|c| format!("{:?}", c.suite())).unwrap_or_default()
        )
        .replace('"', ""));
        Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp)))
    } else {
        Stream::Plain(tcp)
    };
    Ok(Connected { stream, dns_ms, tcp_ms, tls_ms, via_http_proxy, proxy_auth, remote, local, tls_info })
}

// ------------------------------------------------------------------ request execution

fn read_body<R: BufRead>(rd: &mut R, headers: &[(String, String)], method: &str, status: u16, limit: usize, deadline: &Deadline) -> Result<(Vec<u8>, usize, bool), String> {
    let get = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string());
    if method == "HEAD" || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok((vec![], 0, false));
    }
    let mut body = Vec::new();
    let mut truncated = false;
    let te = get("transfer-encoding").unwrap_or_default().to_lowercase();
    let mut wire = 0usize;
    if te.contains("chunked") {
        loop {
            deadline.remaining()?;
            let line = read_line_raw(rd).map_err(|e| e.to_string())?;
            wire += line.len() + 2;
            let size_s = line.split(';').next().unwrap_or("").trim();
            if size_s.is_empty() {
                // eof
                break;
            }
            let size = usize::from_str_radix(size_s, 16).map_err(|_| format!("Invalid chunk size: {line}"))?;
            if size == 0 {
                // trailers
                loop {
                    let l = read_line_raw(rd).map_err(|e| e.to_string())?;
                    if l.is_empty() {
                        break;
                    }
                }
                break;
            }
            let mut chunk = vec![0u8; size];
            rd.read_exact(&mut chunk).map_err(|e| e.to_string())?;
            wire += size + 2;
            if body.len() + size > limit {
                truncated = true;
            } else {
                body.extend_from_slice(&chunk);
            }
            let mut crlf = [0u8; 2];
            let _ = rd.read_exact(&mut crlf);
        }
    } else if let Some(cl) = get("content-length").and_then(|c| c.parse::<usize>().ok()) {
        let mut remaining = cl;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            deadline.remaining()?;
            let n = rd.read(&mut buf[..remaining.min(64 * 1024)]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            remaining -= n;
            wire += n;
            if body.len() + n > limit {
                truncated = true;
            } else {
                body.extend_from_slice(&buf[..n]);
            }
        }
    } else {
        // read until close
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            deadline.remaining()?;
            let n = match rd.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset || e.kind() == io::ErrorKind::ConnectionAborted => 0,
                Err(e) => return Err(e.to_string()),
            };
            if n == 0 {
                break;
            }
            wire += n;
            if body.len() + n > limit {
                truncated = true;
            } else {
                body.extend_from_slice(&buf[..n]);
            }
        }
    }
    Ok((body, wire, truncated))
}

pub fn decompress(body: Vec<u8>, encoding: &str) -> Vec<u8> {
    let enc = encoding.trim().to_lowercase();
    if body.is_empty() {
        return body;
    }
    let mut out = Vec::new();
    let ok = match enc.as_str() {
        "gzip" | "x-gzip" => flate2::read::MultiGzDecoder::new(&body[..]).read_to_end(&mut out).is_ok(),
        "deflate" => flate2::read::ZlibDecoder::new(&body[..]).read_to_end(&mut out).is_ok() || {
            out.clear();
            flate2::read::DeflateDecoder::new(&body[..]).read_to_end(&mut out).is_ok()
        },
        "br" => brotli_decompressor::Decompressor::new(&body[..], 4096).read_to_end(&mut out).is_ok(),
        _ => return body,
    };
    if ok {
        out
    } else {
        body
    }
}

fn add_cookie_header(opts: &HttpOptions, url: &str, headers: &mut Vec<(String, String)>) {
    if opts.disable_cookies {
        return;
    }
    let Some(jar) = &opts.jar else { return };
    let Ok(u) = url::Url::parse(url) else { return };
    let store = jar.lock().unwrap();
    let cookies: Vec<String> = store.get_request_values(&u).map(|(n, v)| format!("{n}={v}")).collect();
    if cookies.is_empty() {
        return;
    }
    let joined = cookies.join("; ");
    if let Some(h) = headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case("cookie")) {
        // user-provided cookies take precedence: append jar cookies that are not already present
        let existing: Vec<String> = h.1.split(';').map(|c| c.trim().split('=').next().unwrap_or("").to_string()).collect();
        let extra: Vec<String> = cookies.into_iter().filter(|c| !existing.contains(&c.split('=').next().unwrap_or("").to_string())).collect();
        if !extra.is_empty() {
            h.1 = format!("{}; {}", h.1, extra.join("; "));
        }
    } else {
        headers.push(("Cookie".into(), joined));
    }
}

fn store_cookies(opts: &HttpOptions, url: &str, headers: &[(String, String)]) {
    if opts.disable_cookies {
        return;
    }
    let Some(jar) = &opts.jar else { return };
    let Ok(u) = url::Url::parse(url) else { return };
    let mut store = jar.lock().unwrap();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("set-cookie") {
            let _ = store.parse(v, &u);
        }
    }
}

/// Execute a single HTTP exchange (no redirects)
fn exchange(req: &HttpRequest, opts: &HttpOptions, deadline: &Deadline) -> Result<HttpResponse, String> {
    let u = parse_url(&req.url, !opts.disable_url_encoding)?;
    let start = Instant::now();
    let mut conn = connect_dl(&u, opts, deadline, true)?;
    let request_target = if conn.via_http_proxy { u.to_string() } else { u.target.clone() };

    // assemble headers
    let mut headers: Vec<(String, String)> = Vec::new();
    if !req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        headers.push(("Host".into(), u.host_header()));
    }
    for (k, v) in &req.headers {
        headers.push((k.clone(), v.clone()));
    }
    if let Some(ui) = &u.userinfo {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
            let dec = percent_encoding::percent_decode_str(ui).decode_utf8_lossy().into_owned();
            headers.push(("Authorization".into(), format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(dec))));
        }
    }
    let full_url = u.to_string();
    add_cookie_header(opts, &full_url, &mut headers);
    let has_body = !req.body.is_empty() || matches!(req.method.as_str(), "POST" | "PUT" | "PATCH");
    if has_body && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length") || k.eq_ignore_ascii_case("transfer-encoding")) {
        headers.push(("Content-Length".into(), req.body.len().to_string()));
    }
    if let Some(pa) = &conn.proxy_auth {
        if conn.via_http_proxy {
            headers.push(("Proxy-Authorization".into(), pa.clone()));
        }
    }
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("connection")) {
        headers.push(("Connection".into(), "close".into()));
    }

    let mut head = format!("{} {} HTTP/1.1\r\n", req.method, request_target);
    for (k, v) in &headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let t_req = Instant::now();
    conn.stream.set_timeout(deadline.remaining()?.or(Some(Duration::from_secs(300))));
    conn.stream.write_all(head.as_bytes()).map_err(|e| format!("Write failed: {e}"))?;
    if !req.body.is_empty() {
        conn.stream.write_all(&req.body).map_err(|e| format!("Write failed: {e}"))?;
    }
    conn.stream.flush().map_err(|e| e.to_string())?;
    let request_ms = ms(t_req.elapsed());
    let t_wait = Instant::now();
    let mut rd = BufReader::with_capacity(64 * 1024, conn.stream);
    // wait for first byte
    let first = rd.fill_buf().map_err(|e| {
        if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
            "ESOCKETTIMEDOUT: Response timed out".to_string()
        } else {
            format!("Error reading response: {e}")
        }
    })?;
    if first.is_empty() {
        return Err("socket hang up (server closed connection without response)".into());
    }
    let ttfb_ms = ms(t_wait.elapsed());
    let t_dl = Instant::now();

    // status + headers (skip 1xx)
    let (status, reason, version, resp_headers, header_bytes) = loop {
        let status_line = read_line_raw(&mut rd).map_err(|e| e.to_string())?;
        let mut header_bytes = status_line.len() + 2;
        let mut parts = status_line.splitn(3, ' ');
        let version = parts.next().unwrap_or("").to_string();
        if !version.starts_with("HTTP/") {
            return Err(format!("Invalid HTTP response: {}", status_line.chars().take(80).collect::<String>()));
        }
        let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or("Invalid status code")?;
        let reason = parts.next().unwrap_or("").to_string();
        let mut hs = Vec::new();
        loop {
            let l = read_line_raw(&mut rd).map_err(|e| e.to_string())?;
            header_bytes += l.len() + 2;
            if l.is_empty() {
                break;
            }
            if let Some(i) = l.find(':') {
                hs.push((l[..i].trim().to_string(), l[i + 1..].trim().to_string()));
            }
        }
        if (100..200).contains(&status) && status != 101 {
            continue;
        }
        break (status, reason, version.trim_start_matches("HTTP/").to_string(), hs, header_bytes);
    };
    let (body, wire, truncated) = read_body(&mut rd, &resp_headers, &req.method, status, opts.max_response_bytes, deadline)?;
    let enc = resp_headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-encoding")).map(|(_, v)| v.clone()).unwrap_or_default();
    let body = decompress(body, &enc);
    let download_ms = ms(t_dl.elapsed());
    store_cookies(opts, &full_url, &resp_headers);
    Ok(HttpResponse {
        status,
        reason: if reason.is_empty() { status_text(status).to_string() } else { reason },
        version,
        headers: resp_headers,
        body,
        header_bytes,
        wire_body_bytes: wire,
        timings: Timings { dns_ms: conn.dns_ms, tcp_ms: conn.tcp_ms, tls_ms: conn.tls_ms, request_ms, ttfb_ms, download_ms, total_ms: ms(start.elapsed()) },
        remote_addr: conn.remote,
        local_addr: conn.local,
        tls: conn.tls_info,
        sent: SentRequest { method: req.method.clone(), url: full_url, headers, body_len: req.body.len() },
        redirects: vec![],
        truncated,
    })
}

/// Execute request following redirects according to options.
pub fn execute(req: &HttpRequest, opts: &HttpOptions) -> Result<HttpResponse, String> {
    let deadline = Deadline(if opts.timeout_ms > 0 { Some(Instant::now() + Duration::from_millis(opts.timeout_ms)) } else { None });
    let start = Instant::now();
    let mut cur = req.clone();
    let mut hops: Vec<(u16, String)> = vec![];
    let mut total_timings = Timings::default();
    loop {
        let mut resp = exchange(&cur, opts, &deadline)?;
        total_timings.dns_ms += resp.timings.dns_ms;
        total_timings.tcp_ms += resp.timings.tcp_ms;
        total_timings.tls_ms += resp.timings.tls_ms;
        let is_redirect = matches!(resp.status, 301 | 302 | 303 | 307 | 308);
        let location = resp.header("location").map(|s| s.to_string());
        if is_redirect && opts.follow_redirects && location.is_some() && (hops.len() as u32) < opts.max_redirects {
            let loc = location.unwrap();
            let base = url::Url::parse(&resp.sent.url).map_err(|e| e.to_string())?;
            let next = base.join(&loc).map_err(|e| format!("Invalid redirect location {loc}: {e}"))?;
            hops.push((resp.status, resp.sent.url.clone()));
            let old_host = base.host_str().unwrap_or("").to_string();
            let mut nreq = cur.clone();
            nreq.url = next.to_string();
            if matches!(resp.status, 301 | 302 | 303) && !opts.follow_original_method && !(resp.status != 303 && cur.method == "HEAD") {
                if cur.method != "GET" && cur.method != "HEAD" {
                    nreq.method = "GET".into();
                    nreq.body.clear();
                    nreq.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type") && !k.eq_ignore_ascii_case("content-length"));
                }
            }
            if next.host_str().unwrap_or("") != old_host && !opts.follow_auth_header {
                nreq.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
            }
            nreq.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("host"));
            nreq.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("referer"));
            if !opts.remove_referer_on_redirect {
                nreq.headers.push(("Referer".into(), resp.sent.url.clone()));
            }
            cur = nreq;
            continue;
        }
        resp.redirects = hops;
        resp.timings.dns_ms = total_timings.dns_ms;
        resp.timings.tcp_ms = total_timings.tcp_ms;
        resp.timings.tls_ms = total_timings.tls_ms;
        resp.timings.total_ms = ms(start.elapsed());
        return Ok(resp);
    }
}

pub fn status_text(code: u16) -> &'static str {
    match code {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        422 => "Unprocessable Entity",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

/// Cookie jar helpers
pub fn new_jar() -> CookieJar {
    Arc::new(Mutex::new(cookie_store::CookieStore::default()))
}

pub fn load_jar(path: &std::path::Path) -> CookieJar {
    let store = std::fs::File::open(path)
        .ok()
        .and_then(|f| cookie_store::serde::json::load_all(io::BufReader::new(f)).ok())
        .unwrap_or_default();
    Arc::new(Mutex::new(store))
}

pub fn save_jar(jar: &CookieJar, path: &std::path::Path) {
    if let Ok(mut f) = std::fs::File::create(path) {
        let store = jar.lock().unwrap();
        let _ = cookie_store::serde::json::save_incl_expired_and_nonpersistent(&store, &mut f);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CookieInfo {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: String,
    pub http_only: bool,
    pub secure: bool,
}

pub fn list_cookies(jar: &CookieJar) -> Vec<CookieInfo> {
    let store = jar.lock().unwrap();
    store
        .iter_any()
        .map(|c| CookieInfo {
            name: c.name().to_string(),
            value: c.value().to_string(),
            domain: match &c.domain {
                cookie_store::CookieDomain::HostOnly(d) | cookie_store::CookieDomain::Suffix(d) => d.clone(),
                _ => c.domain().unwrap_or("").to_string(),
            },
            path: c.path().unwrap_or("/").to_string(),
            expires: match &c.expires {
                cookie_store::CookieExpiration::AtUtc(t) => t.to_string(),
                cookie_store::CookieExpiration::SessionEnd => "Session".into(),
            },
            http_only: c.http_only().unwrap_or(false),
            secure: c.secure().unwrap_or(false),
        })
        .collect()
}

pub fn remove_cookie(jar: &CookieJar, domain: &str, path: &str, name: &str) {
    let mut store = jar.lock().unwrap();
    store.remove(domain, path, name);
}

pub fn add_cookie(jar: &CookieJar, domain: &str, set_cookie: &str) -> Result<(), String> {
    let d = domain.trim().trim_start_matches('.');
    let u = url::Url::parse(&format!("https://{d}/")).map_err(|e| e.to_string())?;
    let mut store = jar.lock().unwrap();
    let raw = if set_cookie.to_lowercase().contains("domain=") { set_cookie.to_string() } else { format!("{set_cookie}; Domain={d}") };
    store.parse(&raw, &u).map(|_| ()).map_err(|e| e.to_string())
}

pub fn clear_cookies(jar: &CookieJar, domain: Option<&str>) {
    let mut store = jar.lock().unwrap();
    match domain {
        None => store.clear(),
        Some(d) => {
            let list: Vec<(String, String, String)> = store
                .iter_any()
                .filter(|c| c.domain().map(|x| x.trim_start_matches('.') == d.trim_start_matches('.')).unwrap_or(false) || matches!(&c.domain, cookie_store::CookieDomain::HostOnly(h) | cookie_store::CookieDomain::Suffix(h) if h == d))
                .map(|c| (match &c.domain {
                    cookie_store::CookieDomain::HostOnly(h) | cookie_store::CookieDomain::Suffix(h) => h.clone(),
                    _ => d.to_string(),
                }, c.path().unwrap_or("/").to_string(), c.name().to_string()))
                .collect();
            for (d, p, n) in list {
                store.remove(&d, &p, &n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_parsing() {
        let u = parse_url("https://Example.com:8443/a b?x=1 2&y=%20#frag", true).unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 8443);
        assert_eq!(u.target, "/a%20b?x=1%202&y=%20");
        let u = parse_url("localhost:3000", true).unwrap();
        assert_eq!(u.scheme, "http");
        assert_eq!(u.target, "/");
        let u = parse_url("http://[::1]:8080/x", true).unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.host_header(), "[::1]:8080");
    }
}
