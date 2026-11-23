#![allow(deprecated)]
//! postmini GUI (egui). Immediate-mode, reactive: repaints only on input or background events,
//! so the idle CPU cost is ~0 and memory stays small.

mod dialogs;
mod files;
mod request;
mod response;
mod sidebar;
mod tabs;
mod widgets;

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};

use crate::http::CookieJar;
use crate::mock::MockServer;
use crate::model::{Collection, Environment, Item, Request};
use crate::runner::{ConsoleEntry, Execution};
use crate::settings::Settings;
use crate::storage::{HistoryEntry, State, Store};
use crate::vars::{Scope, VarContext};

pub use tabs::{CollTab, EnvTab, RunnerTab, WsTab};

// ------------------------------------------------------------------ tabs

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, Default)]
pub enum ReqSection {
    #[default]
    Params,
    Auth,
    Headers,
    Body,
    PreRequest,
    Tests,
    Settings,
    Docs,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum RespSection {
    #[default]
    Body,
    Cookies,
    Headers,
    Tests,
    Timeline,
    Visualize,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum BodyView {
    #[default]
    Pretty,
    Raw,
    Preview,
}

pub struct SendResult {
    pub exec: Execution,
    pub vars_before: VarContext,
    pub vars_after: VarContext,
    pub coll_id: Option<String>,
    pub env_id: Option<String>,
}

pub struct ReqTab {
    pub item: Item,
    pub origin: Option<(String, String)>,
    pub dirty: bool,
    pub section: ReqSection,
    pub resp_section: RespSection,
    pub bulk: bool,
    pub bulk_text: String,
    pub show_auto_headers: bool,
    pub inflight: Option<(Receiver<SendResult>, Instant)>,
    pub result: Option<SendResult>,
    pub body_view: BodyView,
    pub resp_index: Option<widgets::TextIndex>,
    pub resp_filter: String,
    pub wrap: bool,
    pub oauth_busy: Option<Receiver<Result<crate::auth::TokenResult, String>>>,
    pub oauth_msg: String,
    pub split: f32,
}

impl ReqTab {
    pub fn new(item: Item, origin: Option<(String, String)>) -> Self {
        ReqTab {
            item,
            origin,
            dirty: false,
            section: ReqSection::Params,
            resp_section: RespSection::Body,
            bulk: false,
            bulk_text: String::new(),
            show_auto_headers: false,
            inflight: None,
            result: None,
            body_view: BodyView::Pretty,
            resp_index: None,
            resp_filter: String::new(),
            wrap: false,
            oauth_busy: None,
            oauth_msg: String::new(),
            split: 0.5,
        }
    }
    pub fn req(&mut self) -> &mut Request {
        self.item.request.get_or_insert_with(Request::default)
    }
}

pub enum TabKind {
    Request(Box<ReqTab>),
    Env(EnvTab),
    Coll(Box<CollTab>),
    Ws(Box<WsTab>),
    Runner(Box<RunnerTab>),
}

pub struct Tab {
    pub id: u64,
    pub kind: TabKind,
}

#[derive(Serialize, Deserialize, Default)]
struct SavedTab {
    item: Item,
    origin: Option<(String, String)>,
    dirty: bool,
    #[serde(default)]
    ws: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct SavedTabs {
    tabs: Vec<SavedTab>,
    active: usize,
}

// ------------------------------------------------------------------ actions

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Collection(String),
    Item(String, String),
}

#[derive(Clone, Debug)]
pub enum DropPos {
    Into,
    Before,
}

pub enum Action {
    OpenItem(String, String),
    OpenCollSettings(String, Option<String>),
    NewRequest,
    NewWs,
    NewCollection,
    NewEnvironment,
    AddRequest(String, Option<String>),
    AddFolder(String, Option<String>),
    Rename(Target),
    Delete(Target),
    Duplicate(Target),
    Move(Target, i32),
    DragMove { from: Target, to: Target, pos: DropPos },
    Export(Target),
    Run(String, Option<String>),
    StartMock(String),
    StopMock(usize),
    OpenEnv(String),
    SetActiveEnv(Option<String>),
    DuplicateEnv(String),
    DeleteEnv(String),
    ExportEnv(String),
    OpenHistory(usize),
    DeleteHistory(usize),
    ClearHistory,
    SaveHistoryTo(usize),
    Send(u64),
    Cancel(u64),
    Save(u64),
    SaveAs(u64),
    CloseTab(u64),
    CloseOthers(u64),
    DuplicateTab(u64),
    SaveExample(u64),
    Toast(String),
    Error(String),
    Dialog(dialogs::Dialog),
    Console(ConsoleEntry),
    CollChanged(String),
    EnvChanged(String),
    GlobalsChanged,
    ImportText(String),
    ImportFiles(Vec<String>),
    OpenCode(u64),
    FocusTab(u64),
    GetOAuthToken(u64, Option<String>),
    Toggle(String),
}

#[derive(PartialEq, Clone, Copy)]
pub enum SidebarMode {
    Collections,
    Environments,
    History,
    Mocks,
}

pub struct App {
    pub store: Store,
    pub collections: Vec<Collection>,
    pub envs: Vec<Environment>,
    pub globals: Environment,
    pub active_env: Option<String>,
    pub history: Vec<HistoryEntry>,
    pub settings: Settings,
    pub jar: CookieJar,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub next_id: u64,
    pub sidebar: SidebarMode,
    pub show_sidebar: bool,
    pub filter: String,
    pub expanded: HashSet<String>,
    pub dialog: Option<dialogs::Dialog>,
    pub console: Vec<ConsoleEntry>,
    pub show_console: bool,
    pub console_filter: String,
    pub mocks: Vec<(String, String, MockServer)>,
    pub toasts: Vec<(String, Instant, bool)>,
    pub dirty_colls: HashSet<String>,
    pub dirty_envs: HashSet<String>,
    pub dirty_globals: bool,
    pub dirty_history: bool,
    pub last_save: Instant,
    pub quicklook: bool,
    pub applied_style: Option<(bool, f32, f32)>,
}

pub fn run() -> Result<(), String> {
    let store = Store::new(crate::storage::data_dir());
    let settings = store.load_settings();
    let mut opts = eframe::NativeOptions::default();
    opts.viewport = egui::ViewportBuilder::default().with_title("postmini").with_inner_size([1280.0, 800.0]).with_min_inner_size([700.0, 450.0]).with_drag_and_drop(true).with_icon(icon());
    opts.persist_window = true;
    // Lower memory: no multisampling, no depth/stencil buffers
    opts.multisampling = 0;
    opts.depth_buffer = 0;
    opts.stencil_buffer = 0;
    opts.glow_options.vsync = true;
    eframe::run_native(
        "postmini",
        opts,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let mut fonts = egui::FontDefinitions::default();
            // Hack covers ● ↑ ↓ ← ⇅ etc. which Ubuntu-Light lacks
            if let Some(p) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                p.insert(1, "Hack".to_owned());
            }
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(App::new(store, settings, &cc.egui_ctx)))
        }),
    )
    .map_err(|e| e.to_string())
}

fn icon() -> egui::IconData {
    // simple generated 32x32 icon: orange circle with white "P"-ish bar
    let n = 32usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let d = (dx * dx + dy * dy).sqrt();
            let i = (y * n + x) * 4;
            if d < 15.5 {
                let white = (x >= 10 && x <= 13 && y >= 7 && y <= 24) || ((y == 7 || y == 8 || y == 15 || y == 16) && x >= 10 && x <= 19) || (x >= 19 && x <= 21 && y >= 8 && y <= 16);
                let c = if white { [255, 255, 255] } else { [255, 108, 55] };
                rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}

impl App {
    fn new(store: Store, settings: Settings, ctx: &egui::Context) -> Self {
        let collections = store.load_collections();
        let envs = store.load_environments();
        let globals = store.load_globals();
        let history: Vec<HistoryEntry> = store.load("history.json");
        let state: State = store.load("state.json");
        let jar = store.jar();
        let mut app = App {
            collections,
            envs,
            globals,
            active_env: state.active_env.clone(),
            history,
            settings,
            jar,
            tabs: vec![],
            active: 0,
            next_id: 1,
            sidebar: SidebarMode::Collections,
            show_sidebar: true,
            filter: String::new(),
            expanded: state.expanded.into_iter().collect(),
            dialog: None,
            console: vec![],
            show_console: false,
            console_filter: String::new(),
            mocks: vec![],
            toasts: vec![],
            dirty_colls: HashSet::new(),
            dirty_envs: HashSet::new(),
            dirty_globals: false,
            dirty_history: false,
            last_save: Instant::now(),
            quicklook: false,
            applied_style: None,
            store,
        };
        if app.active_env.as_ref().map(|id| !app.envs.iter().any(|e| &e.id == id)).unwrap_or(false) {
            app.active_env = None;
        }
        let saved: SavedTabs = app.store.load("tabs.json");
        for t in saved.tabs {
            if let Some(url) = t.ws {
                let mut w = WsTab::new();
                w.url = url;
                w.name = t.item.name.clone();
                app.push_tab(TabKind::Ws(Box::new(w)));
            } else {
                let mut rt = ReqTab::new(t.item, t.origin);
                rt.dirty = t.dirty;
                app.push_tab(TabKind::Request(Box::new(rt)));
            }
        }
        app.active = saved.active.min(app.tabs.len().saturating_sub(1));
        if app.tabs.is_empty() {
            app.new_request_tab(Item::new_request("Untitled Request", Request::default()), None);
        }
        app.apply_style(ctx);
        // first-run welcome collection
        if app.collections.is_empty() && !app.store.dir.join(".welcomed").exists() {
            let _ = std::fs::write(app.store.dir.join(".welcomed"), "1");
        }
        app
    }

    pub fn apply_style(&mut self, ctx: &egui::Context) {
        let key = (self.settings.dark, self.settings.zoom, self.settings.font_size);
        if self.applied_style == Some(key) {
            return;
        }
        self.applied_style = Some(key);
        ctx.set_theme(if self.settings.dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.set_zoom_factor(self.settings.zoom.clamp(0.5, 3.0));
        let fs = self.settings.font_size.clamp(9.0, 24.0);
        ctx.all_styles_mut(|s| {
            use egui::{FontFamily, FontId, TextStyle};
            s.text_styles = [
                (TextStyle::Heading, FontId::new(fs + 5.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(fs, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(fs - 0.5, FontFamily::Monospace)),
                (TextStyle::Button, FontId::new(fs, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(fs - 2.5, FontFamily::Proportional)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(6.0, 3.0);
            s.visuals.selection.bg_fill = Color32::from_rgb(0xff, 0x6c, 0x37).gamma_multiply(0.6);
            s.visuals.hyperlink_color = Color32::from_rgb(0xff, 0x8a, 0x5c);
        });
    }

    pub fn push_tab(&mut self, kind: TabKind) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab { id, kind });
        self.active = self.tabs.len() - 1;
        id
    }

    pub fn new_request_tab(&mut self, item: Item, origin: Option<(String, String)>) -> u64 {
        self.push_tab(TabKind::Request(Box::new(ReqTab::new(item, origin))))
    }

    pub fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn coll(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id() == id)
    }
    pub fn coll_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.id() == id)
    }
    pub fn active_env(&self) -> Option<&Environment> {
        self.active_env.as_ref().and_then(|id| self.envs.iter().find(|e| &e.id == id))
    }

    pub fn var_ctx(&self, coll_id: Option<&str>) -> VarContext {
        let mut v = VarContext { globals: Scope::from_env(&self.globals), ..Default::default() };
        if let Some(e) = self.active_env() {
            v.environment = Scope::from_env(e);
            v.env_name = e.name.clone();
        }
        if let Some(c) = coll_id.and_then(|id| self.coll(id)) {
            v.collection = Scope::from_vars(&c.variable);
        }
        v
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toasts.push((s.into(), Instant::now(), false));
    }
    pub fn error(&mut self, s: impl Into<String>) {
        let s = s.into();
        self.console.push(ConsoleEntry::new("error", s.clone()));
        self.toasts.push((s, Instant::now(), true));
    }

    pub fn log(&mut self, e: ConsoleEntry) {
        self.console.push(e);
        if self.console.len() > 3000 {
            self.console.drain(0..500);
        }
    }

    // ------------------------------------------------------------ persistence
    fn save_all(&mut self, force: bool) {
        if !force && self.last_save.elapsed() < Duration::from_millis(800) {
            return;
        }
        self.last_save = Instant::now();
        for id in std::mem::take(&mut self.dirty_colls) {
            if let Some(c) = self.coll(&id) {
                if let Err(e) = self.store.save_collection(c) {
                    self.error(format!("Saving collection failed: {e}"));
                }
            }
        }
        for id in std::mem::take(&mut self.dirty_envs) {
            if let Some(e) = self.envs.iter().find(|e| e.id == id) {
                let _ = self.store.save_environment(e);
            }
        }
        if std::mem::take(&mut self.dirty_globals) {
            self.store.save_globals(&self.globals);
        }
        if std::mem::take(&mut self.dirty_history) {
            self.store.save("history.json", &self.history);
        }
    }

    fn save_state(&mut self) {
        self.save_all(true);
        let st = State { active_env: self.active_env.clone(), open_tabs: vec![], expanded: self.expanded.iter().cloned().collect() };
        self.store.save("state.json", &st);
        let mut saved = SavedTabs { tabs: vec![], active: 0 };
        for (i, t) in self.tabs.iter().enumerate() {
            match &t.kind {
                TabKind::Request(r) => {
                    if i == self.active {
                        saved.active = saved.tabs.len();
                    }
                    saved.tabs.push(SavedTab { item: r.item.clone(), origin: r.origin.clone(), dirty: r.dirty, ws: None })
                }
                TabKind::Ws(w) => saved.tabs.push(SavedTab { item: Item { name: w.name.clone(), ..Default::default() }, origin: None, dirty: false, ws: Some(w.url.clone()) }),
                _ => {}
            }
        }
        self.store.save("tabs.json", &saved);
        self.store.save_jar(&self.jar);
        self.store.save_settings(&self.settings);
    }

    // ------------------------------------------------------------ send
    pub fn send(&mut self, tab_id: u64, ctx: &egui::Context) {
        let Some(i) = self.tab_index(tab_id) else { return };
        let (item, origin) = match &self.tabs[i].kind {
            TabKind::Request(r) => {
                if r.inflight.is_some() {
                    return;
                }
                (r.item.clone(), r.origin.clone())
            }
            _ => return,
        };
        let coll_id = origin.as_ref().map(|o| o.0.clone());
        let coll: Option<Collection> = coll_id.as_ref().and_then(|id| self.coll(id).cloned());
        let path = match (&coll, &origin) {
            (Some(c), Some((_, iid))) => c.find_path_by_id(iid).unwrap_or_default(),
            _ => vec![],
        };
        let mut item = item;
        if let Some(r) = item.request.as_mut() {
            widgets::clean_rows(&mut r.header);
            widgets::clean_rows(&mut r.url.query);
            if let Some(b) = r.body.as_mut() {
                widgets::clean_rows(&mut b.urlencoded);
                widgets::clean_rows(&mut b.formdata);
            }
        }
        let vars = self.var_ctx(coll_id.as_deref());
        let settings = self.settings.clone();
        let jar = self.jar.clone();
        let env_id = self.active_env.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::Builder::new()
            .name("send".into())
            .spawn(move || {
                let mut v = vars.clone();
                let loc = crate::runner::Location { collection: coll.as_ref(), path };
                let exec = crate::runner::execute_item(&item, &loc, &mut v, &settings, Some(jar), &crate::runner::IterInfo { iteration: 0, count: 1 });
                let _ = tx.send(SendResult { exec, vars_before: vars, vars_after: v, coll_id, env_id });
                ctx2.request_repaint();
            })
            .ok();
        if let TabKind::Request(r) = &mut self.tabs[i].kind {
            r.inflight = Some((rx, Instant::now()));
        }
    }

    fn poll(&mut self) {
        let mut finished: Vec<(usize, SendResult)> = vec![];
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if let TabKind::Request(r) = &mut t.kind {
                if let Some((rx, _)) = &r.inflight {
                    if let Ok(res) = rx.try_recv() {
                        r.inflight = None;
                        finished.push((i, res));
                    }
                }
                if let Some(rx) = &r.oauth_busy {
                    if let Ok(res) = rx.try_recv() {
                        r.oauth_busy = None;
                        match res {
                            Ok(tok) => {
                                let req = r.item.request.get_or_insert_with(Request::default);
                                let a = req.auth.get_or_insert_with(|| crate::model::Auth::new("oauth2"));
                                a.set_str("accessToken", &tok.access_token);
                                if !tok.refresh_token.is_empty() {
                                    a.set_str("refreshToken", &tok.refresh_token);
                                }
                                if !tok.token_type.is_empty() && !tok.token_type.eq_ignore_ascii_case("bearer") {
                                    a.set_str("tokenType", &tok.token_type);
                                }
                                r.oauth_msg = format!("Token received{}", tok.expires_in.map(|e| format!(", expires in {e}s")).unwrap_or_default());
                                r.dirty = true;
                            }
                            Err(e) => r.oauth_msg = format!("Error: {e}"),
                        }
                    }
                }
            }
        }
        for (i, res) in finished {
            self.finish_send(i, res);
        }
    }

    fn finish_send(&mut self, i: usize, res: SendResult) {
        for c in &res.exec.console {
            self.console.push(c.clone());
        }
        if self.console.len() > 3000 {
            self.console.drain(0..500);
        }
        if let Some(e) = &res.exec.error {
            self.toasts.push((crate::runner::truncate(e, 200), Instant::now(), true));
        }
        // write variable changes back
        if self.settings.auto_persist_vars {
            if res.vars_after.environment.to_json() != res.vars_before.environment.to_json() {
                if let Some(id) = &res.env_id {
                    if let Some(e) = self.envs.iter_mut().find(|e| &e.id == id) {
                        res.vars_after.environment.write_env(e);
                        self.dirty_envs.insert(id.clone());
                    }
                }
            }
            if res.vars_after.globals.to_json() != res.vars_before.globals.to_json() {
                res.vars_after.globals.write_env(&mut self.globals);
                self.dirty_globals = true;
            }
            if res.vars_after.collection.to_json() != res.vars_before.collection.to_json() {
                if let Some(cid) = &res.coll_id {
                    if let Some(c) = self.collections.iter_mut().find(|c| c.id() == cid) {
                        res.vars_after.collection.write_vars(&mut c.variable);
                        self.dirty_colls.insert(cid.clone());
                    }
                }
            }
        }
        // history
        if let TabKind::Request(r) = &self.tabs[i].kind {
            if !res.exec.skipped {
                let entry = HistoryEntry {
                    id: crate::model::new_id(),
                    time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    name: r.item.name.clone(),
                    method: r.item.request.as_ref().map(|q| q.method.clone()).unwrap_or_default(),
                    url: r.item.request.as_ref().map(|q| q.url.raw.clone()).unwrap_or_default(),
                    status: res.exec.response.as_ref().map(|r| r.status).unwrap_or(0),
                    time_ms: res.exec.response.as_ref().map(|r| r.timings.total_ms).unwrap_or(0.0),
                    request: r.item.request.clone().unwrap_or_default(),
                };
                self.history.insert(0, entry);
                let lim = self.settings.history_limit.max(10);
                self.history.truncate(lim);
                self.dirty_history = true;
            }
        }
        if let TabKind::Request(r) = &mut self.tabs[i].kind {
            r.resp_index = None;
            if res.exec.visualizer.is_some() && r.resp_section == RespSection::Body {
                // keep body
            }
            if res.exec.response.is_none() && !res.exec.tests.is_empty() {
                r.resp_section = RespSection::Tests;
            }
            r.result = Some(res);
        }
    }

    // ------------------------------------------------------------ save
    pub fn save_tab(&mut self, tab_id: u64) {
        let Some(i) = self.tab_index(tab_id) else { return };
        let (item, origin) = match &mut self.tabs[i].kind {
            TabKind::Request(r) => {
                if let Some(req) = r.item.request.as_mut() {
                    widgets::clean_rows(&mut req.header);
                    widgets::clean_rows(&mut req.url.query);
                    widgets::clean_rows(&mut req.url.variable);
                    if let Some(b) = req.body.as_mut() {
                        widgets::clean_rows(&mut b.urlencoded);
                        widgets::clean_rows(&mut b.formdata);
                    }
                }
                (r.item.clone(), r.origin.clone())
            }
            _ => return,
        };
        match origin {
            Some((cid, iid)) => {
                let mut ok = false;
                if let Some(c) = self.coll_mut(&cid) {
                    if let Some(p) = c.find_path_by_id(&iid) {
                        if let Some(it) = c.get_mut(&p) {
                            let responses = std::mem::take(&mut it.response);
                            *it = item;
                            it.response = responses;
                            ok = true;
                        }
                    }
                }
                if ok {
                    self.dirty_colls.insert(cid);
                    self.save_all(true);
                    if let TabKind::Request(r) = &mut self.tabs[i].kind {
                        r.dirty = false;
                    }
                    self.toast("Saved");
                } else {
                    self.dialog = Some(dialogs::Dialog::save_as(tab_id, &self.tabs[i]));
                }
            }
            None => self.dialog = Some(dialogs::Dialog::save_as(tab_id, &self.tabs[i])),
        }
    }

    pub fn open_item(&mut self, cid: &str, iid: &str) {
        // existing tab?
        for (i, t) in self.tabs.iter().enumerate() {
            if let TabKind::Request(r) = &t.kind {
                if r.origin.as_ref().map(|o| o.0 == cid && o.1 == iid).unwrap_or(false) {
                    self.active = i;
                    return;
                }
            }
        }
        let item = self.coll(cid).and_then(|c| c.find_path_by_id(iid).and_then(|p| c.get(&p).cloned()));
        if let Some(item) = item {
            if item.is_folder() {
                self.open_coll_settings(cid, Some(iid.to_string()));
                return;
            }
            // replace the current tab if it is an untouched empty request (Postman "preview" behaviour)
            if let Some(TabKind::Request(r)) = self.tabs.get(self.active).map(|t| &t.kind) {
                if r.origin.is_none() && !r.dirty && r.result.is_none() && r.item.request.as_ref().map(|q| q.url.raw.is_empty()).unwrap_or(true) {
                    let id = self.tabs[self.active].id;
                    self.tabs[self.active] = Tab { id, kind: TabKind::Request(Box::new(ReqTab::new(item, Some((cid.into(), iid.into()))))) };
                    return;
                }
            }
            self.new_request_tab(item, Some((cid.into(), iid.into())));
        }
    }

    pub fn open_coll_settings(&mut self, cid: &str, folder: Option<String>) {
        for (i, t) in self.tabs.iter().enumerate() {
            if let TabKind::Coll(c) = &t.kind {
                if c.coll_id == cid && c.item_id == folder {
                    self.active = i;
                    return;
                }
            }
        }
        self.push_tab(TabKind::Coll(Box::new(CollTab::new(cid, folder))));
    }

    // ------------------------------------------------------------ action dispatch
    fn handle(&mut self, a: Action, ctx: &egui::Context) {
        match a {
            Action::OpenItem(c, i) => self.open_item(&c, &i),
            Action::Toggle(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Action::OpenCollSettings(c, f) => self.open_coll_settings(&c, f),
            Action::NewRequest => {
                self.new_request_tab(Item::new_request("Untitled Request", Request::default()), None);
            }
            Action::NewWs => {
                self.push_tab(TabKind::Ws(Box::new(WsTab::new())));
            }
            Action::NewCollection => {
                let c = Collection::new("New Collection");
                let id = c.id().to_string();
                self.collections.push(c);
                self.expanded.insert(id.clone());
                self.dirty_colls.insert(id.clone());
                self.sidebar = SidebarMode::Collections;
                self.dialog = Some(dialogs::Dialog::rename(Target::Collection(id), "New Collection"));
            }
            Action::NewEnvironment => {
                let e = Environment::new("New Environment");
                let id = e.id.clone();
                self.envs.push(e);
                self.dirty_envs.insert(id.clone());
                self.sidebar = SidebarMode::Environments;
                self.push_tab(TabKind::Env(EnvTab::new(&id)));
            }
            Action::AddRequest(cid, folder) => {
                let it = Item::new_request("New Request", Request::default());
                let iid = it.id().to_string();
                if let Some(c) = self.coll_mut(&cid) {
                    let path = folder.as_ref().and_then(|f| c.find_path_by_id(f)).unwrap_or_default();
                    if let Some(list) = c.children_mut(&path) {
                        list.push(it);
                    }
                }
                self.expanded.insert(cid.clone());
                if let Some(f) = folder {
                    self.expanded.insert(f);
                }
                self.dirty_colls.insert(cid.clone());
                self.open_item(&cid, &iid);
            }
            Action::AddFolder(cid, folder) => {
                let it = Item::new_folder("New Folder");
                let iid = it.id().to_string();
                if let Some(c) = self.coll_mut(&cid) {
                    let path = folder.as_ref().and_then(|f| c.find_path_by_id(f)).unwrap_or_default();
                    if let Some(list) = c.children_mut(&path) {
                        list.push(it);
                    }
                }
                self.expanded.insert(cid.clone());
                self.dirty_colls.insert(cid.clone());
                self.dialog = Some(dialogs::Dialog::rename(Target::Item(cid, iid), "New Folder"));
            }
            Action::Rename(t) => {
                let name = match &t {
                    Target::Collection(c) => self.coll(c).map(|c| c.info.name.clone()),
                    Target::Item(c, i) => self.coll(c).and_then(|c| c.find_path_by_id(i).and_then(|p| c.get(&p).map(|x| x.name.clone()))),
                }
                .unwrap_or_default();
                self.dialog = Some(dialogs::Dialog::rename(t, &name));
            }
            Action::Delete(t) => {
                let label = match &t {
                    Target::Collection(c) => format!("collection \"{}\"", self.coll(c).map(|c| c.info.name.clone()).unwrap_or_default()),
                    Target::Item(c, i) => format!("\"{}\"", self.coll(c).and_then(|c| c.find_path_by_id(i).and_then(|p| c.get(&p).map(|x| x.name.clone()))).unwrap_or_default()),
                };
                self.dialog = Some(dialogs::Dialog::ConfirmDelete { target: t, label });
            }
            Action::Duplicate(t) => self.duplicate(t),
            Action::Move(t, d) => {
                if let Target::Item(cid, iid) = t {
                    if let Some(c) = self.coll_mut(&cid) {
                        if let Some(p) = c.find_path_by_id(&iid) {
                            let (last, parent) = p.split_last().unwrap();
                            if let Some(list) = c.children_mut(parent) {
                                let ni = *last as i32 + d;
                                if ni >= 0 && (ni as usize) < list.len() {
                                    list.swap(*last, ni as usize);
                                }
                            }
                        }
                    }
                    self.dirty_colls.insert(cid);
                }
            }
            Action::DragMove { from, to, pos } => self.drag_move(from, to, pos),
            Action::Export(Target::Collection(cid)) => {
                if let Some(c) = self.coll(&cid) {
                    let json = serde_json::to_string_pretty(c).unwrap_or_default();
                    let name = format!("{}.postman_collection.json", crate::storage::safe_name(&c.info.name));
                    self.dialog = Some(dialogs::Dialog::export(name, json));
                }
            }
            Action::Export(_) => {}
            Action::Run(cid, folder) => {
                let mut rt = RunnerTab::new(&cid, folder.clone());
                if let Some(c) = self.coll(&cid) {
                    rt.init_selection(c);
                }
                self.push_tab(TabKind::Runner(Box::new(rt)));
            }
            Action::StartMock(cid) => self.dialog = Some(dialogs::Dialog::Mock { coll_id: cid, port: "3000".into(), delay: "0".into() }),
            Action::StopMock(i) => {
                if i < self.mocks.len() {
                    let m = self.mocks.remove(i);
                    m.2.stop();
                    self.toast(format!("Mock server for \"{}\" stopped", m.1));
                }
            }
            Action::OpenEnv(id) => {
                for (i, t) in self.tabs.iter().enumerate() {
                    if let TabKind::Env(e) = &t.kind {
                        if e.env_id == id {
                            self.active = i;
                            return;
                        }
                    }
                }
                self.push_tab(TabKind::Env(EnvTab::new(&id)));
            }
            Action::SetActiveEnv(id) => {
                self.active_env = id;
                self.store.save("state.json", &State { active_env: self.active_env.clone(), open_tabs: vec![], expanded: self.expanded.iter().cloned().collect() });
            }
            Action::DuplicateEnv(id) => {
                if let Some(e) = self.envs.iter().find(|e| e.id == id).cloned() {
                    let mut n = e;
                    n.id = crate::model::new_id();
                    n.name = format!("{} Copy", n.name);
                    self.dirty_envs.insert(n.id.clone());
                    self.envs.push(n);
                }
            }
            Action::DeleteEnv(id) => {
                self.envs.retain(|e| e.id != id);
                self.store.delete_environment(&id);
                if self.active_env.as_deref() == Some(&id) {
                    self.active_env = None;
                }
                self.tabs.retain(|t| !matches!(&t.kind, TabKind::Env(e) if e.env_id == id));
                self.active = self.active.min(self.tabs.len().saturating_sub(1));
            }
            Action::ExportEnv(id) => {
                let e = if id == tabs::GLOBALS_ID { Some(self.globals.clone()) } else { self.envs.iter().find(|e| e.id == id).cloned() };
                if let Some(e) = e {
                    let name = format!("{}.postman_{}.json", crate::storage::safe_name(&e.name), if id == tabs::GLOBALS_ID { "globals" } else { "environment" });
                    self.dialog = Some(dialogs::Dialog::export(name, serde_json::to_string_pretty(&e).unwrap_or_default()));
                }
            }
            Action::OpenHistory(i) => {
                if let Some(h) = self.history.get(i) {
                    let name = if h.name.is_empty() || h.name == "Untitled Request" { h.url.clone() } else { h.name.clone() };
                    let item = Item::new_request(&name, h.request.clone());
                    self.new_request_tab(item, None);
                }
            }
            Action::DeleteHistory(i) => {
                if i < self.history.len() {
                    self.history.remove(i);
                    self.dirty_history = true;
                }
            }
            Action::ClearHistory => {
                self.history.clear();
                self.dirty_history = true;
            }
            Action::SaveHistoryTo(i) => {
                if let Some(h) = self.history.get(i) {
                    let name = if h.name.is_empty() || h.name == "Untitled Request" { h.url.clone() } else { h.name.clone() };
                    let id = self.new_request_tab(Item::new_request(&name, h.request.clone()), None);
                    let ti = self.tab_index(id).unwrap();
                    self.dialog = Some(dialogs::Dialog::save_as(id, &self.tabs[ti]));
                }
            }
            Action::Send(id) => self.send(id, ctx),
            Action::Cancel(id) => {
                if let Some(i) = self.tab_index(id) {
                    if let TabKind::Request(r) = &mut self.tabs[i].kind {
                        r.inflight = None;
                    }
                }
            }
            Action::Save(id) => self.save_tab(id),
            Action::SaveAs(id) => {
                if let Some(i) = self.tab_index(id) {
                    self.dialog = Some(dialogs::Dialog::save_as(id, &self.tabs[i]));
                }
            }
            Action::CloseTab(id) => self.close_tab(id, false),
            Action::CloseOthers(id) => {
                let ids: Vec<u64> = self.tabs.iter().map(|t| t.id).filter(|t| *t != id).collect();
                for t in ids {
                    self.close_tab(t, true);
                }
            }
            Action::DuplicateTab(id) => {
                if let Some(i) = self.tab_index(id) {
                    if let TabKind::Request(r) = &self.tabs[i].kind {
                        let mut it = r.item.clone();
                        it.id = Some(crate::model::new_id());
                        it.name = format!("{} Copy", it.name);
                        self.new_request_tab(it, None);
                    }
                }
            }
            Action::SaveExample(id) => self.save_example(id),
            Action::Toast(s) => self.toast(s),
            Action::Error(s) => self.error(s),
            Action::Dialog(d) => self.dialog = Some(d),
            Action::Console(e) => self.log(e),
            Action::CollChanged(id) => {
                self.dirty_colls.insert(id);
            }
            Action::EnvChanged(id) => {
                if id == tabs::GLOBALS_ID {
                    self.dirty_globals = true;
                } else {
                    self.dirty_envs.insert(id);
                }
            }
            Action::GlobalsChanged => self.dirty_globals = true,
            Action::ImportText(t) => self.import_text(&t),
            Action::ImportFiles(fs) => {
                for f in fs {
                    match std::fs::read_to_string(&f) {
                        Ok(t) => self.import_text(&t),
                        Err(e) => self.error(format!("Cannot read {f}: {e}")),
                    }
                }
            }
            Action::OpenCode(id) => self.dialog = Some(dialogs::Dialog::Code { tab: id, lang: 0 }),
            Action::FocusTab(id) => {
                if let Some(i) = self.tab_index(id) {
                    self.active = i;
                }
            }
            Action::GetOAuthToken(id, refresh) => {
                let Some(i) = self.tab_index(id) else { return };
                let coll_id = match &self.tabs[i].kind {
                    TabKind::Request(r) => r.origin.as_ref().map(|o| o.0.clone()),
                    _ => None,
                };
                let vars = self.var_ctx(coll_id.as_deref());
                let settings = self.settings.clone();
                let jar = self.jar.clone();
                if let TabKind::Request(r) = &mut self.tabs[i].kind {
                    let Some(a) = r.item.request.as_ref().and_then(|q| q.auth.clone()) else { return };
                    let mut a = crate::prepare::resolve_auth(&a, &vars);
                    if let Some(rt) = refresh {
                        a.set_str("grant_type", "refresh_token");
                        a.set_str("refreshToken", &rt);
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    let c2 = ctx.clone();
                    std::thread::spawn(move || {
                        let opts = crate::prepare::http_options(&settings, &serde_json::Map::new(), Some(jar));
                        let _ = tx.send(crate::auth::oauth2_get_token(&a, &opts));
                        c2.request_repaint();
                    });
                    r.oauth_busy = Some(rx);
                    r.oauth_msg = "Waiting for token…".into();
                }
            }
        }
    }

    fn close_tab(&mut self, id: u64, force: bool) {
        let Some(i) = self.tab_index(id) else { return };
        if !force {
            if let TabKind::Request(r) = &self.tabs[i].kind {
                if r.dirty && r.origin.is_some() {
                    self.dialog = Some(dialogs::Dialog::ConfirmClose { tab: id, name: r.item.name.clone() });
                    return;
                }
            }
        }
        if let TabKind::Ws(w) = &self.tabs[i].kind {
            if let Some(c) = &w.conn {
                let _ = c.tx.send(crate::ws::WsCmd::Close);
            }
        }
        if let TabKind::Runner(r) = &self.tabs[i].kind {
            r.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if i < self.active {
            self.active -= 1;
        }
    }

    fn duplicate(&mut self, t: Target) {
        match t {
            Target::Collection(cid) => {
                if let Some(c) = self.coll(&cid).cloned() {
                    let mut n = c;
                    n.info.postman_id = crate::model::new_id();
                    n.info.name = format!("{} Copy", n.info.name);
                    fn reid(items: &mut [Item]) {
                        for i in items {
                            i.id = Some(crate::model::new_id());
                            if let Some(ch) = &mut i.item {
                                reid(ch);
                            }
                        }
                    }
                    reid(&mut n.item);
                    self.dirty_colls.insert(n.id().to_string());
                    self.collections.push(n);
                }
            }
            Target::Item(cid, iid) => {
                if let Some(c) = self.coll_mut(&cid) {
                    if let Some(p) = c.find_path_by_id(&iid) {
                        if let Some(it) = c.get(&p).cloned() {
                            let mut n = it;
                            n.name = format!("{} Copy", n.name);
                            n.id = None;
                            n.ensure_ids();
                            fn reid(i: &mut Item) {
                                i.id = Some(crate::model::new_id());
                                if let Some(ch) = &mut i.item {
                                    for c in ch {
                                        reid(c);
                                    }
                                }
                            }
                            reid(&mut n);
                            let (last, parent) = p.split_last().unwrap();
                            if let Some(list) = c.children_mut(parent) {
                                list.insert(last + 1, n);
                            }
                        }
                    }
                }
                self.dirty_colls.insert(cid);
            }
        }
    }

    fn drag_move(&mut self, from: Target, to: Target, pos: DropPos) {
        let Target::Item(fc, fi) = from else { return };
        if from_is_ancestor(self, &fc, &fi, &to) {
            return;
        }
        let item = match self.coll_mut(&fc).and_then(|c| c.find_path_by_id(&fi).and_then(|p| c.remove(&p))) {
            Some(i) => i,
            None => return,
        };
        let (tc, inserted) = match &to {
            Target::Collection(tc) => {
                let ok = self.coll_mut(tc).map(|c| c.item.push(item.clone())).is_some();
                (tc.clone(), ok)
            }
            Target::Item(tc, ti) => {
                let mut ok = false;
                if let Some(c) = self.coll_mut(tc) {
                    if let Some(p) = c.find_path_by_id(ti) {
                        let target_is_folder = c.get(&p).map(|x| x.is_folder()).unwrap_or(false);
                        match pos {
                            DropPos::Into if target_is_folder => {
                                if let Some(l) = c.children_mut(&p) {
                                    l.push(item.clone());
                                    ok = true;
                                }
                            }
                            _ => {
                                let (last, parent) = p.split_last().unwrap();
                                if let Some(l) = c.children_mut(parent) {
                                    l.insert(*last, item.clone());
                                    ok = true;
                                }
                            }
                        }
                    }
                }
                (tc.clone(), ok)
            }
        };
        if !inserted {
            // put it back at the end of source collection
            if let Some(c) = self.coll_mut(&fc) {
                c.item.push(item);
            }
        }
        // update origin of open tabs if moved across collections
        if tc != fc {
            for t in &mut self.tabs {
                if let TabKind::Request(r) = &mut t.kind {
                    if r.origin.as_ref().map(|o| o.1 == fi).unwrap_or(false) {
                        r.origin = Some((tc.clone(), fi.clone()));
                    }
                }
            }
        }
        self.dirty_colls.insert(fc);
        self.dirty_colls.insert(tc);
    }

    fn save_example(&mut self, id: u64) {
        let Some(i) = self.tab_index(id) else { return };
        let (origin, ex) = match &self.tabs[i].kind {
            TabKind::Request(r) => {
                let Some(res) = r.result.as_ref().and_then(|x| x.exec.response.as_ref()) else { return };
                let lang = {
                    let ct = res.content_type();
                    if ct.contains("json") {
                        "json"
                    } else if ct.contains("xml") {
                        "xml"
                    } else if ct.contains("html") {
                        "html"
                    } else {
                        "text"
                    }
                };
                let ex = serde_json::json!({
                    "name": format!("{} {}", res.status, res.reason),
                    "originalRequest": r.item.request,
                    "status": res.reason,
                    "code": res.status,
                    "_postman_previewlanguage": lang,
                    "header": res.headers.iter().map(|(k, v)| serde_json::json!({"key": k, "value": v})).collect::<Vec<_>>(),
                    "cookie": [],
                    "body": res.text()
                });
                (r.origin.clone(), ex)
            }
            _ => return,
        };
        match origin {
            Some((cid, iid)) => {
                if let Some(c) = self.coll_mut(&cid) {
                    if let Some(p) = c.find_path_by_id(&iid) {
                        if let Some(it) = c.get_mut(&p) {
                            it.response.push(ex);
                        }
                    }
                }
                self.dirty_colls.insert(cid);
                self.toast("Response saved as example");
            }
            None => self.error("Save the request to a collection first"),
        }
    }

    fn import_text(&mut self, t: &str) {
        match crate::importer::import_text(t) {
            Ok(crate::importer::Imported::Collection(mut c)) => {
                if self.collections.iter().any(|x| x.id() == c.id()) {
                    // same id already exists: import as copy
                    c.info.postman_id = crate::model::new_id();
                }
                let name = c.info.name.clone();
                let id = c.id().to_string();
                self.collections.push(c);
                self.expanded.insert(id.clone());
                self.dirty_colls.insert(id);
                self.sidebar = SidebarMode::Collections;
                self.toast(format!("Imported collection \"{name}\""));
            }
            Ok(crate::importer::Imported::Environment(mut e)) => {
                if self.envs.iter().any(|x| x.id == e.id) {
                    e.id = crate::model::new_id();
                }
                if e.scope == "globals" {
                    for v in e.values {
                        self.globals.values.retain(|g| g.key != v.key);
                        self.globals.values.push(v);
                    }
                    self.dirty_globals = true;
                    self.toast("Imported globals");
                } else {
                    let name = e.name.clone();
                    self.dirty_envs.insert(e.id.clone());
                    self.envs.push(e);
                    self.sidebar = SidebarMode::Environments;
                    self.toast(format!("Imported environment \"{name}\""));
                }
            }
            Ok(crate::importer::Imported::Request(r)) => {
                let name = r.url.raw.clone();
                let mut rt = ReqTab::new(Item::new_request(&name, r), None);
                rt.dirty = true;
                self.push_tab(TabKind::Request(Box::new(rt)));
                self.toast("Imported request");
            }
            Err(e) => self.error(format!("Import failed: {e}")),
        }
    }

    // ------------------------------------------------------------ UI
    fn top_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("postmini").strong().color(Color32::from_rgb(0xff, 0x6c, 0x37)).size(16.0));
            ui.separator();
            if ui.button("☰").on_hover_text("Toggle sidebar (Ctrl+B)").clicked() {
                self.show_sidebar = !self.show_sidebar;
            }
            ui.menu_button("New", |ui| {
                if ui.button("HTTP Request   Ctrl+T").clicked() {
                    actions.push(Action::NewRequest);
                    ui.close();
                }
                if ui.button("WebSocket Request").clicked() {
                    actions.push(Action::NewWs);
                    ui.close();
                }
                if ui.button("Collection").clicked() {
                    actions.push(Action::NewCollection);
                    ui.close();
                }
                if ui.button("Environment").clicked() {
                    actions.push(Action::NewEnvironment);
                    ui.close();
                }
            });
            if ui.button("Import").on_hover_text("Import collections, environments, OpenAPI, cURL (Ctrl+O)").clicked() {
                actions.push(Action::Dialog(dialogs::Dialog::import()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings (Ctrl+,)").clicked() {
                    actions.push(Action::Dialog(dialogs::Dialog::Settings { draft: Box::new(self.settings.clone()), section: 0 }));
                }
                if ui.button("🍪").on_hover_text("Cookies").clicked() {
                    actions.push(Action::Dialog(dialogs::Dialog::cookies()));
                }
                let ql = ui.button("👁").on_hover_text("Environment quick look");
                if ql.clicked() {
                    self.quicklook = !self.quicklook;
                }
                let names: Vec<(String, String)> = self.envs.iter().map(|e| (e.id.clone(), e.name.clone())).collect();
                let cur = self.active_env().map(|e| e.name.clone()).unwrap_or_else(|| "No Environment".into());
                let mut sel = self.active_env.clone();
                egui::ComboBox::from_id_salt("envsel").width(180.0).selected_text(cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut sel, None, "No Environment");
                    for (id, n) in &names {
                        ui.selectable_value(&mut sel, Some(id.clone()), n);
                    }
                });
                if sel != self.active_env {
                    actions.push(Action::SetActiveEnv(sel));
                }
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        egui::ScrollArea::horizontal().id_salt("tabbar").max_height(30.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, t) in self.tabs.iter().enumerate() {
                    let (badge, title, dirty, color) = match &t.kind {
                        TabKind::Request(r) => {
                            let m = r.item.request.as_ref().map(|q| q.method.clone()).unwrap_or_else(|| "GET".into());
                            (widgets::short_method(&m), r.item.name.clone(), r.dirty, widgets::method_color(&m))
                        }
                        TabKind::Env(e) => {
                            let n = if e.env_id == tabs::GLOBALS_ID { "Globals".to_string() } else { self.envs.iter().find(|x| x.id == e.env_id).map(|x| x.name.clone()).unwrap_or_default() };
                            ("ENV".into(), n, false, Color32::from_rgb(0x9a, 0x9a, 0xff))
                        }
                        TabKind::Coll(c) => {
                            let n = self.coll(&c.coll_id).map(|col| match &c.item_id {
                                Some(f) => col.find_path_by_id(f).and_then(|p| col.get(&p).map(|x| x.name.clone())).unwrap_or_default(),
                                None => col.info.name.clone(),
                            });
                            (if c.item_id.is_some() { "DIR".into() } else { "COLL".into() }, n.unwrap_or_default(), false, Color32::from_rgb(0xc0, 0xa0, 0x60))
                        }
                        TabKind::Ws(w) => ("WS".into(), if w.name.is_empty() { w.url.clone() } else { w.name.clone() }, false, widgets::method_color("WS")),
                        TabKind::Runner(r) => ("RUN".into(), format!("Runner — {}", self.coll(&r.coll_id).map(|c| c.info.name.clone()).unwrap_or_default()), false, Color32::from_rgb(0xff, 0x6c, 0x37)),
                    };
                    let sel = i == self.active;
                    let frame = egui::Frame::NONE
                        .fill(if sel { ui.visuals().extreme_bg_color } else { ui.visuals().faint_bg_color })
                        .stroke(if sel { egui::Stroke::new(1.0, Color32::from_rgb(0xff, 0x6c, 0x37)) } else { egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color) })
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(6, 2));
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(badge).small().strong().color(color));
                            let short: String = if title.chars().count() > 28 { format!("{}…", title.chars().take(27).collect::<String>()) } else { title.clone() };
                            let r = ui.add(egui::Label::new(if dirty { RichText::new(format!("{short} ●")) } else { RichText::new(short) }).sense(egui::Sense::click())).on_hover_text(&title);
                            if r.clicked() {
                                actions.push(Action::FocusTab(t.id));
                            }
                            if r.middle_clicked() {
                                actions.push(Action::CloseTab(t.id));
                            }
                            r.context_menu(|ui| {
                                if ui.button("Close").clicked() {
                                    actions.push(Action::CloseTab(t.id));
                                    ui.close();
                                }
                                if ui.button("Close other tabs").clicked() {
                                    actions.push(Action::CloseOthers(t.id));
                                    ui.close();
                                }
                                if matches!(t.kind, TabKind::Request(_)) && ui.button("Duplicate tab").clicked() {
                                    actions.push(Action::DuplicateTab(t.id));
                                    ui.close();
                                }
                            });
                            if ui.add(egui::Button::new(RichText::new("×").small()).frame(false)).on_hover_text("Close (Ctrl+W)").clicked() {
                                actions.push(Action::CloseTab(t.id));
                            }
                        });
                    });
                }
                if ui.button("+").on_hover_text("New request (Ctrl+T)").clicked() {
                    actions.push(Action::NewRequest);
                }
            });
        });
    }

    fn console_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Console").strong());
            ui.add_sized([200.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut self.console_filter).hint_text("Filter"));
            if ui.button("Clear").clicked() {
                self.console.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("×").clicked() {
                    self.show_console = false;
                }
                ui.label(RichText::new(format!("{} entries", self.console.len())).small().weak());
            });
        });
        ui.separator();
        let f = self.console_filter.to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            for (i, e) in self.console.iter().enumerate() {
                if !f.is_empty() && !e.text.to_lowercase().contains(&f) {
                    continue;
                }
                let color = match e.level.as_str() {
                    "error" => Color32::from_rgb(0xec, 0x5b, 0x56),
                    "warn" => Color32::from_rgb(0xf0, 0xb0, 0x3e),
                    "info" => Color32::from_rgb(0x5a, 0xa9, 0xf5),
                    "net" => ui.visuals().strong_text_color(),
                    _ => ui.visuals().text_color(),
                };
                if e.level == "net" {
                    let first = e.text.lines().next().unwrap_or("").to_string();
                    egui::CollapsingHeader::new(RichText::new(format!("{}  {}", e.time, first)).monospace().color(color)).id_salt(("con", i)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(e.text.lines().skip(1).collect::<Vec<_>>().join("\n")).monospace().small()).selectable(true));
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&e.time).monospace().small().weak());
                        ui.add(egui::Label::new(RichText::new(&e.text).monospace().color(color)).selectable(true));
                    });
                }
            }
        });
    }

    fn quicklook_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let mut open = self.quicklook;
        egui::Window::new("Environment quick look").open(&mut open).default_width(460.0).collapsible(false).show(ctx, |ui| {
            let env_id = self.active_env.clone();
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.active_env().map(|e| e.name.clone()).unwrap_or_else(|| "No environment selected".into())).strong());
                if let Some(id) = &env_id {
                    if ui.small_button("Edit").clicked() {
                        actions.push(Action::OpenEnv(id.clone()));
                    }
                }
            });
            if let Some(id) = &env_id {
                if let Some(e) = self.envs.iter_mut().find(|e| &e.id == id) {
                    if tabs::quick_vars(ui, &mut e.values, "qle") {
                        actions.push(Action::EnvChanged(id.clone()));
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Globals").strong());
                if ui.small_button("Edit").clicked() {
                    actions.push(Action::OpenEnv(tabs::GLOBALS_ID.into()));
                }
            });
            if tabs::quick_vars(ui, &mut self.globals.values, "qlg") {
                actions.push(Action::GlobalsChanged);
            }
        });
        self.quicklook = open;
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let cur = self.tabs.get(self.active).map(|t| t.id);
        let sc = |k, m| KeyboardShortcut::new(m, k);
        ctx.input_mut(|i| {
            if i.consume_shortcut(&sc(Key::Enter, Modifiers::COMMAND)) {
                if let Some(id) = cur {
                    actions.push(Action::Send(id));
                }
            }
            if i.consume_shortcut(&sc(Key::S, Modifiers::COMMAND | Modifiers::SHIFT)) {
                if let Some(id) = cur {
                    actions.push(Action::SaveAs(id));
                }
            }
            if i.consume_shortcut(&sc(Key::S, Modifiers::COMMAND)) {
                if let Some(id) = cur {
                    actions.push(Action::Save(id));
                }
            }
            if i.consume_shortcut(&sc(Key::T, Modifiers::COMMAND)) || i.consume_shortcut(&sc(Key::N, Modifiers::COMMAND)) {
                actions.push(Action::NewRequest);
            }
            if i.consume_shortcut(&sc(Key::W, Modifiers::COMMAND)) {
                if let Some(id) = cur {
                    actions.push(Action::CloseTab(id));
                }
            }
            if i.consume_shortcut(&sc(Key::O, Modifiers::COMMAND)) {
                actions.push(Action::Dialog(dialogs::Dialog::import()));
            }
            if i.consume_shortcut(&sc(Key::Comma, Modifiers::COMMAND)) {
                actions.push(Action::Dialog(dialogs::Dialog::Settings { draft: Box::new(self.settings.clone()), section: 0 }));
            }
            if i.consume_shortcut(&sc(Key::B, Modifiers::COMMAND)) {
                self.show_sidebar = !self.show_sidebar;
            }
            if i.consume_shortcut(&sc(Key::C, Modifiers::COMMAND | Modifiers::ALT)) {
                self.show_console = !self.show_console;
            }
            if i.consume_shortcut(&sc(Key::Tab, Modifiers::COMMAND)) && !self.tabs.is_empty() {
                self.active = (self.active + 1) % self.tabs.len();
            }
            if i.consume_shortcut(&sc(Key::Tab, Modifiers::COMMAND | Modifiers::SHIFT)) && !self.tabs.is_empty() {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            }
            if i.consume_shortcut(&sc(Key::Plus, Modifiers::COMMAND)) || i.consume_shortcut(&sc(Key::Equals, Modifiers::COMMAND)) {
                self.settings.zoom = (self.settings.zoom + 0.1).min(3.0);
            }
            if i.consume_shortcut(&sc(Key::Minus, Modifiers::COMMAND)) {
                self.settings.zoom = (self.settings.zoom - 0.1).max(0.5);
            }
            if i.consume_shortcut(&sc(Key::Num0, Modifiers::COMMAND)) {
                self.settings.zoom = 1.0;
            }
        });
        // dropped files => import (except when they are dropped onto a file field)
        let dropped: Vec<String> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_string_lossy().into_owned()).filter(|p| !p.is_empty()).collect());
        if !dropped.is_empty() {
            actions.push(Action::ImportFiles(dropped));
        }
    }
}

fn from_is_ancestor(app: &App, fc: &str, fi: &str, to: &Target) -> bool {
    // prevent dropping a folder into itself/descendant
    let Target::Item(tc, ti) = to else { return false };
    if tc != fc {
        return false;
    }
    if ti == fi {
        return true;
    }
    if let Some(c) = app.coll(fc) {
        if let (Some(fp), Some(tp)) = (c.find_path_by_id(fi), c.find_path_by_id(ti)) {
            return tp.starts_with(&fp);
        }
    }
    false
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let frame_start = Instant::now();
        self.ui_inner(ui);
        if std::env::var_os("POSTMINI_FRAME_STATS").is_some() {
            eprintln!("frame {:.3} ms", frame_start.elapsed().as_secs_f64() * 1000.0);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for (_, _, m) in &self.mocks {
            m.stop();
        }
        self.save_state();
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_state();
    }
}

impl App {
    fn ui_inner(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.apply_style(&ctx);
        self.poll();
        let mut actions: Vec<Action> = Vec::new();
        self.shortcuts(&ctx, &mut actions);

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(3.0);
            self.top_bar(ui, &mut actions);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").exact_size(22.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if ui.selectable_label(self.show_console, "▤ Console").on_hover_text("Ctrl+Alt+C").clicked() {
                    self.show_console = !self.show_console;
                }
                let errs = self.console.iter().filter(|c| c.level == "error").count();
                if errs > 0 {
                    ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), format!("{errs} errors"));
                }
                if !self.mocks.is_empty() {
                    ui.label(RichText::new(format!("● {} mock server(s) running", self.mocks.len())).color(Color32::from_rgb(0x3c, 0xc4, 0x7c)).small());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("v{}  ·  {}", env!("CARGO_PKG_VERSION"), self.store.dir.display())).small().weak());
                });
            });
        });
        if self.show_console {
            egui::Panel::bottom("console").resizable(true).default_size(220.0).show(ui, |ui| self.console_panel(ui));
        }
        if self.show_sidebar {
            egui::Panel::left("sidebar").resizable(true).default_size(290.0).show(ui, |ui| {
                sidebar::show(self, ui, &mut actions);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.tab_bar(ui, &mut actions);
            ui.separator();
            if self.tabs.is_empty() {
                ui.centered_and_justified(|ui| {
                    if ui.button("Create a new request (Ctrl+T)").clicked() {
                        actions.push(Action::NewRequest);
                    }
                });
            } else {
                let i = self.active.min(self.tabs.len() - 1);
                tabs::show_tab(self, i, ui, &mut actions);
            }
        });

        if self.quicklook {
            self.quicklook_window(&ctx, &mut actions);
        }
        dialogs::show(self, &ctx, &mut actions);

        // toasts
        self.toasts.retain(|t| t.1.elapsed() < Duration::from_secs(if t.2 { 6 } else { 3 }));
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("toasts")).anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -30.0]).show(&ctx, |ui| {
                for (t, _, err) in &self.toasts {
                    egui::Frame::popup(ui.style()).fill(if *err { Color32::from_rgb(0x5a, 0x1d, 0x1d) } else { ui.visuals().window_fill }).show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.label(RichText::new(t).color(if *err { Color32::WHITE } else { ui.visuals().text_color() }));
                    });
                }
            });
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        for a in actions {
            self.handle(a, &ctx);
        }
        // in-flight requests: animate spinner at a low frame rate only while waiting
        let busy = self.tabs.iter().any(|t| match &t.kind {
            TabKind::Request(r) => r.inflight.is_some() || r.oauth_busy.is_some(),
            TabKind::Runner(r) => r.rx.is_some(),
            _ => false,
        });
        if busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if !self.dirty_colls.is_empty() || !self.dirty_envs.is_empty() || self.dirty_globals || self.dirty_history {
            self.save_all(false);
            ctx.request_repaint_after(Duration::from_millis(900));
        }
    }

}
