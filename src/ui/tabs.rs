use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use egui::{Color32, RichText, Ui};
use serde_json::Value;

use super::widgets::{self, kv_table, method_color, KvOpts};
use super::{Action, App, TabKind};
use crate::model::{get_script, set_script, value_to_string, Collection, EnvVar, Variable, KV};
use crate::runner::{ConsoleEntry, ItemResult, RunEvent, RunOptions, RunSummary};
use crate::vars::{Scope, VarContext};

pub const GLOBALS_ID: &str = "__globals__";

pub fn show_tab(app: &mut App, i: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    match &app.tabs[i].kind {
        TabKind::Request(_) => super::request::show(app, i, ui, actions),
        TabKind::Env(_) => env_tab(app, i, ui, actions),
        TabKind::Coll(_) => coll_tab(app, i, ui, actions),
        TabKind::Ws(_) => ws_tab(app, i, ui, actions),
        TabKind::Runner(_) => runner_tab(app, i, ui, actions),
    }
}

/// Compact editable list of variables (quick look)
pub fn quick_vars(ui: &mut Ui, vals: &mut Vec<EnvVar>, id: &str) -> bool {
    let mut changed = false;
    if vals.is_empty() {
        ui.label(RichText::new("No variables").weak());
        return false;
    }
    egui::ScrollArea::vertical().id_salt(id).max_height(220.0).show(ui, |ui| {
        egui::Grid::new(id).num_columns(2).striped(true).show(ui, |ui| {
            for v in vals.iter_mut() {
                ui.label(RichText::new(&v.key).monospace());
                let mut s = value_to_string(&v.value);
                let r = if v.is_secret() { ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s).password(true)) } else { ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s)) };
                if r.changed() {
                    v.value = Value::String(s);
                    changed = true;
                }
                ui.end_row();
            }
        });
    });
    changed
}

// ------------------------------------------------------------------ environment tab

pub struct EnvTab {
    pub env_id: String,
    pub show_secrets: bool,
    pub bulk: bool,
    pub bulk_text: String,
    pub filter: String,
}

impl EnvTab {
    pub fn new(id: &str) -> Self {
        EnvTab { env_id: id.to_string(), show_secrets: false, bulk: false, bulk_text: String::new(), filter: String::new() }
    }
}

fn env_tab(app: &mut App, i: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    let active = app.active_env.clone();
    let App { tabs, envs, globals, .. } = app;
    let TabKind::Env(t) = &mut tabs[i].kind else { return };
    let is_globals = t.env_id == GLOBALS_ID;
    let env = if is_globals {
        Some(globals)
    } else {
        envs.iter_mut().find(|e| e.id == t.env_id)
    };
    let Some(env) = env else {
        ui.label("Environment not found (deleted?)");
        return;
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        if is_globals {
            ui.heading("Globals");
        } else {
            let mut name = env.name.clone();
            if ui.add_sized([300.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Heading)).changed() {
                env.name = name;
                changed = true;
            }
            let is_active = active.as_deref() == Some(&env.id);
            if is_active {
                ui.label(RichText::new("● active").color(Color32::from_rgb(0x3c, 0xc4, 0x7c)));
            } else if ui.button("Set active").clicked() {
                actions.push(Action::SetActiveEnv(Some(env.id.clone())));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Export…").clicked() {
                actions.push(Action::ExportEnv(t.env_id.clone()));
            }
            ui.checkbox(&mut t.show_secrets, "Show secrets");
            ui.add_sized([160.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut t.filter).hint_text("🔍 Filter"));
        });
    });
    ui.label(RichText::new("Variables are saved automatically. Values set from scripts (pm.environment.set) also appear here.").small().weak());
    ui.separator();
    ui.horizontal(|ui| {
        if ui.small_button(if t.bulk { "Table Edit" } else { "Bulk Edit" }).clicked() {
            if !t.bulk {
                t.bulk_text = env.values.iter().map(|v| format!("{}{}:{}", if v.enabled { "" } else { "//" }, v.key, value_to_string(&v.value))).collect::<Vec<_>>().join("\n");
            }
            t.bulk = !t.bulk;
        }
    });
    if t.bulk {
        if ui.add(egui::TextEdit::multiline(&mut t.bulk_text).code_editor().desired_rows(16).desired_width(f32::INFINITY)).changed() {
            let mut new = vec![];
            for l in t.bulk_text.lines() {
                if l.trim().is_empty() {
                    continue;
                }
                let (dis, l) = match l.trim_start().strip_prefix("//") {
                    Some(r) => (true, r),
                    None => (false, l),
                };
                let (k, v) = l.split_once(':').unwrap_or((l, ""));
                let mut ev = env.values.iter().find(|x| x.key == k.trim()).cloned().unwrap_or_else(|| EnvVar::new(k.trim(), ""));
                ev.value = Value::String(v.to_string());
                ev.enabled = !dis;
                new.push(ev);
            }
            env.values = new;
            changed = true;
        }
    } else {
        if env.values.last().map(|v| !v.key.is_empty()).unwrap_or(true) {
            env.values.push(EnvVar::new("", ""));
        }
        let n = env.values.len();
        let f = t.filter.to_lowercase();
        let mut del = None;
        let w = ((ui.available_width() - 240.0) / 2.0).max(140.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("envgrid").num_columns(5).striped(true).spacing([6.0, 4.0]).show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new("Variable").strong());
                ui.label(RichText::new("Type").strong());
                ui.label(RichText::new("Value").strong());
                ui.label("");
                ui.end_row();
                for (idx, v) in env.values.iter_mut().enumerate() {
                    let last = idx == n - 1;
                    if !f.is_empty() && !last && !v.key.to_lowercase().contains(&f) {
                        continue;
                    }
                    if last {
                        ui.label("");
                    } else if ui.checkbox(&mut v.enabled, "").changed() {
                        changed = true;
                    }
                    if ui.add_sized([w, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut v.key).hint_text("Add new variable").font(egui::TextStyle::Monospace)).changed() {
                        changed = true;
                    }
                    let mut secret = v.is_secret();
                    egui::ComboBox::from_id_salt(("vt", idx)).width(70.0).selected_text(if secret { "secret" } else { "default" }).show_ui(ui, |ui| {
                        if ui.selectable_value(&mut secret, false, "default").clicked() | ui.selectable_value(&mut secret, true, "secret").clicked() {
                            v.kind = if secret { "secret".into() } else { "default".into() };
                            changed = true;
                        }
                    });
                    let mut s = value_to_string(&v.value);
                    let r = if v.is_secret() && !t.show_secrets { ui.add_sized([w, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s).password(true)) } else { ui.add_sized([w, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s).font(egui::TextStyle::Monospace)) };
                    if r.changed() {
                        v.value = Value::String(s);
                        changed = true;
                    }
                    if !last && ui.small_button("🗑").clicked() {
                        del = Some(idx);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(d) = del {
            env.values.remove(d);
            changed = true;
        }
    }
    if changed {
        actions.push(Action::EnvChanged(t.env_id.clone()));
    }
    // drop the trailing placeholder from saved data
    let _ = env.values.iter().filter(|v| !v.key.is_empty()).count();
}

// ------------------------------------------------------------------ collection / folder settings tab

#[derive(PartialEq, Clone, Copy)]
pub enum CollSection {
    Overview,
    Auth,
    Pre,
    Tests,
    Variables,
    Settings,
}

pub struct CollTab {
    pub coll_id: String,
    pub item_id: Option<String>,
    pub section: CollSection,
    pub bulk: bool,
    pub bulk_text: String,
    pub vars_kv: Option<Vec<KV>>,
}

impl CollTab {
    pub fn new(cid: &str, item: Option<String>) -> Self {
        CollTab { coll_id: cid.to_string(), item_id: item, section: CollSection::Overview, bulk: false, bulk_text: String::new(), vars_kv: None }
    }
}

fn vars_to_kv(v: &[Variable]) -> Vec<KV> {
    v.iter().map(|x| KV { key: x.key.clone(), value: value_to_string(&x.value), disabled: x.disabled, ..Default::default() }).collect()
}

fn kv_to_vars(kv: &[KV], old: &[Variable]) -> Vec<Variable> {
    kv.iter()
        .filter(|k| !k.key.is_empty())
        .map(|k| {
            let mut v = old.iter().find(|o| o.key == k.key).cloned().unwrap_or_default();
            v.key = k.key.clone();
            v.value = Value::String(k.value.clone());
            v.disabled = k.disabled;
            v
        })
        .collect()
}

fn coll_tab(app: &mut App, i: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    let dark = app.settings.dark;
    let (cid, iid) = match &app.tabs[i].kind {
        TabKind::Coll(c) => (c.coll_id.clone(), c.item_id.clone()),
        _ => return,
    };
    let vars = app.var_ctx(Some(&cid));
    let App { tabs, collections, .. } = app;
    let TabKind::Coll(t) = &mut tabs[i].kind else { return };
    let Some(c) = collections.iter_mut().find(|c| c.id() == cid) else {
        ui.label("Collection not found");
        return;
    };
    let mut changed = false;
    let is_folder = iid.is_some();
    let inherited = if let Some(f) = &iid {
        let p = c.find_path_by_id(f).unwrap_or_default();
        let mut src = None;
        for it in c.chain(&p).iter().rev().skip(1) {
            if let Some(a) = it.auth.as_ref().filter(|a| a.kind != "inherit") {
                src = Some((format!("folder \"{}\"", it.name), a.kind.clone()));
                break;
            }
        }
        src.or_else(|| c.auth.as_ref().filter(|a| a.kind != "inherit").map(|a| (format!("collection \"{}\"", c.info.name), a.kind.clone())))
    } else {
        None
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(if is_folder { "Folder" } else { "Collection" }).weak());
        let name = match &iid {
            Some(f) => c.find_path_by_id(f).and_then(|p| c.get(&p).map(|x| x.name.clone())).unwrap_or_default(),
            None => c.info.name.clone(),
        };
        ui.heading(name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("▶ Run").clicked() {
                actions.push(Action::Run(cid.clone(), iid.clone()));
            }
        });
    });
    ui.horizontal(|ui| {
        let mut secs = vec![(CollSection::Overview, "Overview"), (CollSection::Auth, "Authorization"), (CollSection::Pre, "Pre-request Script"), (CollSection::Tests, "Tests")];
        if !is_folder {
            secs.push((CollSection::Variables, "Variables"));
        }
        secs.push((CollSection::Settings, "Settings"));
        for (s, l) in secs {
            if ui.selectable_label(t.section == s, l).clicked() {
                t.section = s;
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // get mutable handles for events / auth / description
        match t.section {
            CollSection::Overview => {
                let desc: &mut Option<crate::model::Description> = match &iid {
                    Some(f) => {
                        let p = c.find_path_by_id(f).unwrap_or_default();
                        &mut c.get_mut(&p).unwrap().description
                    }
                    None => &mut c.info.description,
                };
                let mut d = desc.as_ref().map(|d| d.0.clone()).unwrap_or_default();
                ui.label(RichText::new("Description (Markdown)").weak());
                if ui.add(egui::TextEdit::multiline(&mut d).desired_rows(12).desired_width(f32::INFINITY)).changed() {
                    *desc = if d.is_empty() { None } else { Some(crate::model::Description(d)) };
                    changed = true;
                }
                if !is_folder {
                    let n = c.requests().len();
                    ui.add_space(8.0);
                    ui.label(format!("{n} requests · {} variables · id {}", c.variable.len(), c.id()));
                }
            }
            CollSection::Auth => {
                let auth = match &iid {
                    Some(f) => {
                        let p = c.find_path_by_id(f).unwrap_or_default();
                        &mut c.get_mut(&p).unwrap().auth
                    }
                    None => &mut c.auth,
                };
                ui.label(RichText::new("This authorization method will be used for every request in this collection/folder. You can override it in individual requests.").small().weak());
                if super::request::auth_editor(ui, auth, &vars, dark, is_folder, inherited.clone(), None, "", false, actions) {
                    changed = true;
                }
            }
            CollSection::Pre | CollSection::Tests => {
                let listen = if t.section == CollSection::Pre { "prerequest" } else { "test" };
                let events = match &iid {
                    Some(f) => {
                        let p = c.find_path_by_id(f).unwrap_or_default();
                        &mut c.get_mut(&p).unwrap().event
                    }
                    None => &mut c.event,
                };
                ui.label(RichText::new(if listen == "prerequest" { "This script will execute before every request in this collection/folder." } else { "This script will execute after every request in this collection/folder." }).small().weak());
                let mut code = get_script(events, listen);
                let snips = if listen == "prerequest" { super::request::SNIPPETS_PRE } else { super::request::SNIPPETS_TEST };
                if super::request::script_editor(ui, &mut code, snips, dark, listen) {
                    set_script(events, listen, &code);
                    changed = true;
                }
            }
            CollSection::Variables => {
                ui.label(RichText::new("Collection variables are available to all requests in this collection (pm.collectionVariables).").small().weak());
                let kv = t.vars_kv.get_or_insert_with(|| vars_to_kv(&c.variable));
                if kv_table(ui, "collvars", kv, &KvOpts { description: false, files: false, key_hint: "Variable", value_hint: "Value" }, &mut t.bulk, &mut t.bulk_text, &vars, dark) {
                    c.variable = kv_to_vars(kv, &c.variable);
                    changed = true;
                }
            }
            CollSection::Settings => {
                let ppb = match &iid {
                    Some(f) => {
                        let p = c.find_path_by_id(f).unwrap_or_default();
                        &mut c.get_mut(&p).unwrap().protocol_profile_behavior
                    }
                    None => &mut c.protocol_profile_behavior,
                };
                if super::request::behavior_editor(ui, ppb) {
                    changed = true;
                }
            }
        }
    });
    if changed {
        actions.push(Action::CollChanged(cid));
    }
}

// ------------------------------------------------------------------ websocket tab

pub struct WsMsg {
    pub text: String,
    pub incoming: bool,
    pub time: String,
    pub system: bool,
}

pub struct WsTab {
    pub name: String,
    pub url: String,
    pub headers: Vec<KV>,
    pub bulk: bool,
    pub bulk_text: String,
    pub conn: Option<crate::ws::WsConn>,
    pub connected: bool,
    pub connecting: bool,
    pub messages: Vec<WsMsg>,
    pub compose: String,
    pub show_headers: bool,
    pub filter: String,
    pub selected: Option<usize>,
}

impl WsTab {
    pub fn new() -> Self {
        WsTab { name: String::new(), url: String::new(), headers: vec![], bulk: false, bulk_text: String::new(), conn: None, connected: false, connecting: false, messages: vec![], compose: String::new(), show_headers: false, filter: String::new(), selected: None }
    }
}

fn ws_tab(app: &mut App, i: usize, ui: &mut Ui, _actions: &mut Vec<Action>) {
    let vars = app.var_ctx(None);
    let dark = app.settings.dark;
    let settings = app.settings.clone();
    let TabKind::Ws(t) = &mut app.tabs[i].kind else { return };
    // poll events
    if let Some(c) = &t.conn {
        while let Ok(e) = c.rx.try_recv() {
            match e {
                crate::ws::WsEvent::Connected(hs) => {
                    t.connected = true;
                    t.connecting = false;
                    t.messages.push(WsMsg { text: format!("Connected to {}\n{}", t.url, hs.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n")), incoming: true, time: chrono::Local::now().format("%H:%M:%S").to_string(), system: true });
                }
                crate::ws::WsEvent::Message { text, incoming, time, .. } => t.messages.push(WsMsg { text, incoming, time, system: false }),
                crate::ws::WsEvent::Error(e) => {
                    t.messages.push(WsMsg { text: format!("Error: {e}"), incoming: true, time: chrono::Local::now().format("%H:%M:%S").to_string(), system: true });
                    t.connected = false;
                    t.connecting = false;
                }
                crate::ws::WsEvent::Closed(r) => {
                    t.messages.push(WsMsg { text: format!("Disconnected: {r}"), incoming: true, time: chrono::Local::now().format("%H:%M:%S").to_string(), system: true });
                    t.connected = false;
                    t.connecting = false;
                }
            }
        }
        if !t.connected && !t.connecting {
            t.conn = None;
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("WS").strong().color(method_color("WS")));
        ui.add_sized([160.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut t.name).hint_text("Name"));
        let w = ui.available_width() - 110.0;
        widgets::var_edit(ui, &mut t.url, &vars, "ws://localhost:8080 or wss://…", w, dark);
        if t.connected || t.connecting {
            if ui.add_sized([100.0, 26.0], egui::Button::new("Disconnect")).clicked() {
                if let Some(c) = &t.conn {
                    let _ = c.tx.send(crate::ws::WsCmd::Close);
                }
            }
        } else if ui.add_sized([100.0, 26.0], egui::Button::new(RichText::new("Connect").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0x09, 0x7b, 0xed))).clicked() {
            let url = vars.resolve(&t.url);
            let hs: Vec<(String, String)> = t.headers.iter().filter(|h| !h.disabled && !h.key.is_empty()).map(|h| (vars.resolve(&h.key), vars.resolve(&h.value))).collect();
            let ctx = ui.ctx().clone();
            t.conn = Some(crate::ws::connect(&url, hs, &settings, move || ctx.request_repaint()));
            t.connecting = true;
        }
    });
    ui.horizontal(|ui| {
        if ui.selectable_label(!t.show_headers, "Message").clicked() {
            t.show_headers = false;
        }
        if ui.selectable_label(t.show_headers, format!("Headers ({})", t.headers.iter().filter(|h| !h.key.is_empty()).count())).clicked() {
            t.show_headers = true;
        }
    });
    ui.separator();
    if t.show_headers {
        kv_table(ui, "wsh", &mut t.headers, &KvOpts { description: false, files: false, key_hint: "Key", value_hint: "Value" }, &mut t.bulk, &mut t.bulk_text, &vars, dark);
        return;
    }
    widgets::code_editor(ui, &mut t.compose, widgets::Lang::Json, 5, dark, "wscompose");
    ui.horizontal(|ui| {
        let can = t.connected;
        if ui.add_enabled(can, egui::Button::new("Send")).on_hover_text("Ctrl+Enter sends the HTTP request; use this button for WebSocket").clicked() {
            if let Some(c) = &t.conn {
                let _ = c.tx.send(crate::ws::WsCmd::Text(vars.resolve(&t.compose)));
            }
        }
        if ui.add_enabled(can, egui::Button::new("Ping")).clicked() {
            if let Some(c) = &t.conn {
                let _ = c.tx.send(crate::ws::WsCmd::Ping);
            }
        }
        if ui.small_button("Format JSON").clicked() {
            if let Some(p) = widgets::pretty_json(&t.compose) {
                t.compose = p;
            }
        }
        ui.separator();
        ui.add_sized([180.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut t.filter).hint_text("🔍 Search messages"));
        if ui.button("Clear").clicked() {
            t.messages.clear();
        }
        if t.connecting {
            ui.spinner();
        } else if t.connected {
            ui.label(RichText::new("● Connected").color(Color32::from_rgb(0x3c, 0xc4, 0x7c)));
        }
    });
    ui.separator();
    let f = t.filter.to_lowercase();
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for m in &t.messages {
            if !f.is_empty() && !m.text.to_lowercase().contains(&f) {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                let (arrow, col) = if m.system {
                    ("●", Color32::GRAY)
                } else if m.incoming {
                    ("↓", Color32::from_rgb(0x5a, 0xa9, 0xf5))
                } else {
                    ("↑", Color32::from_rgb(0x3c, 0xc4, 0x7c))
                };
                ui.label(RichText::new(arrow).strong().color(col));
                ui.label(RichText::new(&m.time).small().monospace().weak());
                let text = if !m.system { widgets::pretty_json(&m.text).unwrap_or_else(|| m.text.clone()) } else { m.text.clone() };
                ui.add(egui::Label::new(RichText::new(text).monospace()).selectable(true));
            });
            ui.separator();
        }
    });
}

// ------------------------------------------------------------------ runner tab

pub enum RunMsg {
    Iter(usize),
    Item(Box<ItemResult>, Vec<ConsoleEntry>),
    Done(Box<RunSummary>, Box<VarContext>),
}

pub struct RunnerTab {
    pub coll_id: String,
    pub folder: Option<String>,
    pub iterations: usize,
    pub delay: u64,
    pub data_path: String,
    pub data: Vec<serde_json::Map<String, Value>>,
    pub data_err: String,
    pub bail: bool,
    pub persist: bool,
    pub selection: Vec<(String, String, String, bool)>,
    pub rx: Option<Receiver<RunMsg>>,
    pub cancel: Arc<AtomicBool>,
    pub results: Vec<ItemResult>,
    pub summary: Option<RunSummary>,
    pub iter: usize,
    pub filter: u8,
    pub started: Option<std::time::Instant>,
    pub vars_before: Option<VarContext>,
    pub env_id: Option<String>,
}

impl RunnerTab {
    pub fn new(cid: &str, folder: Option<String>) -> Self {
        RunnerTab { coll_id: cid.into(), folder, iterations: 1, delay: 0, data_path: String::new(), data: vec![], data_err: String::new(), bail: false, persist: true, selection: vec![], rx: None, cancel: Arc::new(AtomicBool::new(false)), results: vec![], summary: None, iter: 0, filter: 0, started: None, vars_before: None, env_id: None }
    }
    pub fn init_selection(&mut self, c: &Collection) {
        let fpath = self.folder.as_ref().and_then(|f| c.find_path_by_id(f));
        self.selection = c
            .requests()
            .into_iter()
            .filter(|(p, _)| fpath.as_ref().map(|f| p.starts_with(f)).unwrap_or(true))
            .map(|(p, it)| {
                let chain = c.chain(&p);
                let path = chain.iter().take(chain.len() - 1).map(|i| i.name.clone()).collect::<Vec<_>>().join(" / ");
                let name = if path.is_empty() { it.name.clone() } else { format!("{path} / {}", it.name) };
                (it.id().to_string(), name, it.request.as_ref().map(|r| r.method.clone()).unwrap_or_default(), true)
            })
            .collect();
    }
}

fn runner_tab(app: &mut App, i: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    // poll
    let mut done: Option<(RunSummary, VarContext)> = None;
    if let TabKind::Runner(t) = &mut app.tabs[i].kind {
        if let Some(rx) = &t.rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    RunMsg::Iter(n) => t.iter = n,
                    RunMsg::Item(r, cons) => {
                        t.results.push(*r);
                        for c in cons {
                            actions.push(Action::Console(c));
                        }
                    }
                    RunMsg::Done(s, v) => done = Some((*s, *v)),
                }
            }
        }
        if done.is_some() {
            t.rx = None;
        }
    }
    if let Some((s, v)) = done {
        let (persist, before, env_id, cid) = match &mut app.tabs[i].kind {
            TabKind::Runner(t) => {
                t.summary = Some(s);
                (t.persist, t.vars_before.take(), t.env_id.clone(), t.coll_id.clone())
            }
            _ => unreachable!(),
        };
        if persist {
            if let Some(b) = before {
                if b.environment.to_json() != v.environment.to_json() {
                    if let Some(id) = env_id {
                        if let Some(e) = app.envs.iter_mut().find(|e| e.id == id) {
                            v.environment.write_env(e);
                            actions.push(Action::EnvChanged(id));
                        }
                    }
                }
                if b.globals.to_json() != v.globals.to_json() {
                    v.globals.write_env(&mut app.globals);
                    actions.push(Action::GlobalsChanged);
                }
                if b.collection.to_json() != v.collection.to_json() {
                    if let Some(c) = app.collections.iter_mut().find(|c| c.id() == cid) {
                        v.collection.write_vars(&mut c.variable);
                        actions.push(Action::CollChanged(cid));
                    }
                }
            }
        }
    }
    let env_name = app.active_env().map(|e| e.name.clone()).unwrap_or_else(|| "No Environment".into());
    let TabKind::Runner(t) = &app.tabs[i].kind else { return };
    let cid = t.coll_id.clone();
    let coll = app.coll(&cid).cloned();
    let vars = app.var_ctx(Some(&cid));
    let settings = app.settings.clone();
    let jar = app.jar.clone();
    let env_id = app.active_env.clone();
    let TabKind::Runner(t) = &mut app.tabs[i].kind else { return };
    let Some(coll) = coll else {
        ui.label("Collection not found");
        return;
    };
    let running = t.rx.is_some();
    ui.horizontal(|ui| {
        ui.heading(format!("Runner — {}", coll.info.name));
        if let Some(f) = &t.folder {
            if let Some(p) = coll.find_path_by_id(f) {
                ui.label(RichText::new(format!("/ {}", coll.get(&p).map(|x| x.name.clone()).unwrap_or_default())).weak());
            }
        }
        ui.label(RichText::new(format!("Environment: {env_name}")).weak());
    });
    ui.separator();
    egui::Panel::left("runleft").resizable(true).default_size(440.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("runsel").max_height(ui.available_height()).show(ui, |ui| {
            ui.add_space(8.0);
            egui::Grid::new("runopts").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Iterations");
                ui.add(egui::DragValue::new(&mut t.iterations).range(1..=100000));
                ui.end_row();
                ui.label("Delay (ms)");
                ui.add(egui::DragValue::new(&mut t.delay).range(0..=600000));
                ui.end_row();
                ui.label("Data file (CSV/JSON)");
                ui.horizontal(|ui| {
                    let r = ui.add_sized([220.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut t.data_path).hint_text("path"));
                    let mut load = r.lost_focus();
                    if super::files::can_browse() && ui.button("Select…").clicked() {
                        if let Some(f) = super::files::pick_file(&[("Data", &["csv", "json"])]) {
                            t.data_path = f;
                            load = true;
                        }
                    }
                    if load {
                        if t.data_path.trim().is_empty() {
                            t.data.clear();
                            t.data_err.clear();
                        } else {
                            match crate::runner::parse_data_file(t.data_path.trim()) {
                                Ok(d) => {
                                    t.iterations = d.len().max(1);
                                    t.data = d;
                                    t.data_err.clear();
                                }
                                Err(e) => t.data_err = e,
                            }
                        }
                    }
                });
                ui.end_row();
                if !t.data_err.is_empty() {
                    ui.label("");
                    ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), &t.data_err);
                    ui.end_row();
                } else if !t.data.is_empty() {
                    ui.label("");
                    ui.label(RichText::new(format!("{} rows · columns: {}", t.data.len(), t.data[0].keys().cloned().collect::<Vec<_>>().join(", "))).small());
                    ui.end_row();
                }
                ui.label("");
                ui.checkbox(&mut t.persist, "Persist responses' variable changes (environment, globals, collection)");
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut t.bail, "Stop run on first error / failed test");
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if running {
                    if ui.button("■ Stop").clicked() {
                        t.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.spinner();
                    ui.label(format!("Iteration {} · {} requests", t.iter + 1, t.results.len()));
                } else if ui.add(egui::Button::new(RichText::new(format!("▶ Run {}", coll.info.name)).strong().color(Color32::WHITE)).fill(Color32::from_rgb(0xff, 0x6c, 0x37))).clicked() {
                    let ids: Vec<String> = t.selection.iter().filter(|s| s.3).map(|s| s.0.clone()).collect();
                    let opts = RunOptions { iterations: t.iterations, delay_ms: t.delay, data: t.data.clone(), bail: t.bail, selected: Some(ids), folder: None };
                    let (tx, rx) = std::sync::mpsc::channel();
                    t.cancel = Arc::new(AtomicBool::new(false));
                    let cancel = t.cancel.clone();
                    let ctx = ui.ctx().clone();
                    let v = vars.clone();
                    let c2 = coll.clone();
                    let s2 = settings.clone();
                    let j2 = jar.clone();
                    std::thread::spawn(move || {
                        crate::runner::run_collection(&c2, v, &opts, &s2, Some(j2), cancel, |ev| {
                            let m = match ev {
                                RunEvent::IterationStart(i) => RunMsg::Iter(i),
                                RunEvent::Item(r, c) => RunMsg::Item(Box::new(r), c),
                                RunEvent::Done(s, v) => RunMsg::Done(Box::new(s), Box::new(v)),
                            };
                            let _ = tx.send(m);
                            ctx.request_repaint();
                        });
                    });
                    t.rx = Some(rx);
                    t.results.clear();
                    t.summary = None;
                    t.iter = 0;
                    t.started = Some(std::time::Instant::now());
                    t.vars_before = Some(vars.clone());
                    t.env_id = env_id.clone();
                }
            });
                    ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Run order").strong());
                if ui.small_button("Select all").clicked() {
                    t.selection.iter_mut().for_each(|s| s.3 = true);
                }
                if ui.small_button("Deselect all").clicked() {
                    t.selection.iter_mut().for_each(|s| s.3 = false);
                }
                if ui.small_button("Reset").clicked() {
                    t.init_selection(&coll);
                }
            });
            let n = t.selection.len();
            let mut swap: Option<(usize, usize)> = None;
            for (k, s) in t.selection.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.3, "");
                    ui.label(RichText::new(widgets::short_method(&s.2)).small().monospace().color(method_color(&s.2)));
                    ui.label(&s.1);
                    if k > 0 && ui.small_button("⏶").clicked() {
                        swap = Some((k, k - 1));
                    }
                    if k + 1 < n && ui.small_button("⏷").clicked() {
                        swap = Some((k, k + 1));
                    }
                });
            }
            if let Some((a, b)) = swap {
                t.selection.swap(a, b);
            }
        });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Results").strong());
            let passed: usize = t.results.iter().map(|r| r.tests.iter().filter(|x| x.passed && !x.skipped).count()).sum();
            let failed: usize = t.results.iter().map(|r| r.tests.iter().filter(|x| !x.passed && !x.skipped).count()).sum();
            for (v, l) in [(0u8, format!("All ({})", passed + failed)), (1, format!("Passed ({passed})")), (2, format!("Failed ({failed})"))] {
                if ui.selectable_label(t.filter == v, l).clicked() {
                    t.filter = v;
                }
            }
            if !t.results.is_empty() && ui.small_button("Export…").clicked() {
                let j = serde_json::json!({ "collection": coll.info.name, "summary": t.summary, "results": t.results });
                actions.push(Action::Dialog(super::dialogs::Dialog::export(format!("{}.postman_test_run.json", crate::storage::safe_name(&coll.info.name)), serde_json::to_string_pretty(&j).unwrap_or_default())));
            }
        });
        if let Some(s) = &t.summary {
            ui.label(RichText::new(format!("{} iterations · {} requests · {} passed · {} failed · {:.1}s · avg {:.0} ms{}", s.iterations, s.requests, s.tests_passed, s.tests_failed, s.total_time_ms / 1000.0, s.avg_response_ms, if s.stopped { " · stopped" } else { "" })).small());
        }
        ui.separator();
        egui::ScrollArea::vertical().id_salt("runres").auto_shrink([false, false]).stick_to_bottom(running).show(ui, |ui| {
            let mut last_iter = usize::MAX;
            for r in &t.results {
                let failed = r.tests.iter().any(|x| !x.passed && !x.skipped) || r.error.is_some();
                if (t.filter == 1 && failed) || (t.filter == 2 && !failed) {
                    continue;
                }
                if r.iteration != last_iter {
                    last_iter = r.iteration;
                    ui.label(RichText::new(format!("Iteration {}", r.iteration + 1)).strong().weak());
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(widgets::short_method(&r.method)).small().monospace().color(method_color(&r.method)));
                    ui.label(RichText::new(&r.name).strong());
                    if r.status > 0 {
                        ui.label(RichText::new(format!("{} {}", r.status, r.reason)).color(widgets::status_color(r.status)));
                        ui.label(RichText::new(format!("{:.0} ms · {}", r.time_ms, crate::runner::human_size(r.size))).small().weak());
                    } else if r.skipped {
                        ui.label(RichText::new("skipped").weak());
                    }
                });
                if !r.url.is_empty() {
                    ui.label(RichText::new(&r.url).small().monospace().weak());
                }
                if let Some(e) = &r.error {
                    ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), e);
                }
                for x in &r.tests {
                    ui.horizontal_wrapped(|ui| {
                        let (l, c) = if x.skipped {
                            ("SKIP", Color32::GRAY)
                        } else if x.passed {
                            ("PASS", Color32::from_rgb(0x2e, 0x9e, 0x62))
                        } else {
                            ("FAIL", Color32::from_rgb(0xd0, 0x45, 0x40))
                        };
                        ui.add_space(12.0);
                        ui.label(RichText::new(format!(" {l} ")).small().strong().background_color(c).color(Color32::WHITE));
                        ui.label(&x.name);
                        if !x.passed {
                            if let Some(e) = &x.error {
                                ui.label(RichText::new(e).small().color(Color32::from_rgb(0xec, 0x7b, 0x76)));
                            }
                        }
                    });
                }
                ui.separator();
            }
        });
    });
    let _ = Scope::default();
}
