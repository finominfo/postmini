use serde::{Deserialize, Serialize};

use crate::http::{ClientCert, ProxyCfg};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub timeout_ms: u64,
    pub verify_ssl: bool,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub send_no_cache: bool,
    pub send_postman_token: bool,
    pub user_agent: String,
    pub trim_keys_values: bool,
    pub max_response_mb: u64,
    /// "none" | "system" | "custom"
    pub proxy_mode: String,
    pub proxy: ProxyCfg,
    pub client_certs: Vec<ClientCert>,
    pub ca_file: String,
    pub dark: bool,
    pub zoom: f32,
    pub font_size: f32,
    pub auto_persist_vars: bool,
    pub history_limit: usize,
    pub script_timeout_ms: u64,
    pub working_dir: String,
    pub auto_save: bool,
    pub wrap_body: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            timeout_ms: 0,
            verify_ssl: true,
            follow_redirects: true,
            max_redirects: 10,
            send_no_cache: true,
            send_postman_token: true,
            user_agent: format!("postmini/{}", env!("CARGO_PKG_VERSION")),
            trim_keys_values: false,
            max_response_mb: 50,
            proxy_mode: "system".into(),
            proxy: ProxyCfg { url: String::new(), bypass: "localhost,127.0.0.1".into(), http: true, https: true },
            client_certs: vec![],
            ca_file: String::new(),
            dark: true,
            zoom: 1.0,
            font_size: 13.0,
            auto_persist_vars: true,
            history_limit: 300,
            script_timeout_ms: 10_000,
            working_dir: String::new(),
            auto_save: false,
            wrap_body: false,
        }
    }
}

impl Settings {
    pub fn effective_proxy(&self) -> Option<ProxyCfg> {
        match self.proxy_mode.as_str() {
            "custom" if !self.proxy.url.trim().is_empty() => Some(self.proxy.clone()),
            "system" => system_proxy(),
            _ => None,
        }
    }
}

/// System proxy from environment variables (HTTP_PROXY / HTTPS_PROXY / NO_PROXY),
/// and on Windows from the Internet Settings registry key.
pub fn system_proxy() -> Option<ProxyCfg> {
    let get = |k: &str| std::env::var(k).ok().or_else(|| std::env::var(k.to_lowercase()).ok()).filter(|s| !s.is_empty());
    let https = get("HTTPS_PROXY");
    let http = get("HTTP_PROXY");
    let all = get("ALL_PROXY");
    let no = get("NO_PROXY").unwrap_or_default();
    if let Some(u) = https.clone().or(http.clone()).or(all) {
        return Some(ProxyCfg { url: u, bypass: no, http: http.is_some() || https.is_none(), https: https.is_some() || http.is_some() });
    }
    #[cfg(windows)]
    {
        if let Some(p) = windows_proxy() {
            return Some(p);
        }
    }
    None
}

#[cfg(windows)]
fn windows_proxy() -> Option<ProxyCfg> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings"])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut enabled = false;
    let mut server = String::new();
    let mut bypass = String::new();
    for l in text.lines() {
        let parts: Vec<&str> = l.split_whitespace().collect();
        if parts.len() >= 3 {
            match parts[0] {
                "ProxyEnable" => enabled = parts[2] == "0x1",
                "ProxyServer" => server = parts[2..].join(" "),
                "ProxyOverride" => bypass = parts[2..].join(" ").replace(';', ",").replace("<local>", "localhost,127.0.0.1"),
                _ => {}
            }
        }
    }
    if !enabled || server.is_empty() {
        return None;
    }
    // "http=host:port;https=host:port" or "host:port"
    let url = if server.contains('=') {
        server.split(';').find(|s| s.starts_with("https=")).or_else(|| server.split(';').find(|s| s.starts_with("http="))).map(|s| s.split_once('=').unwrap().1.to_string()).unwrap_or_default()
    } else {
        server
    };
    Some(ProxyCfg { url: format!("http://{url}"), bypass, http: true, https: true })
}
