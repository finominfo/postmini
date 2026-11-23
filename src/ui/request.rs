use egui::{Color32, RichText, Ui};
use serde_json::Value;

use super::widgets::{self, code_editor, kv_table, method_color, var_edit, KvOpts, Lang};
use super::{Action, App, ReqSection, TabKind};
use crate::model::{get_script, set_script, Auth, Body, GraphQl, AUTH_TYPES};
use crate::vars::VarContext;

pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "COPY", "LINK", "UNLINK", "PURGE", "LOCK", "UNLOCK", "PROPFIND", "VIEW"];

pub const SNIPPETS_PRE: &[(&str, &str)] = &[
    ("Get an environment variable", "pm.environment.get(\"variable_key\");"),
    ("Set an environment variable", "pm.environment.set(\"variable_key\", \"variable_value\");"),
    ("Get a global variable", "pm.globals.get(\"variable_key\");"),
    ("Set a global variable", "pm.globals.set(\"variable_key\", \"variable_value\");"),
    ("Get a collection variable", "pm.collectionVariables.get(\"variable_key\");"),
    ("Set a collection variable", "pm.collectionVariables.set(\"variable_key\", \"variable_value\");"),
    ("Get a variable", "pm.variables.get(\"variable_key\");"),
    ("Clear an environment variable", "pm.environment.unset(\"variable_key\");"),
    ("Send a request", "pm.sendRequest(\"https://postman-echo.com/get\", function (err, response) {\n    console.log(response.json());\n});"),
    ("Add a header", "pm.request.headers.add({ key: \"X-Header\", value: \"value\" });"),
    ("Generate HMAC signature", "const sig = CryptoJS.HmacSHA256(pm.request.body.toString(), pm.environment.get(\"secret\")).toString();\npm.request.headers.upsert({ key: \"X-Signature\", value: sig });"),
    ("Set timestamp variable", "pm.variables.set(\"ts\", Date.now());"),
];

pub const SNIPPETS_TEST: &[(&str, &str)] = &[
    ("Status code: Code is 200", "pm.test(\"Status code is 200\", function () {\n    pm.response.to.have.status(200);\n});"),
    ("Response body: Contains string", "pm.test(\"Body matches string\", function () {\n    pm.expect(pm.response.text()).to.include(\"string_you_want_to_search\");\n});"),
    ("Response body: JSON value check", "pm.test(\"Your test name\", function () {\n    var jsonData = pm.response.json();\n    pm.expect(jsonData.value).to.eql(100);\n});"),
    ("Response body: Is equal to a string", "pm.test(\"Body is correct\", function () {\n    pm.response.to.have.body(\"response_body_string\");\n});"),
    ("Response headers: Content-Type header check", "pm.test(\"Content-Type is present\", function () {\n    pm.response.to.have.header(\"Content-Type\");\n});"),
    ("Response time is less than 200ms", "pm.test(\"Response time is less than 200ms\", function () {\n    pm.expect(pm.response.responseTime).to.be.below(200);\n});"),
    ("Status code: Successful POST request", "pm.test(\"Successful POST request\", function () {\n    pm.expect(pm.response.code).to.be.oneOf([201, 202]);\n});"),
    ("Status code: Code name has string", "pm.test(\"Status code name has string\", function () {\n    pm.response.to.have.status(\"Created\");\n});"),
    ("Response body: Convert XML body to JSON", "var jsonObject = xml2Json(responseBody);"),
    ("Use Tiny Validator for JSON data", "var schema = {\n    \"items\": {\n        \"type\": \"boolean\"\n    }\n};\n\nvar data1 = [true, false];\nvar data2 = [true, 123];\n\npm.test('Schema is valid', function () {\n    pm.expect(tv4.validate(data1, schema)).to.be.true;\n    pm.expect(tv4.validate(data2, schema)).to.be.false;\n});"),
    ("Validate JSON schema", "const schema = {\n    type: \"object\",\n    required: [\"id\"],\n    properties: { id: { type: \"number\" } }\n};\npm.test(\"Schema is valid\", function () {\n    pm.response.to.have.jsonSchema(schema);\n});"),
    ("Set a variable from the response", "const jsonData = pm.response.json();\npm.environment.set(\"token\", jsonData.token);"),
    ("Visualize response", "var template = `<table>{{#each response}}<tr><td>{{name}}</td></tr>{{/each}}</table>`;\npm.visualizer.set(template, { response: pm.response.json() });"),
    ("Set next request", "pm.execution.setNextRequest(\"request_name\");"),
];

fn auth_field(ui: &mut Ui, a: &mut Auth, key: &str, label: &str, vars: &VarContext, dark: bool, secret: bool) -> bool {
    let mut v = a.get(key);
    let mut changed = false;
    ui.label(label);
    if secret {
        let id = ui.id().with(("show", key));
        let mut show = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
        ui.horizontal(|ui| {
            if show {
                if var_edit(ui, &mut v, vars, "", 340.0, dark).changed() {
                    changed = true;
                }
            } else if ui.add_sized([340.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut v).password(true)).changed() {
                changed = true;
            }
            if ui.small_button(if show { "🙈" } else { "👁" }).clicked() {
                show = !show;
                ui.data_mut(|d| d.insert_temp(id, show));
            }
        });
    } else if var_edit(ui, &mut v, vars, "", 360.0, dark).changed() {
        changed = true;
    }
    ui.end_row();
    if changed {
        a.set_str(key, &v);
    }
    changed
}

fn auth_bool(ui: &mut Ui, a: &mut Auth, key: &str, label: &str) -> bool {
    let mut b = a.get_bool(key);
    ui.label("");
    let c = ui.checkbox(&mut b, label).changed();
    ui.end_row();
    if c {
        a.set(key, Value::Bool(b));
    }
    c
}

fn auth_combo(ui: &mut Ui, a: &mut Auth, key: &str, label: &str, opts: &[(&str, &str)], def: &str) -> bool {
    let cur = a.get_or(key, def);
    let mut sel = if cur.is_empty() { def.to_string() } else { cur };
    let txt = opts.iter().find(|o| o.0 == sel).map(|o| o.1).unwrap_or(&sel).to_string();
    ui.label(label);
    let mut changed = false;
    egui::ComboBox::from_id_salt(("ac", key)).selected_text(txt).width(240.0).show_ui(ui, |ui| {
        for (v, l) in opts {
            if ui.selectable_value(&mut sel, v.to_string(), *l).clicked() {
                changed = true;
            }
        }
    });
    ui.end_row();
    if changed {
        a.set_str(key, &sel);
    }
    changed
}

/// Auth editor shared by request, folder and collection tabs. Returns true if changed.
pub fn auth_editor(ui: &mut Ui, auth: &mut Option<Auth>, vars: &VarContext, dark: bool, allow_inherit: bool, inherited: Option<(String, String)>, tab_id: Option<u64>, oauth_msg: &str, busy: bool, actions: &mut Vec<Action>) -> bool {
    let mut changed = false;
    let cur_kind = auth.as_ref().map(|a| a.kind.clone()).unwrap_or_else(|| if allow_inherit { "inherit".into() } else { "noauth".into() });
    let mut sel = cur_kind.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Auth Type").strong());
        egui::ComboBox::from_id_salt("authtype").width(220.0).selected_text(AUTH_TYPES.iter().find(|t| t.0 == sel).map(|t| t.1).unwrap_or(&sel).to_string()).show_ui(ui, |ui| {
            for (k, l) in AUTH_TYPES {
                if *k == "inherit" && !allow_inherit {
                    continue;
                }
                ui.selectable_value(&mut sel, k.to_string(), *l);
            }
        });
    });
    if sel != cur_kind {
        let a = auth.get_or_insert_with(|| Auth::new(&cur_kind));
        a.switch_to(&sel);
        changed = true;
    }
    ui.separator();
    let Some(a) = auth.as_mut() else {
        if allow_inherit {
            inherit_note(ui, &inherited);
        } else {
            ui.label(RichText::new("This collection does not use any authorization.").weak());
        }
        return changed;
    };
    let kind = a.kind.clone();
    egui::Grid::new("authgrid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        match kind.as_str() {
            "inherit" => {
                ui.label("");
                ui.end_row();
            }
            "noauth" => {
                ui.label(RichText::new("This request does not use any authorization.").weak());
                ui.end_row();
            }
            "basic" => {
                changed |= auth_field(ui, a, "username", "Username", vars, dark, false);
                changed |= auth_field(ui, a, "password", "Password", vars, dark, true);
            }
            "bearer" => {
                changed |= auth_field(ui, a, "token", "Token", vars, dark, true);
            }
            "apikey" => {
                changed |= auth_field(ui, a, "key", "Key", vars, dark, false);
                changed |= auth_field(ui, a, "value", "Value", vars, dark, true);
                changed |= auth_combo(ui, a, "in", "Add to", &[("header", "Header"), ("query", "Query Params")], "header");
            }
            "jwt" => {
                changed |= auth_combo(ui, a, "addTokenTo", "Add JWT token to", &[("header", "Request Header"), ("queryParam", "Query Param")], "header");
                changed |= auth_combo(ui, a, "algorithm", "Algorithm", &[("HS256", "HS256"), ("HS384", "HS384"), ("HS512", "HS512")], "HS256");
                changed |= auth_field(ui, a, "secret", "Secret", vars, dark, true);
                changed |= auth_bool(ui, a, "isSecretBase64Encoded", "Secret Base64 encoded");
                ui.label("Payload (JSON)");
                let mut p = a.get_or("payload", "{}");
                if code_editor(ui, &mut p, Lang::Json, 4, dark, "jwtpayload").changed() {
                    a.set_str("payload", &p);
                    changed = true;
                }
                ui.end_row();
                changed |= auth_field(ui, a, "headerPrefix", "Request header prefix", vars, dark, false);
                changed |= auth_field(ui, a, "queryParamKey", "Query param name", vars, dark, false);
                changed |= auth_field(ui, a, "header", "JWT headers (JSON)", vars, dark, false);
            }
            "digest" => {
                changed |= auth_field(ui, a, "username", "Username", vars, dark, false);
                changed |= auth_field(ui, a, "password", "Password", vars, dark, true);
                changed |= auth_bool(ui, a, "disableRetryRequest", "Disable retrying the request (send only once)");
                ui.label(RichText::new("Advanced (auto-filled from the server challenge)").small().weak());
                ui.end_row();
                changed |= auth_field(ui, a, "realm", "Realm", vars, dark, false);
                changed |= auth_field(ui, a, "nonce", "Nonce", vars, dark, false);
                changed |= auth_combo(ui, a, "algorithm", "Algorithm", &[("MD5", "MD5"), ("MD5-sess", "MD5-sess"), ("SHA-256", "SHA-256"), ("SHA-256-sess", "SHA-256-sess"), ("SHA-512-256", "SHA-512-256")], "MD5");
                changed |= auth_field(ui, a, "qop", "qop", vars, dark, false);
                changed |= auth_field(ui, a, "nonceCount", "Nonce Count", vars, dark, false);
                changed |= auth_field(ui, a, "clientNonce", "Client Nonce", vars, dark, false);
                changed |= auth_field(ui, a, "opaque", "Opaque", vars, dark, false);
            }
            "oauth1" => {
                changed |= auth_combo(ui, a, "signatureMethod", "Signature Method", &[("HMAC-SHA1", "HMAC-SHA1"), ("HMAC-SHA256", "HMAC-SHA256"), ("HMAC-SHA512", "HMAC-SHA512"), ("PLAINTEXT", "PLAINTEXT")], "HMAC-SHA1");
                changed |= auth_field(ui, a, "consumerKey", "Consumer Key", vars, dark, false);
                changed |= auth_field(ui, a, "consumerSecret", "Consumer Secret", vars, dark, true);
                changed |= auth_field(ui, a, "token", "Access Token", vars, dark, false);
                changed |= auth_field(ui, a, "tokenSecret", "Token Secret", vars, dark, true);
                changed |= auth_combo(ui, a, "addParamsToHeader", "Add auth data to", &[("true", "Request Headers"), ("false", "Request Body / URL")], "true");
                changed |= auth_field(ui, a, "callback", "Callback URL", vars, dark, false);
                changed |= auth_field(ui, a, "verifier", "Verifier", vars, dark, false);
                changed |= auth_field(ui, a, "timestamp", "Timestamp", vars, dark, false);
                changed |= auth_field(ui, a, "nonce", "Nonce", vars, dark, false);
                changed |= auth_field(ui, a, "version", "Version", vars, dark, false);
                changed |= auth_field(ui, a, "realm", "Realm", vars, dark, false);
                changed |= auth_bool(ui, a, "includeBodyHash", "Include body hash");
                changed |= auth_bool(ui, a, "addEmptyParamsToSign", "Add empty parameters to signature");
            }
            "oauth2" => {
                ui.label(RichText::new("Current Token").strong());
                ui.end_row();
                changed |= auth_field(ui, a, "accessToken", "Access Token", vars, dark, true);
                changed |= auth_field(ui, a, "headerPrefix", "Header Prefix", vars, dark, false);
                changed |= auth_combo(ui, a, "addTokenTo", "Add token to", &[("header", "Request Headers"), ("queryParams", "Request URL")], "header");
                ui.separator();
                ui.end_row();
                ui.label(RichText::new("Configure New Token").strong());
                ui.end_row();
                changed |= auth_combo(ui, a, "grant_type", "Grant Type", &[("authorization_code", "Authorization Code"), ("authorization_code_with_pkce", "Authorization Code (With PKCE)"), ("implicit", "Implicit"), ("password_credentials", "Password Credentials"), ("client_credentials", "Client Credentials")], "authorization_code");
                let g = a.get_or("grant_type", "authorization_code");
                if g.starts_with("authorization_code") || g == "implicit" {
                    changed |= auth_field(ui, a, "redirect_uri", "Callback URL (loopback)", vars, dark, false);
                    changed |= auth_field(ui, a, "authUrl", "Auth URL", vars, dark, false);
                }
                if g != "implicit" {
                    changed |= auth_field(ui, a, "accessTokenUrl", "Access Token URL", vars, dark, false);
                }
                changed |= auth_field(ui, a, "clientId", "Client ID", vars, dark, false);
                if g != "implicit" {
                    changed |= auth_field(ui, a, "clientSecret", "Client Secret", vars, dark, true);
                }
                if g == "password_credentials" {
                    changed |= auth_field(ui, a, "username", "Username", vars, dark, false);
                    changed |= auth_field(ui, a, "password", "Password", vars, dark, true);
                }
                if g == "authorization_code_with_pkce" {
                    changed |= auth_combo(ui, a, "challengeAlgorithm", "Code Challenge Method", &[("S256", "SHA-256"), ("plain", "Plain")], "S256");
                    changed |= auth_field(ui, a, "code_verifier", "Code Verifier (optional)", vars, dark, false);
                }
                changed |= auth_field(ui, a, "scope", "Scope", vars, dark, false);
                changed |= auth_field(ui, a, "state", "State", vars, dark, false);
                changed |= auth_combo(ui, a, "client_authentication", "Client Authentication", &[("header", "Send as Basic Auth header"), ("body", "Send client credentials in body")], "header");
                changed |= auth_field(ui, a, "audience", "Audience", vars, dark, false);
                changed |= auth_field(ui, a, "resource", "Resource", vars, dark, false);
                ui.label("");
                ui.horizontal(|ui| {
                    if let Some(id) = tab_id {
                        if ui.add_enabled(!busy, egui::Button::new(RichText::new("Get New Access Token").strong())).clicked() {
                            actions.push(Action::GetOAuthToken(id, None));
                        }
                        let rt = a.get("refreshToken");
                        if !rt.is_empty() && ui.add_enabled(!busy, egui::Button::new("Refresh token")).clicked() {
                            actions.push(Action::GetOAuthToken(id, Some(rt)));
                        }
                        if busy {
                            ui.spinner();
                        }
                    } else {
                        ui.label(RichText::new("Open a request that inherits this auth to fetch a token.").small().weak());
                    }
                });
                ui.end_row();
                if !oauth_msg.is_empty() {
                    ui.label("");
                    ui.label(RichText::new(oauth_msg).color(if oauth_msg.starts_with("Error") { Color32::from_rgb(0xec, 0x5b, 0x56) } else { Color32::from_rgb(0x3c, 0xc4, 0x7c) }));
                    ui.end_row();
                }
            }
            "hawk" => {
                changed |= auth_field(ui, a, "authId", "Hawk Auth ID", vars, dark, false);
                changed |= auth_field(ui, a, "authKey", "Hawk Auth Key", vars, dark, true);
                changed |= auth_combo(ui, a, "algorithm", "Algorithm", &[("sha256", "SHA256"), ("sha1", "SHA1")], "sha256");
                changed |= auth_field(ui, a, "user", "User", vars, dark, false);
                changed |= auth_field(ui, a, "nonce", "Nonce", vars, dark, false);
                changed |= auth_field(ui, a, "extraData", "ext", vars, dark, false);
                changed |= auth_field(ui, a, "app", "app", vars, dark, false);
                changed |= auth_field(ui, a, "delegation", "dlg", vars, dark, false);
                changed |= auth_field(ui, a, "timestamp", "Timestamp", vars, dark, false);
                changed |= auth_bool(ui, a, "includePayloadHash", "Include payload hash");
            }
            "awsv4" => {
                changed |= auth_field(ui, a, "accessKey", "AccessKey", vars, dark, false);
                changed |= auth_field(ui, a, "secretKey", "SecretKey", vars, dark, true);
                changed |= auth_field(ui, a, "region", "AWS Region", vars, dark, false);
                changed |= auth_field(ui, a, "service", "Service Name", vars, dark, false);
                changed |= auth_field(ui, a, "sessionToken", "Session Token", vars, dark, true);
                changed |= auth_bool(ui, a, "addAuthDataToQuery", "Add auth data to query params (presigned URL)");
            }
            "edgegrid" => {
                changed |= auth_field(ui, a, "accessToken", "Access Token", vars, dark, false);
                changed |= auth_field(ui, a, "clientToken", "Client Token", vars, dark, false);
                changed |= auth_field(ui, a, "clientSecret", "Client Secret", vars, dark, true);
                changed |= auth_field(ui, a, "nonce", "Nonce", vars, dark, false);
                changed |= auth_field(ui, a, "timestamp", "Timestamp", vars, dark, false);
                changed |= auth_field(ui, a, "baseURL", "Base URL", vars, dark, false);
                changed |= auth_field(ui, a, "headersToSign", "Headers to Sign", vars, dark, false);
            }
            "ntlm" | "asap" => {
                ui.colored_label(Color32::from_rgb(0xf0, 0xb0, 0x3e), "This auth type is kept for compatibility but cannot be executed by postmini.");
                ui.end_row();
                for (k, _) in a.params.clone() {
                    changed |= auth_field(ui, a, &k, &k, vars, dark, false);
                }
            }
            _ => {}
        }
    });
    if kind == "inherit" {
        inherit_note(ui, &inherited);
    }
    changed
}

fn inherit_note(ui: &mut Ui, inherited: &Option<(String, String)>) {
    match inherited {
        Some((src, ty)) => {
            ui.label(format!("This request is using {} from {}.", AUTH_TYPES.iter().find(|t| t.0 == ty).map(|t| t.1).unwrap_or(ty), src));
        }
        None => {
            ui.label(RichText::new("The authorization header will be inherited from the parent folder/collection (none configured).").weak());
        }
    }
}

pub fn script_editor(ui: &mut Ui, code: &mut String, snippets: &[(&str, &str)], dark: bool, id: &str) -> bool {
    let mut changed = false;
    ui.horizontal_top(|ui| {
        let w = ui.available_width();
        ui.vertical(|ui| {
            ui.set_width((w - 230.0).max(200.0));
            egui::ScrollArea::vertical().id_salt((id, "sc")).auto_shrink([false, false]).max_height(ui.available_height()).show(ui, |ui| {
                if code_editor(ui, code, Lang::Js, 18, dark, id).changed() {
                    changed = true;
                }
            });
        });
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.label(RichText::new("Snippets").strong());
            egui::ScrollArea::vertical().id_salt((id, "snip")).show(ui, |ui| {
                for (name, s) in snippets {
                    if ui.link(RichText::new(*name).small()).on_hover_text(*s).clicked() {
                        if !code.is_empty() && !code.ends_with('\n') {
                            code.push('\n');
                        }
                        code.push_str(s);
                        code.push('\n');
                        changed = true;
                    }
                }
            });
        });
    });
    changed
}

const BEHAVIORS: &[(&str, &str, &str, bool)] = &[
    ("strictSSL", "Enable SSL certificate verification", "Verify SSL certificates when sending a request (default: from Settings).", true),
    ("followRedirects", "Automatically follow redirects", "Follow HTTP 3xx responses as redirects.", true),
    ("followOriginalHttpMethod", "Follow original HTTP Method", "Redirect with the original HTTP method instead of the default behavior of redirecting with GET.", false),
    ("followAuthorizationHeader", "Follow Authorization header", "Retain authorization header when a redirect happens to a different hostname.", false),
    ("removeRefererHeaderOnRedirect", "Remove referer header on redirect", "Remove the referer header when a redirect happens.", false),
    ("disableBodyPruning", "Enable body for GET/HEAD", "Send the request body for GET, HEAD, COPY, PURGE and UNLOCK requests.", false),
    ("disableUrlEncoding", "Disable URL encoding", "Do not percent-encode the URL path and query.", false),
    ("disableCookies", "Disable cookie jar", "Existing cookies in the jar are not sent and cookies from the response are not stored.", false),
];

pub fn behavior_editor(ui: &mut Ui, ppb: &mut Option<serde_json::Map<String, Value>>) -> bool {
    let mut changed = false;
    let m = ppb.get_or_insert_with(serde_json::Map::new);
    egui::Grid::new("behgrid").num_columns(2).spacing([16.0, 8.0]).striped(true).show(ui, |ui| {
        for (k, title, desc, def) in BEHAVIORS {
            ui.vertical(|ui| {
                ui.label(RichText::new(*title).strong());
                ui.label(RichText::new(*desc).small().weak());
            });
            let mut v = m.get(*k).and_then(|v| v.as_bool()).unwrap_or(*def);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut v, "").changed() {
                    m.insert(k.to_string(), Value::Bool(v));
                    changed = true;
                }
                if m.contains_key(*k) && ui.small_button("Default").clicked() {
                    m.remove(*k);
                    changed = true;
                }
            });
            ui.end_row();
        }
        ui.label(RichText::new("Maximum number of redirects").strong());
        let mut mr = m.get("maxRedirects").and_then(|v| v.as_u64()).unwrap_or(10);
        if ui.add(egui::DragValue::new(&mut mr).range(0..=100)).changed() {
            m.insert("maxRedirects".into(), Value::from(mr));
            changed = true;
        }
        ui.end_row();
    });
    if m.is_empty() {
        *ppb = None;
    }
    changed
}

pub fn show(app: &mut App, idx: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    let tab_id = app.tabs[idx].id;
    let origin = match &app.tabs[idx].kind {
        TabKind::Request(r) => r.origin.clone(),
        _ => return,
    };
    let coll_id = origin.as_ref().map(|o| o.0.clone());
    let vars = app.var_ctx(coll_id.as_deref());
    let dark = app.settings.dark;
    // breadcrumb + inherited auth info
    let mut crumbs: Vec<String> = vec![];
    let mut inherited: Option<(String, String)> = None;
    if let Some((cid, iid)) = &origin {
        if let Some(c) = app.coll(cid) {
            crumbs.push(c.info.name.clone());
            if let Some(p) = c.find_path_by_id(iid) {
                let chain = c.chain(&p);
                for it in chain.iter().take(chain.len().saturating_sub(1)) {
                    crumbs.push(it.name.clone());
                }
                for it in chain.iter().rev().skip(1) {
                    if let Some(a) = &it.auth {
                        if a.kind != "inherit" {
                            inherited = Some((format!("folder \"{}\"", it.name), a.kind.clone()));
                            break;
                        }
                    }
                }
            }
            if inherited.is_none() {
                if let Some(a) = c.auth.as_ref().filter(|a| a.kind != "inherit") {
                    inherited = Some((format!("collection \"{}\"", c.info.name), a.kind.clone()));
                }
            }
        }
    }
    let TabKind::Request(tab) = &mut app.tabs[idx].kind else { return };

    // ---------------------------------------------------------------- header line
    ui.horizontal(|ui| {
        for c in &crumbs {
            ui.label(RichText::new(c).weak());
            ui.label(RichText::new("/").weak());
        }
        let mut name = tab.item.name.clone();
        if ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Body)).changed() {
            tab.item.name = name;
            tab.dirty = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("⏷", |ui| {
                if ui.button("Save As…   Ctrl+Shift+S").clicked() {
                    actions.push(Action::SaveAs(tab_id));
                    ui.close();
                }
                if ui.button("Duplicate tab").clicked() {
                    actions.push(Action::DuplicateTab(tab_id));
                    ui.close();
                }
            });
            let save_txt = if tab.dirty || tab.origin.is_none() { RichText::new("💾 Save").strong() } else { RichText::new("💾 Save") };
            if ui.button(save_txt).on_hover_text("Ctrl+S").clicked() {
                actions.push(Action::Save(tab_id));
            }
            if ui.button("</> Code").on_hover_text("Generate code snippet").clicked() {
                actions.push(Action::OpenCode(tab_id));
            }
            if !tab.item.response.is_empty() {
                ui.menu_button(format!("Examples ({})", tab.item.response.len()), |ui| {
                    let mut del = None;
                    for (i, ex) in tab.item.response.iter().enumerate() {
                        let name = ex.get("name").map(crate::model::value_to_string).unwrap_or_default();
                        ui.horizontal(|ui| {
                            if ui.button(&name).clicked() {
                                let code = ex.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
                                let body = ex.get("body").map(crate::model::value_to_string).unwrap_or_default();
                                let body = widgets::pretty_json(&body).unwrap_or(body);
                                actions.push(Action::Dialog(super::dialogs::Dialog::Text { title: format!("Example: {name} ({code})"), text: body }));
                                ui.close();
                            }
                            if ui.small_button("🗑").clicked() {
                                del = Some(i);
                            }
                        });
                    }
                    if let Some(i) = del {
                        tab.item.response.remove(i);
                        tab.dirty = true;
                    }
                });
            }
        });
    });

    // ---------------------------------------------------------------- URL bar
    let inflight = tab.inflight.is_some();
    ui.horizontal(|ui| {
        let req = tab.item.request.get_or_insert_with(Default::default);
        let mut m = req.method.clone();
        egui::ComboBox::from_id_salt("method").width(92.0).selected_text(RichText::new(&m).strong().color(method_color(&m))).show_ui(ui, |ui| {
            for x in METHODS {
                ui.selectable_value(&mut m, x.to_string(), RichText::new(*x).color(method_color(x)));
            }
            ui.separator();
            let id = ui.id().with("custom_m");
            let mut custom: String = ui.data(|d| d.get_temp(id).unwrap_or_default());
            ui.horizontal(|ui| {
                if ui.add_sized([100.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut custom).hint_text("Custom method")).changed() {
                    ui.data_mut(|d| d.insert_temp(id, custom.clone()));
                }
                if ui.small_button("Use").clicked() && !custom.trim().is_empty() {
                    m = custom.trim().to_uppercase();
                }
            });
        });
        if m != req.method {
            req.method = m;
            tab.dirty = true;
        }
        let w = ui.available_width() - 90.0;
        let mut url = req.url.raw.clone();
        let r = var_edit(ui, &mut url, &vars, "Enter URL or paste cURL", w, dark);
        if r.changed() {
            if crate::importer::is_curl(&url) {
                match crate::importer::parse_curl(&url) {
                    Ok(newreq) => {
                        *req = newreq;
                        actions.push(Action::Toast("Imported cURL command".into()));
                    }
                    Err(e) => {
                        req.url.raw = url;
                        actions.push(Action::Error(e));
                    }
                }
            } else {
                req.url.raw = url;
                req.url.sync_from_raw();
            }
            tab.dirty = true;
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            actions.push(Action::Send(tab_id));
        }
        if inflight {
            if ui.add_sized([80.0, 26.0], egui::Button::new("Cancel")).clicked() {
                actions.push(Action::Cancel(tab_id));
            }
        } else if ui.add_sized([80.0, 26.0], egui::Button::new(RichText::new("Send").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0x09, 0x7b, 0xed))).on_hover_text("Ctrl+Enter").clicked() {
            actions.push(Action::Send(tab_id));
        }
    });
    ui.add_space(2.0);

    // ---------------------------------------------------------------- request config / response split
    let avail = ui.available_height();
    let top_h = (avail * tab.split).clamp(120.0, avail - 120.0);
    let (top_rect, rest) = {
        let r = ui.available_rect_before_wrap();
        let top = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), top_h));
        let bottom = egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y + top_h + 6.0), r.max);
        (top, bottom)
    };
    let mut top_ui = ui.new_child(egui::UiBuilder::new().max_rect(top_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    top_ui.set_clip_rect(top_rect);
    request_config(&mut top_ui, tab, &vars, dark, tab_id, &inherited, actions);
    // splitter
    let split_rect = egui::Rect::from_min_max(egui::pos2(top_rect.min.x, top_rect.max.y), egui::pos2(top_rect.max.x, top_rect.max.y + 6.0));
    let sresp = ui.interact(split_rect, ui.id().with("split"), egui::Sense::drag());
    let col = if sresp.hovered() || sresp.dragged() { Color32::from_rgb(0xff, 0x6c, 0x37) } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
    ui.painter().hline(split_rect.x_range(), split_rect.center().y, egui::Stroke::new(1.0, col));
    if sresp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if sresp.dragged() {
        tab.split = ((top_h + sresp.drag_delta().y) / avail).clamp(0.15, 0.85);
    }
    let mut bottom_ui = ui.new_child(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::top_down(egui::Align::Min)));
    bottom_ui.set_clip_rect(rest);
    super::response::show(&mut bottom_ui, tab, tab_id, dark, actions);
    ui.allocate_rect(egui::Rect::from_min_max(top_rect.min, rest.max), egui::Sense::hover());
}

fn count_label(name: &str, n: usize) -> String {
    if n > 0 {
        format!("{name} ({n})")
    } else {
        name.to_string()
    }
}

fn request_config(ui: &mut Ui, tab: &mut super::ReqTab, vars: &VarContext, dark: bool, tab_id: u64, inherited: &Option<(String, String)>, actions: &mut Vec<Action>) {
    let req = tab.item.request.get_or_insert_with(Default::default);
    let nparams = req.url.query.iter().filter(|q| !q.disabled && !q.key.is_empty()).count();
    let nheaders = req.header.iter().filter(|q| !q.disabled && !q.key.is_empty()).count();
    let has_body = req.body.as_ref().map(|b| !b.mode.is_empty() && b.mode != "none").unwrap_or(false);
    let has_pre = !get_script(&tab.item.event, "prerequest").trim().is_empty();
    let has_test = !get_script(&tab.item.event, "test").trim().is_empty();
    let auth_kind = req.auth.as_ref().map(|a| a.kind.clone()).unwrap_or_else(|| "inherit".into());
    ui.horizontal(|ui| {
        let dot = |s: &str, on: bool| if on { format!("{s} ●") } else { s.to_string() };
        for (sec, label) in [
            (ReqSection::Params, count_label("Params", nparams)),
            (ReqSection::Auth, dot("Authorization", auth_kind != "inherit" && auth_kind != "noauth")),
            (ReqSection::Headers, count_label("Headers", nheaders)),
            (ReqSection::Body, dot("Body", has_body)),
            (ReqSection::PreRequest, dot("Pre-request Script", has_pre)),
            (ReqSection::Tests, dot("Tests", has_test)),
            (ReqSection::Settings, "Settings".to_string()),
            (ReqSection::Docs, "Docs".to_string()),
        ] {
            if ui.selectable_label(tab.section == sec, label).clicked() {
                tab.section = sec;
            }
        }
    });
    ui.separator();
    let section = tab.section;
    egui::ScrollArea::vertical().id_salt("reqcfg").auto_shrink([false, false]).show(ui, |ui| {
        let mut dirty = false;
        match section {
            ReqSection::Params => {
                let req = tab.item.request.get_or_insert_with(Default::default);
                ui.label(RichText::new("Query Params").strong());
                if kv_table(ui, "query", &mut req.url.query, &KvOpts { description: true, files: false, key_hint: "Key", value_hint: "Value" }, &mut tab.bulk, &mut tab.bulk_text, vars, dark) {
                    req.url.sync_raw_from_query();
                    dirty = true;
                }
                if !req.url.variable.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Path Variables").strong());
                    egui::Grid::new("pathvars").num_columns(3).striped(true).show(ui, |ui| {
                        for v in req.url.variable.iter_mut() {
                            ui.label(RichText::new(&v.key).monospace());
                            if var_edit(ui, &mut v.value, vars, "Value", 260.0, dark).changed() {
                                dirty = true;
                            }
                            let mut d = v.desc();
                            if ui.add_sized([200.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut d).hint_text("Description")).changed() {
                                v.description = if d.is_empty() { None } else { Some(crate::model::Description(d)) };
                                dirty = true;
                            }
                            ui.end_row();
                        }
                    });
                }
            }
            ReqSection::Auth => {
                let busy = tab.oauth_busy.is_some();
                let msg = tab.oauth_msg.clone();
                let req = tab.item.request.get_or_insert_with(Default::default);
                if auth_editor(ui, &mut req.auth, vars, dark, true, inherited.clone(), Some(tab_id), &msg, busy, actions) {
                    dirty = true;
                }
            }
            ReqSection::Headers => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Headers").strong());
                    let lbl = if tab.show_auto_headers { "Hide auto-generated headers" } else { "Show auto-generated headers" };
                    if ui.small_button(lbl).clicked() {
                        tab.show_auto_headers = !tab.show_auto_headers;
                    }
                });
                if tab.show_auto_headers {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        for (k, v) in [("Host", "<calculated when request is sent>"), ("User-Agent", "postmini/x (configurable in Settings)"), ("Accept", "*/*"), ("Accept-Encoding", "gzip, deflate, br"), ("Cache-Control", "no-cache (configurable)"), ("Postman-Token", "<random uuid> (configurable)"), ("Content-Type", "<from body mode>"), ("Content-Length", "<calculated when request is sent>"), ("Connection", "close"), ("Authorization", "<from Authorization tab>"), ("Cookie", "<from cookie jar>")] {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(k).monospace().weak());
                                ui.label(RichText::new(v).small().weak());
                            });
                        }
                    });
                }
                let req = tab.item.request.get_or_insert_with(Default::default);
                if kv_table(ui, "headers", &mut req.header, &KvOpts { description: true, files: false, key_hint: "Key", value_hint: "Value" }, &mut tab.bulk, &mut tab.bulk_text, vars, dark) {
                    dirty = true;
                }
            }
            ReqSection::Body => {
                let req = tab.item.request.get_or_insert_with(Default::default);
                if body_editor(ui, &mut req.body, &mut tab.bulk, &mut tab.bulk_text, vars, dark, &mut req.header) {
                    dirty = true;
                }
            }
            ReqSection::PreRequest => {
                ui.label(RichText::new("Pre-request scripts run before the request is sent (collection → folder → request).").small().weak());
                let mut code = get_script(&tab.item.event, "prerequest");
                if script_editor(ui, &mut code, SNIPPETS_PRE, dark, "pre") {
                    set_script(&mut tab.item.event, "prerequest", &code);
                    dirty = true;
                }
            }
            ReqSection::Tests => {
                ui.label(RichText::new("Test scripts run after the response is received. Use pm.test() and pm.expect().").small().weak());
                let mut code = get_script(&tab.item.event, "test");
                if script_editor(ui, &mut code, SNIPPETS_TEST, dark, "test") {
                    set_script(&mut tab.item.event, "test", &code);
                    dirty = true;
                }
            }
            ReqSection::Settings => {
                if behavior_editor(ui, &mut tab.item.protocol_profile_behavior) {
                    dirty = true;
                }
            }
            ReqSection::Docs => {
                let req = tab.item.request.get_or_insert_with(Default::default);
                let mut d = req.description.as_ref().map(|d| d.0.clone()).unwrap_or_default();
                ui.label(RichText::new("Documentation (Markdown)").weak());
                if ui.add(egui::TextEdit::multiline(&mut d).desired_rows(12).desired_width(f32::INFINITY)).changed() {
                    req.description = if d.is_empty() { None } else { Some(crate::model::Description(d)) };
                    dirty = true;
                }
            }
        }
        if dirty {
            tab.dirty = true;
        }
    });
}

pub fn body_editor(ui: &mut Ui, body: &mut Option<Body>, bulk: &mut bool, bulk_text: &mut String, vars: &VarContext, dark: bool, headers: &mut Vec<crate::model::KV>) -> bool {
    let mut changed = false;
    let mut mode = body.as_ref().map(|b| if b.mode.is_empty() { "none".to_string() } else { b.mode.clone() }).unwrap_or_else(|| "none".into());
    let old = mode.clone();
    ui.horizontal(|ui| {
        for (m, l) in [("none", "none"), ("formdata", "form-data"), ("urlencoded", "x-www-form-urlencoded"), ("raw", "raw"), ("file", "binary"), ("graphql", "GraphQL")] {
            ui.radio_value(&mut mode, m.to_string(), l);
        }
        if mode == "raw" {
            let b = body.get_or_insert_with(Body::default);
            let mut lang = b.raw_language();
            egui::ComboBox::from_id_salt("rawlang").width(100.0).selected_text(match lang.as_str() {
                "json" => "JSON",
                "xml" => "XML",
                "html" => "HTML",
                "javascript" => "JavaScript",
                _ => "Text",
            })
            .show_ui(ui, |ui| {
                for (v, l) in [("text", "Text"), ("javascript", "JavaScript"), ("json", "JSON"), ("html", "HTML"), ("xml", "XML")] {
                    ui.selectable_value(&mut lang, v.to_string(), l);
                }
            });
            if lang != b.raw_language() {
                b.set_raw_language(&lang);
                // update Content-Type header if the user did not set one manually
                changed = true;
            }
            if (lang == "json" || lang == "xml" || lang == "html") && ui.small_button("Beautify").clicked() {
                let raw = b.raw.clone().unwrap_or_default();
                let pretty = if lang == "json" { widgets::pretty_json(&raw) } else { Some(widgets::pretty_xml(&raw)) };
                if let Some(p) = pretty {
                    b.raw = Some(p);
                    changed = true;
                }
            }
        }
    });
    if mode != old {
        let b = body.get_or_insert_with(Body::default);
        b.mode = if mode == "none" { String::new() } else { mode.clone() };
        if mode == "graphql" && b.graphql.is_none() {
            b.graphql = Some(GraphQl::default());
        }
        if mode == "raw" && b.raw.is_none() {
            b.raw = Some(String::new());
        }
        if mode == "none" {
            *body = None;
        }
        changed = true;
    }
    let _ = headers;
    let Some(b) = body.as_mut() else {
        ui.add_space(8.0);
        ui.label(RichText::new("This request does not have a body").weak());
        return changed;
    };
    match b.mode.as_str() {
        "raw" => {
            let mut raw = b.raw.clone().unwrap_or_default();
            let lang = match b.raw_language().as_str() {
                "json" => Lang::Json,
                "javascript" => Lang::Js,
                "xml" | "html" => Lang::Xml,
                _ => Lang::Plain,
            };
            if code_editor(ui, &mut raw, lang, 14, dark, "rawbody").changed() {
                b.raw = Some(raw.clone());
                changed = true;
            }
            if lang == Lang::Json && !raw.trim().is_empty() && !raw.contains("{{") {
                if let Err(e) = serde_json::from_str::<Value>(&raw) {
                    ui.label(RichText::new(format!("JSON: {e}")).small().color(Color32::from_rgb(0xf0, 0xb0, 0x3e)));
                }
            }
        }
        "urlencoded" => {
            if kv_table(ui, "urlenc", &mut b.urlencoded, &KvOpts { description: true, files: false, key_hint: "Key", value_hint: "Value" }, bulk, bulk_text, vars, dark) {
                changed = true;
            }
        }
        "formdata" => {
            if kv_table(ui, "formdata", &mut b.formdata, &KvOpts { description: true, files: true, key_hint: "Key", value_hint: "Value" }, bulk, bulk_text, vars, dark) {
                changed = true;
            }
        }
        "file" => {
            ui.horizontal(|ui| {
                let mut p = b.file_src();
                ui.label("File:");
                if ui.add_sized([400.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut p).hint_text("path to file")).changed() {
                    b.file = Some(serde_json::json!({ "src": p }));
                    changed = true;
                }
                if super::files::can_browse() && ui.button("Select File").clicked() {
                    if let Some(f) = super::files::pick_file(&[]) {
                        b.file = Some(serde_json::json!({ "src": f }));
                        changed = true;
                    }
                }
            });
        }
        "graphql" => {
            let g = b.graphql.get_or_insert_with(GraphQl::default);
            ui.columns(2, |cols| {
                cols[0].label(RichText::new("QUERY").small().strong());
                if code_editor(&mut cols[0], &mut g.query, Lang::Plain, 14, dark, "gqlq").changed() {
                    changed = true;
                }
                cols[1].label(RichText::new("GRAPHQL VARIABLES").small().strong());
                if code_editor(&mut cols[1], &mut g.variables, Lang::Json, 14, dark, "gqlv").changed() {
                    changed = true;
                }
            });
        }
        _ => {}
    }
    changed
}
