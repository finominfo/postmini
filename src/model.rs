//! Postman Collection v2.1 compatible data model.
//! Unknown fields are preserved via `extra` maps so round-tripping a
//! collection exported by Postman does not lose information.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

pub const SCHEMA_V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Accept any JSON value and turn it into a string (Postman exports numbers/bools/null as values sometimes).
pub fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(value_to_string(&v))
}

pub fn value_to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v {
        Value::Bool(b) => b,
        Value::String(s) => s == "true",
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    })
}

fn is_false(b: &bool) -> bool {
    !*b
}

// ---------------------------------------------------------------- Description
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Description(pub String);

impl Serialize for Description {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for Description {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(Description(match v {
            Value::String(s) => s,
            Value::Object(m) => m.get("content").map(value_to_string).unwrap_or_default(),
            Value::Null => String::new(),
            o => o.to_string(),
        }))
    }
}

fn desc_empty(d: &Option<Description>) -> bool {
    d.as_ref().map(|d| d.0.is_empty()).unwrap_or(true)
}

// ---------------------------------------------------------------- Key/Value
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct KV {
    #[serde(default, deserialize_with = "lenient_string")]
    pub key: String,
    #[serde(default, deserialize_with = "lenient_string", skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default, deserialize_with = "lenient_bool", skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "desc_empty")]
    pub description: Option<Description>,
    /// form-data: "text" | "file"
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// form-data file path(s)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<Value>,
    #[serde(rename = "contentType", default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl KV {
    pub fn new(k: &str, v: &str) -> Self {
        KV { key: k.into(), value: v.into(), ..Default::default() }
    }
    pub fn is_file(&self) -> bool {
        self.kind.as_deref() == Some("file")
    }
    pub fn src_path(&self) -> String {
        match &self.src {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(a)) => a.first().map(value_to_string).unwrap_or_default(),
            _ => String::new(),
        }
    }
    pub fn desc(&self) -> String {
        self.description.as_ref().map(|d| d.0.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- Url
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Url {
    pub raw: String,
    pub query: Vec<KV>,
    /// path variables (`:id`)
    pub variable: Vec<KV>,
    pub extra: Map<String, Value>,
}

impl Url {
    pub fn from_raw(raw: &str) -> Self {
        let mut u = Url { raw: raw.to_string(), ..Default::default() };
        u.sync_from_raw();
        u
    }

    /// Re-parse query params and path variables from `raw`, keeping disabled params & descriptions.
    pub fn sync_from_raw(&mut self) {
        let raw = self.raw.clone();
        let (base, query) = split_query(&raw);
        let disabled: Vec<KV> = self.query.iter().filter(|q| q.disabled).cloned().collect();
        let mut new_q: Vec<KV> = Vec::new();
        if let Some(q) = query {
            for part in q.split('&') {
                if part.is_empty() {
                    continue;
                }
                let (k, v) = match part.find('=') {
                    Some(i) => (&part[..i], &part[i + 1..]),
                    None => (part, ""),
                };
                let mut kv = KV::new(k, v);
                if let Some(old) = self.query.iter().find(|o| !o.disabled && o.key == k) {
                    kv.description = old.description.clone();
                }
                new_q.push(kv);
            }
        }
        // keep disabled params at their relative position (append)
        new_q.extend(disabled);
        self.query = new_q;
        // path variables
        let names = path_variable_names(base);
        let mut vars = Vec::new();
        for n in names {
            let old = self.variable.iter().find(|v| v.key == n).cloned();
            vars.push(old.unwrap_or_else(|| KV::new(&n, "")));
        }
        self.variable = vars;
    }

    /// Rebuild `raw` from the query table (after the user edited params).
    pub fn sync_raw_from_query(&mut self) {
        let (base, _) = split_query(&self.raw);
        let base = base.to_string();
        let qs: Vec<String> = self
            .query
            .iter()
            .filter(|q| !q.disabled && !(q.key.is_empty() && q.value.is_empty()))
            .map(|q| if q.value.is_empty() && !q.extra.contains_key("_eq") { q.key.clone() } else { format!("{}={}", q.key, q.value) })
            .collect();
        self.raw = if qs.is_empty() { base } else { format!("{}?{}", base, qs.join("&")) };
        let hash = self.extra.get("hash").and_then(|h| h.as_str()).map(|s| s.to_string());
        if let Some(h) = hash {
            if !h.is_empty() && !self.raw.contains('#') {
                self.raw.push('#');
                self.raw.push_str(&h);
            }
        }
    }
}

pub fn split_query(raw: &str) -> (&str, Option<&str>) {
    let no_hash = match raw.find('#') {
        Some(i) => &raw[..i],
        None => raw,
    };
    match no_hash.find('?') {
        Some(i) => (&no_hash[..i], Some(&no_hash[i + 1..])),
        None => (no_hash, None),
    }
}

pub fn path_variable_names(base: &str) -> Vec<String> {
    // skip protocol + host
    let after_proto = match base.find("://") {
        Some(i) => &base[i + 3..],
        None => base,
    };
    let path = match after_proto.find('/') {
        Some(i) => &after_proto[i..],
        None => "",
    };
    let mut out = Vec::new();
    for seg in path.split('/') {
        if let Some(name) = seg.strip_prefix(':') {
            if !name.is_empty() && !out.iter().any(|n: &String| n == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}

impl Serialize for Url {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = Map::new();
        m.insert("raw".into(), Value::String(self.raw.clone()));
        let (base, _) = split_query(&self.raw);
        let (proto, rest) = match base.find("://") {
            Some(i) => (Some(&base[..i]), &base[i + 3..]),
            None => (None, base),
        };
        if let Some(p) = proto {
            m.insert("protocol".into(), Value::String(p.to_string()));
        }
        let (hostport, path) = match rest.find('/') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        let (host, port) = if hostport.starts_with("{{") {
            (hostport, None)
        } else {
            match hostport.rfind(':') {
                Some(i) if !hostport[i + 1..].contains(']') && !hostport[i + 1..].is_empty() => (&hostport[..i], Some(&hostport[i + 1..])),
                _ => (hostport, None),
            }
        };
        if !host.is_empty() {
            m.insert("host".into(), Value::Array(host.split('.').map(|p| Value::String(p.into())).collect()));
        }
        if let Some(p) = port {
            m.insert("port".into(), Value::String(p.into()));
        }
        if let Some(p) = path {
            m.insert("path".into(), Value::Array(p.split('/').map(|p| Value::String(p.into())).collect()));
        }
        if !self.query.is_empty() {
            m.insert("query".into(), serde_json::to_value(&self.query).unwrap_or(Value::Null));
        }
        if !self.variable.is_empty() {
            m.insert("variable".into(), serde_json::to_value(&self.variable).unwrap_or(Value::Null));
        }
        for (k, v) in &self.extra {
            if !m.contains_key(k) && !matches!(k.as_str(), "protocol" | "host" | "path" | "port") {
                m.insert(k.clone(), v.clone());
            }
        }
        Value::Object(m).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Url {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(match v {
            Value::String(s) => Url::from_raw(&s),
            Value::Object(mut m) => {
                let query: Vec<KV> = m.remove("query").and_then(|q| serde_json::from_value(q).ok()).unwrap_or_default();
                let variable: Vec<KV> = m.remove("variable").and_then(|q| serde_json::from_value(q).ok()).unwrap_or_default();
                let raw = match m.remove("raw") {
                    Some(Value::String(r)) => r,
                    _ => {
                        // build from parts
                        let mut r = String::new();
                        if let Some(p) = m.get("protocol").and_then(|p| p.as_str()) {
                            r.push_str(p);
                            r.push_str("://");
                        }
                        match m.get("host") {
                            Some(Value::Array(a)) => r.push_str(&a.iter().map(value_to_string).collect::<Vec<_>>().join(".")),
                            Some(Value::String(h)) => r.push_str(h),
                            _ => {}
                        }
                        if let Some(p) = m.get("port") {
                            r.push(':');
                            r.push_str(&value_to_string(p));
                        }
                        match m.get("path") {
                            Some(Value::Array(a)) => {
                                for p in a {
                                    r.push('/');
                                    r.push_str(&value_to_string(p));
                                }
                            }
                            Some(Value::String(p)) => {
                                if !p.starts_with('/') {
                                    r.push('/');
                                }
                                r.push_str(p)
                            }
                            _ => {}
                        }
                        let qs: Vec<String> = query.iter().filter(|q| !q.disabled).map(|q| format!("{}={}", q.key, q.value)).collect();
                        if !qs.is_empty() {
                            r.push('?');
                            r.push_str(&qs.join("&"));
                        }
                        r
                    }
                };
                for k in ["protocol", "host", "path", "port"] {
                    m.remove(k);
                }
                Url { raw, query, variable, extra: m }
            }
            _ => Url::default(),
        })
    }
}

// ---------------------------------------------------------------- Auth
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Auth {
    pub kind: String,
    /// Parameters for the current type (key -> value)
    pub params: Vec<(String, Value)>,
    /// Parameters of other types are preserved
    pub other: Map<String, Value>,
}

pub const AUTH_TYPES: &[(&str, &str)] = &[
    ("inherit", "Inherit auth from parent"),
    ("noauth", "No Auth"),
    ("basic", "Basic Auth"),
    ("bearer", "Bearer Token"),
    ("jwt", "JWT Bearer"),
    ("digest", "Digest Auth"),
    ("oauth1", "OAuth 1.0"),
    ("oauth2", "OAuth 2.0"),
    ("hawk", "Hawk Authentication"),
    ("awsv4", "AWS Signature"),
    ("ntlm", "NTLM Authentication"),
    ("apikey", "API Key"),
    ("edgegrid", "Akamai EdgeGrid"),
    ("asap", "ASAP (Atlassian)"),
];

impl Auth {
    pub fn new(kind: &str) -> Self {
        Auth { kind: kind.into(), ..Default::default() }
    }
    pub fn get(&self, key: &str) -> String {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| value_to_string(v)).unwrap_or_default()
    }
    pub fn get_or(&self, key: &str, def: &str) -> String {
        match self.params.iter().find(|(k, _)| k == key) {
            Some((_, v)) => value_to_string(v),
            None => def.to_string(),
        }
    }
    pub fn get_bool(&self, key: &str) -> bool {
        match self.params.iter().find(|(k, _)| k == key) {
            Some((_, Value::Bool(b))) => *b,
            Some((_, Value::String(s))) => s == "true",
            _ => false,
        }
    }
    pub fn set(&mut self, key: &str, v: Value) {
        if let Some(p) = self.params.iter_mut().find(|(k, _)| k == key) {
            p.1 = v;
        } else {
            self.params.push((key.into(), v));
        }
    }
    pub fn set_str(&mut self, key: &str, v: &str) {
        self.set(key, Value::String(v.into()));
    }
    /// Switch type, storing current params into `other`, restoring params of the new type.
    pub fn switch_to(&mut self, kind: &str) {
        if self.kind == kind {
            return;
        }
        let cur = self.params_value();
        if !self.kind.is_empty() && !self.params.is_empty() {
            self.other.insert(self.kind.clone(), cur);
        }
        self.kind = kind.into();
        self.params = self.other.remove(kind).map(parse_auth_params).unwrap_or_default();
    }
    fn params_value(&self) -> Value {
        Value::Array(
            self.params
                .iter()
                .map(|(k, v)| {
                    let ty = match v {
                        Value::Bool(_) => "boolean",
                        Value::Object(_) | Value::Array(_) => "any",
                        _ => "string",
                    };
                    serde_json::json!({"key": k, "value": v, "type": ty})
                })
                .collect(),
        )
    }
}

fn parse_auth_params(v: Value) -> Vec<(String, Value)> {
    match v {
        Value::Array(a) => a
            .into_iter()
            .filter_map(|e| {
                let k = e.get("key")?.as_str()?.to_string();
                let v = e.get("value").cloned().unwrap_or(Value::Null);
                Some((k, v))
            })
            .collect(),
        Value::Object(m) => m.into_iter().collect(),
        _ => vec![],
    }
}

impl Serialize for Auth {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = Map::new();
        m.insert("type".into(), Value::String(self.kind.clone()));
        if !matches!(self.kind.as_str(), "noauth" | "inherit") {
            m.insert(self.kind.clone(), self.params_value());
        }
        for (k, v) in &self.other {
            if !m.contains_key(k) {
                m.insert(k.clone(), v.clone());
            }
        }
        Value::Object(m).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Auth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let mut m = match v {
            Value::Object(m) => m,
            _ => return Ok(Auth::new("noauth")),
        };
        let kind = m.remove("type").map(|t| value_to_string(&t)).unwrap_or_else(|| "noauth".into());
        let params = m.remove(&kind).map(parse_auth_params).unwrap_or_default();
        Ok(Auth { kind, params, other: m })
    }
}

// ---------------------------------------------------------------- Body
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Body {
    #[serde(default)]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urlencoded: Vec<KV>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formdata: Vec<KV>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphql: Option<GraphQl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct GraphQl {
    #[serde(default, deserialize_with = "lenient_string")]
    pub query: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub variables: String,
}

impl Body {
    pub fn raw_language(&self) -> String {
        self.options
            .as_ref()
            .and_then(|o| o.get("raw"))
            .and_then(|r| r.get("language"))
            .and_then(|l| l.as_str())
            .unwrap_or("text")
            .to_string()
    }
    pub fn set_raw_language(&mut self, lang: &str) {
        let mut o = self.options.take().unwrap_or_else(|| serde_json::json!({}));
        if !o.is_object() {
            o = serde_json::json!({});
        }
        o["raw"] = serde_json::json!({ "language": lang });
        self.options = Some(o);
    }
    pub fn file_src(&self) -> String {
        self.file.as_ref().and_then(|f| f.get("src")).map(value_to_string).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- Request
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub header: Vec<KV>,
    pub body: Option<Body>,
    pub url: Url,
    pub auth: Option<Auth>,
    pub description: Option<Description>,
    pub extra: Map<String, Value>,
}

impl Default for Request {
    fn default() -> Self {
        Request { method: "GET".into(), header: vec![], body: None, url: Url::default(), auth: None, description: None, extra: Map::new() }
    }
}

#[derive(Serialize, Deserialize)]
struct RequestRepr {
    #[serde(default = "default_method")]
    method: String,
    #[serde(default, deserialize_with = "lenient_headers")]
    header: Vec<KV>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Body>,
    #[serde(default)]
    url: Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "desc_empty")]
    description: Option<Description>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn default_method() -> String {
    "GET".into()
}

fn lenient_headers<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<KV>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v {
        Value::Array(_) => serde_json::from_value(v).unwrap_or_default(),
        Value::String(s) => s
            .lines()
            .filter_map(|l| {
                let i = l.find(':')?;
                Some(KV::new(l[..i].trim(), l[i + 1..].trim()))
            })
            .collect(),
        _ => vec![],
    })
}

impl Serialize for Request {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RequestRepr {
            method: self.method.clone(),
            header: self.header.clone(),
            body: self.body.clone(),
            url: self.url.clone(),
            auth: self.auth.clone(),
            description: self.description.clone(),
            extra: self.extra.clone(),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if let Value::String(s) = v {
            return Ok(Request { url: Url::from_raw(&s), ..Default::default() });
        }
        let r: RequestRepr = serde_json::from_value(v).map_err(serde::de::Error::custom)?;
        Ok(Request {
            method: r.method.to_uppercase(),
            header: r.header,
            body: r.body,
            url: r.url,
            auth: r.auth,
            description: r.description,
            extra: r.extra,
        })
    }
}

// ---------------------------------------------------------------- Events / scripts
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub listen: String,
    #[serde(default)]
    pub script: Script,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub exec: String,
    pub extra: Map<String, Value>,
}

impl Serialize for Script {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = Map::new();
        m.insert("type".into(), Value::String("text/javascript".into()));
        m.insert("exec".into(), Value::Array(self.exec.split('\n').map(|l| Value::String(l.to_string())).collect()));
        for (k, v) in &self.extra {
            if !m.contains_key(k) {
                m.insert(k.clone(), v.clone());
            }
        }
        Value::Object(m).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Script {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let mut m = match v {
            Value::Object(m) => m,
            Value::String(s) => return Ok(Script { exec: s, extra: Map::new() }),
            _ => return Ok(Script::default()),
        };
        let exec = match m.remove("exec") {
            Some(Value::Array(a)) => a.iter().map(value_to_string).collect::<Vec<_>>().join("\n"),
            Some(Value::String(s)) => s,
            _ => String::new(),
        };
        m.remove("type");
        Ok(Script { exec, extra: m })
    }
}

pub fn get_script(events: &[Event], listen: &str) -> String {
    events.iter().filter(|e| e.listen == listen && !e.disabled).map(|e| e.script.exec.clone()).collect::<Vec<_>>().join("\n")
}

pub fn set_script(events: &mut Vec<Event>, listen: &str, code: &str) {
    if let Some(e) = events.iter_mut().find(|e| e.listen == listen) {
        e.script.exec = code.to_string();
    } else if !code.trim().is_empty() {
        events.push(Event { listen: listen.into(), script: Script { exec: code.into(), extra: Map::new() }, ..Default::default() });
    }
    events.retain(|e| !(e.script.exec.trim().is_empty()));
}

// ---------------------------------------------------------------- Variables
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Variable {
    #[serde(default, deserialize_with = "lenient_string")]
    pub key: String,
    #[serde(default)]
    pub value: Value,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient_bool", skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ---------------------------------------------------------------- Item
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Item {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default, skip_serializing_if = "desc_empty")]
    pub description: Option<Description>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Request>,
    /// Saved examples (responses)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response: Vec<Value>,
    /// Some => folder
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<Item>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable: Vec<Variable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(rename = "protocolProfileBehavior", default, skip_serializing_if = "Option::is_none")]
    pub protocol_profile_behavior: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Item {
    pub fn is_folder(&self) -> bool {
        self.item.is_some() || self.request.is_none()
    }
    pub fn new_request(name: &str, req: Request) -> Self {
        Item { id: Some(new_id()), name: name.into(), request: Some(req), ..Default::default() }
    }
    pub fn new_folder(name: &str) -> Self {
        Item { id: Some(new_id()), name: name.into(), item: Some(vec![]), ..Default::default() }
    }
    pub fn ensure_ids(&mut self) {
        if self.id.as_deref().map(|s| s.is_empty()).unwrap_or(true) {
            self.id = Some(new_id());
        }
        if let Some(ch) = &mut self.item {
            for c in ch {
                c.ensure_ids();
            }
        }
    }
    pub fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
}

// ---------------------------------------------------------------- Collection
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Info {
    #[serde(rename = "_postman_id", default)]
    pub postman_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default, skip_serializing_if = "desc_empty")]
    pub description: Option<Description>,
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_schema() -> String {
    SCHEMA_V21.into()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Collection {
    pub info: Info,
    #[serde(default)]
    pub item: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable: Vec<Variable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(rename = "protocolProfileBehavior", default, skip_serializing_if = "Option::is_none")]
    pub protocol_profile_behavior: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Collection {
    pub fn new(name: &str) -> Self {
        Collection { info: Info { postman_id: new_id(), name: name.into(), description: None, schema: SCHEMA_V21.into(), extra: Map::new() }, ..Default::default() }
    }
    pub fn id(&self) -> &str {
        &self.info.postman_id
    }
    pub fn ensure_ids(&mut self) {
        if self.info.postman_id.is_empty() {
            self.info.postman_id = new_id();
        }
        self.info.schema = SCHEMA_V21.into();
        for i in &mut self.item {
            i.ensure_ids();
        }
    }

    /// Find item by path of indices
    pub fn get(&self, path: &[usize]) -> Option<&Item> {
        let mut list = &self.item;
        let mut cur: Option<&Item> = None;
        for &i in path {
            let it = list.get(i)?;
            cur = Some(it);
            match &it.item {
                Some(ch) => list = ch,
                None => list = &EMPTY,
            }
        }
        cur
    }
    pub fn get_mut(&mut self, path: &[usize]) -> Option<&mut Item> {
        let (first, rest) = path.split_first()?;
        let mut cur = self.item.get_mut(*first)?;
        for &i in rest {
            cur = cur.item.as_mut()?.get_mut(i)?;
        }
        Some(cur)
    }
    pub fn children_mut(&mut self, path: &[usize]) -> Option<&mut Vec<Item>> {
        if path.is_empty() {
            return Some(&mut self.item);
        }
        let it = self.get_mut(path)?;
        if it.item.is_none() {
            it.item = Some(vec![]);
        }
        it.item.as_mut()
    }
    pub fn find_path_by_id(&self, id: &str) -> Option<Vec<usize>> {
        fn rec(list: &[Item], id: &str, path: &mut Vec<usize>) -> bool {
            for (i, it) in list.iter().enumerate() {
                path.push(i);
                if it.id() == id {
                    return true;
                }
                if let Some(ch) = &it.item {
                    if rec(ch, id, path) {
                        return true;
                    }
                }
                path.pop();
            }
            false
        }
        let mut p = vec![];
        if rec(&self.item, id, &mut p) {
            Some(p)
        } else {
            None
        }
    }
    /// Remove item at path and return it
    pub fn remove(&mut self, path: &[usize]) -> Option<Item> {
        let (last, parent) = path.split_last()?;
        let list = self.children_mut(parent)?;
        if *last < list.len() {
            Some(list.remove(*last))
        } else {
            None
        }
    }
    /// Chain of items from root to the item at `path` (inclusive)
    pub fn chain(&self, path: &[usize]) -> Vec<&Item> {
        let mut out = vec![];
        for n in 1..=path.len() {
            if let Some(i) = self.get(&path[..n]) {
                out.push(i);
            }
        }
        out
    }
    /// Effective auth for item at `path` following the inherit chain.
    pub fn effective_auth(&self, path: &[usize], own: Option<&Auth>) -> Option<Auth> {
        if let Some(a) = own {
            if a.kind != "inherit" {
                return Some(a.clone());
            }
        }
        let chain = self.chain(path);
        // walk parents (excluding the item itself)
        for it in chain.iter().rev().skip(1) {
            if let Some(a) = &it.auth {
                if a.kind != "inherit" {
                    return Some(a.clone());
                }
            }
        }
        self.auth.clone().filter(|a| a.kind != "inherit")
    }
    /// Pre-request / test scripts from collection, folders, request (in execution order)
    pub fn scripts(&self, path: &[usize], listen: &str) -> Vec<(String, String)> {
        let mut out = vec![];
        let s = get_script(&self.event, listen);
        if !s.trim().is_empty() {
            out.push((format!("{} (collection)", self.info.name), s));
        }
        for it in self.chain(path) {
            let s = get_script(&it.event, listen);
            if !s.trim().is_empty() {
                out.push((it.name.clone(), s));
            }
        }
        out
    }
    /// Merged protocolProfileBehavior
    pub fn behavior(&self, path: &[usize]) -> Map<String, Value> {
        let mut m = self.protocol_profile_behavior.clone().unwrap_or_default();
        for it in self.chain(path) {
            if let Some(b) = &it.protocol_profile_behavior {
                for (k, v) in b {
                    m.insert(k.clone(), v.clone());
                }
            }
        }
        m
    }
    /// Flatten all request items: (path, item)
    pub fn requests(&self) -> Vec<(Vec<usize>, &Item)> {
        fn rec<'a>(list: &'a [Item], pre: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, &'a Item)>) {
            for (i, it) in list.iter().enumerate() {
                pre.push(i);
                if let Some(ch) = &it.item {
                    rec(ch, pre, out);
                } else if it.request.is_some() {
                    out.push((pre.clone(), it));
                }
                pre.pop();
            }
        }
        let mut out = vec![];
        rec(&self.item, &mut vec![], &mut out);
        out
    }
}

static EMPTY: Vec<Item> = Vec::new();

/// Parse a collection JSON (v2.0 / v2.1, also v1 best-effort)
pub fn parse_collection(text: &str) -> Result<Collection, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;
    // Postman API wraps in {"collection": {...}}
    let v = match v {
        Value::Object(ref m) if m.contains_key("collection") && !m.contains_key("info") => m["collection"].clone(),
        v => v,
    };
    if v.get("info").is_some() {
        let mut c: Collection = serde_json::from_value(v).map_err(|e| format!("Invalid collection: {e}"))?;
        c.ensure_ids();
        return Ok(c);
    }
    if v.get("requests").is_some() {
        return crate::importer::convert_v1(&v);
    }
    Err("Not a Postman collection".into())
}

// ---------------------------------------------------------------- Environments
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EnvVar {
    #[serde(default, deserialize_with = "lenient_string")]
    pub key: String,
    #[serde(default)]
    pub value: Value,
    #[serde(rename = "type", default = "default_var_type")]
    pub kind: String,
    #[serde(default = "default_true", deserialize_with = "lenient_bool")]
    pub enabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_var_type() -> String {
    "default".into()
}
fn default_true() -> bool {
    true
}

impl EnvVar {
    pub fn new(k: &str, v: &str) -> Self {
        EnvVar { key: k.into(), value: Value::String(v.into()), kind: "default".into(), enabled: true, extra: Map::new() }
    }
    pub fn is_secret(&self) -> bool {
        self.kind == "secret"
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Environment {
    #[serde(default)]
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default)]
    pub values: Vec<EnvVar>,
    #[serde(rename = "_postman_variable_scope", default = "default_env_scope")]
    pub scope: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_env_scope() -> String {
    "environment".into()
}

impl Environment {
    pub fn new(name: &str) -> Self {
        Environment { id: new_id(), name: name.into(), values: vec![], scope: "environment".into(), extra: Map::new() }
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;
        let v = match v {
            Value::Object(ref m) if m.contains_key("environment") && !m.contains_key("values") => m["environment"].clone(),
            v => v,
        };
        if v.get("values").is_none() {
            return Err("Not a Postman environment (missing 'values')".into());
        }
        let mut e: Environment = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if e.id.is_empty() {
            e.id = new_id();
        }
        Ok(e)
    }
}
