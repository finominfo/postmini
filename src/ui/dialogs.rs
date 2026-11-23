use std::sync::mpsc::Receiver;

use egui::{Color32, RichText};

use super::{widgets, Action, App, Tab, TabKind, Target};
use crate::settings::Settings;

pub enum Dialog {
    Rename { target: Target, name: String },
    SaveAs { tab: u64, name: String, coll: Option<String>, folder: Option<String>, new_coll: String },
    Import { text: String, path: String, url: String, error: String, mode: u8, busy: Option<Receiver<Result<String, String>>> },
    Settings { draft: Box<Settings>, section: u8 },
    Code { tab: u64, lang: usize },
    Cookies { filter: String, domain: String, raw: String, error: String },
    ConfirmDelete { target: Target, label: String },
    ConfirmClose { tab: u64, name: String },
    Mock { coll_id: String, port: String, delay: String },
    Text { title: String, text: String },
    Export { name: String, data: Vec<u8>, path: String, msg: String },
}

impl Dialog {
    pub fn rename(t: Target, name: &str) -> Self {
        Dialog::Rename { target: t, name: name.to_string() }
    }
    pub fn import() -> Self {
        Dialog::Import { text: String::new(), path: String::new(), url: String::new(), error: String::new(), mode: 0, busy: None }
    }
    pub fn cookies() -> Self {
        Dialog::Cookies { filter: String::new(), domain: String::new(), raw: String::new(), error: String::new() }
    }
    pub fn save_as(tab: u64, t: &Tab) -> Self {
        let (name, coll) = match &t.kind {
            TabKind::Request(r) => (r.item.name.clone(), r.origin.as_ref().map(|o| o.0.clone())),
            _ => (String::new(), None),
        };
        Dialog::SaveAs { tab, name, coll, folder: None, new_coll: String::new() }
    }
    pub fn export(name: String, json: String) -> Self {
        Self::save_bytes(name, json.into_bytes())
    }
    pub fn save_bytes(name: String, data: Vec<u8>) -> Self {
        let dir = dirs::download_dir().or_else(dirs::document_dir).or_else(dirs::home_dir).unwrap_or_default();
        let path = dir.join(&name).to_string_lossy().into_owned();
        Dialog::Export { name, data, path, msg: String::new() }
    }
}

fn modal(ctx: &egui::Context, title: &str, width: f32, open: &mut bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title).collapsible(false).resizable(true).default_width(width).anchor(egui::Align2::CENTER_TOP, [0.0, 60.0]).open(open).show(ctx, add);
}

pub fn show(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let Some(mut d) = app.dialog.take() else { return };
    let mut open = true;
    let mut close = false;
    match &mut d {
        Dialog::Rename { target, name } => {
            modal(ctx, "Rename", 360.0, &mut open, |ui| {
                let r = ui.add_sized([320.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(name));
                r.request_focus();
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() || enter {
                        match target {
                            Target::Collection(c) => {
                                if let Some(col) = app.coll_mut(c) {
                                    col.info.name = name.clone();
                                }
                                actions.push(Action::CollChanged(c.clone()));
                            }
                            Target::Item(c, i) => {
                                if let Some(col) = app.coll_mut(c) {
                                    if let Some(p) = col.find_path_by_id(i) {
                                        if let Some(it) = col.get_mut(&p) {
                                            it.name = name.clone();
                                        }
                                    }
                                }
                                for t in &mut app.tabs {
                                    if let TabKind::Request(r) = &mut t.kind {
                                        if r.origin.as_ref().map(|o| &o.1 == i).unwrap_or(false) {
                                            r.item.name = name.clone();
                                        }
                                    }
                                }
                                actions.push(Action::CollChanged(c.clone()));
                            }
                        }
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        }
        Dialog::ConfirmDelete { target, label } => {
            modal(ctx, "Delete", 380.0, &mut open, |ui| {
                ui.label(format!("Are you sure you want to delete {label}?"));
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Delete").color(Color32::from_rgb(0xec, 0x5b, 0x56))).clicked() {
                        match target {
                            Target::Collection(c) => {
                                app.collections.retain(|x| x.id() != c);
                                app.store.delete_collection(c);
                                app.dirty_colls.remove(c.as_str());
                                for t in &mut app.tabs {
                                    if let TabKind::Request(r) = &mut t.kind {
                                        if r.origin.as_ref().map(|o| &o.0 == c).unwrap_or(false) {
                                            r.origin = None;
                                        }
                                    }
                                }
                                let cc = c.clone();
                                app.tabs.retain(|t| !matches!(&t.kind, TabKind::Coll(x) if x.coll_id == cc) && !matches!(&t.kind, TabKind::Runner(x) if x.coll_id == cc));
                                app.active = app.active.min(app.tabs.len().saturating_sub(1));
                            }
                            Target::Item(c, i) => {
                                if let Some(col) = app.coll_mut(c) {
                                    if let Some(p) = col.find_path_by_id(i) {
                                        col.remove(&p);
                                    }
                                }
                                for t in &mut app.tabs {
                                    if let TabKind::Request(r) = &mut t.kind {
                                        if r.origin.as_ref().map(|o| &o.1 == i).unwrap_or(false) {
                                            r.origin = None;
                                        }
                                    }
                                }
                                actions.push(Action::CollChanged(c.clone()));
                            }
                        }
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        }
        Dialog::ConfirmClose { tab, name } => {
            modal(ctx, "Unsaved changes", 380.0, &mut open, |ui| {
                ui.label(format!("\"{name}\" has unsaved changes. Save before closing?"));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        actions.push(Action::Save(*tab));
                        actions.push(Action::CloseTab(*tab));
                        close = true;
                    }
                    if ui.button("Don't save").clicked() {
                        if let Some(i) = app.tab_index(*tab) {
                            if let TabKind::Request(r) = &mut app.tabs[i].kind {
                                r.dirty = false;
                            }
                        }
                        actions.push(Action::CloseTab(*tab));
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        }
        Dialog::SaveAs { tab, name, coll, folder, new_coll } => {
            modal(ctx, "Save request", 460.0, &mut open, |ui| {
                ui.label("Request name");
                ui.add_sized([420.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(name));
                ui.add_space(6.0);
                ui.label("Save to");
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for c in &app.collections {
                        let sel = coll.as_deref() == Some(c.id()) && folder.is_none();
                        if ui.selectable_label(sel, format!("🗀 {}", c.info.name)).clicked() {
                            *coll = Some(c.id().to_string());
                            *folder = None;
                        }
                        fn folders(ui: &mut egui::Ui, cid: &str, items: &[crate::model::Item], depth: usize, coll: &mut Option<String>, folder: &mut Option<String>) {
                            for it in items {
                                if it.is_folder() {
                                    let sel = folder.as_deref() == Some(it.id());
                                    ui.horizontal(|ui| {
                                        ui.add_space(depth as f32 * 16.0);
                                        if ui.selectable_label(sel, format!("🗀 {}", it.name)).clicked() {
                                            *coll = Some(cid.to_string());
                                            *folder = Some(it.id().to_string());
                                        }
                                    });
                                    if let Some(ch) = &it.item {
                                        folders(ui, cid, ch, depth + 1, coll, folder);
                                    }
                                }
                            }
                        }
                        folders(ui, c.id(), &c.item, 1, coll, folder);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(new_coll).hint_text("New collection name"));
                    if ui.button("+ Create collection").clicked() && !new_coll.trim().is_empty() {
                        let c = crate::model::Collection::new(new_coll.trim());
                        *coll = Some(c.id().to_string());
                        *folder = None;
                        app.dirty_colls.insert(c.id().to_string());
                        app.collections.push(c);
                        new_coll.clear();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.add_enabled(coll.is_some(), egui::Button::new(RichText::new("Save").strong())).clicked() {
                        let cid = coll.clone().unwrap();
                        if let Some(i) = app.tab_index(*tab) {
                            if let TabKind::Request(r) = &mut app.tabs[i].kind {
                                let mut item = r.item.clone();
                                item.name = name.clone();
                                item.id = Some(crate::model::new_id());
                                let iid = item.id().to_string();
                                if let Some(c) = app.collections.iter_mut().find(|c| c.id() == cid) {
                                    let path = folder.as_ref().and_then(|f| c.find_path_by_id(f)).unwrap_or_default();
                                    if let Some(l) = c.children_mut(&path) {
                                        l.push(item.clone());
                                    }
                                }
                                r.item = item;
                                r.origin = Some((cid.clone(), iid));
                                r.dirty = false;
                                app.dirty_colls.insert(cid.clone());
                                app.expanded.insert(cid.clone());
                                if let Some(f) = folder {
                                    app.expanded.insert(f.clone());
                                }
                                actions.push(Action::Toast("Saved".into()));
                            }
                        }
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        }
        Dialog::Import { text, path, url, error, mode, busy } => {
            if let Some(rx) = busy {
                if let Ok(r) = rx.try_recv() {
                    *busy = None;
                    match r {
                        Ok(t) => {
                            actions.push(Action::ImportText(t));
                            close = true;
                        }
                        Err(e) => *error = e,
                    }
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
            modal(ctx, "Import", 620.0, &mut open, |ui| {
                ui.label(RichText::new("Postman collections (v1, v2.0, v2.1), environments, globals, OpenAPI 3 / Swagger 2 (JSON or YAML) and cURL commands. You can also drag & drop files onto the window.").small().weak());
                ui.horizontal(|ui| {
                    for (m, l) in [(0u8, "File"), (1, "Raw text / cURL"), (2, "Link")] {
                        if ui.selectable_label(*mode == m, l).clicked() {
                            *mode = m;
                        }
                    }
                });
                ui.separator();
                match mode {
                    0 => {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(path).hint_text("path to file(s), separated by ;").desired_width(440.0));
                            if super::files::can_browse() && ui.button("Upload Files").clicked() {
                                let fs = super::files::pick_files(&[("Postman / OpenAPI", &["json", "yaml", "yml"]), ("All", &["*"])]);
                                if !fs.is_empty() {
                                    actions.push(Action::ImportFiles(fs));
                                    close = true;
                                }
                            }
                        });
                        if ui.button("Import").clicked() {
                            actions.push(Action::ImportFiles(path.split(';').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect()));
                            close = true;
                        }
                    }
                    1 => {
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(text).code_editor().desired_rows(14).desired_width(f32::INFINITY).hint_text("Paste cURL, JSON or YAML here"));
                        });
                        if ui.button("Import").clicked() {
                            actions.push(Action::ImportText(text.clone()));
                            close = true;
                        }
                    }
                    _ => {
                        ui.add_sized([560.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(url).hint_text("https://example.com/openapi.json"));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(busy.is_none(), egui::Button::new("Fetch & import")).clicked() {
                                let (tx, rx) = std::sync::mpsc::channel();
                                let u = url.clone();
                                let opts = crate::prepare::http_options(&app.settings, &serde_json::Map::new(), None);
                                let c2 = ctx.clone();
                                std::thread::spawn(move || {
                                    let req = crate::http::HttpRequest { method: "GET".into(), url: u, headers: vec![("Accept".into(), "application/json, application/yaml, */*".into()), ("Accept-Encoding".into(), "gzip, deflate, br".into())], body: vec![] };
                                    let r = crate::http::execute(&req, &opts).and_then(|r| if r.status < 400 { Ok(r.text()) } else { Err(format!("HTTP {}", r.status)) });
                                    let _ = tx.send(r);
                                    c2.request_repaint();
                                });
                                *busy = Some(rx);
                            }
                            if busy.is_some() {
                                ui.spinner();
                            }
                        });
                    }
                }
                if !error.is_empty() {
                    ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), error.as_str());
                }
            });
        }
        Dialog::Code { tab, lang } => {
            let input = app.tab_index(*tab).and_then(|i| match &app.tabs[i].kind {
                TabKind::Request(r) => {
                    let coll_id = r.origin.as_ref().map(|o| o.0.clone());
                    let vars = app.var_ctx(coll_id.as_deref());
                    let req = r.item.request.clone().unwrap_or_default();
                    let auth = match (&req.auth, &r.origin) {
                        (Some(a), _) if a.kind != "inherit" => Some(a.clone()),
                        (_, Some((cid, iid))) => app.coll(cid).and_then(|c| c.find_path_by_id(iid).and_then(|p| c.effective_auth(&p, None))),
                        _ => None,
                    };
                    Some(crate::codegen::input_for(&req, auth.as_ref(), &vars, &app.settings))
                }
                _ => None,
            });
            let dark = app.settings.dark;
            modal(ctx, "Code snippet", 720.0, &mut open, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("codelang").width(240.0).selected_text(crate::codegen::LANGS[*lang]).show_ui(ui, |ui| {
                        for (i, l) in crate::codegen::LANGS.iter().enumerate() {
                            ui.selectable_value(lang, i, *l);
                        }
                    });
                });
                match &input {
                    Some(Ok(inp)) => {
                        let code = crate::codegen::generate(crate::codegen::LANGS[*lang], inp);
                        if ui.button("📋 Copy").clicked() {
                            ui.ctx().copy_text(code.clone());
                            actions.push(Action::Toast("Copied to clipboard".into()));
                        }
                        egui::ScrollArea::both().max_height(480.0).show(ui, |ui| {
                            let idx = widgets::TextIndex::new(code, widgets::Lang::Plain);
                            for i in 0..idx.len() {
                                ui.add(egui::Label::new(RichText::new(idx.line(i)).monospace()).selectable(true).extend());
                            }
                            let _ = dark;
                        });
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), e);
                    }
                    None => {
                        ui.label("Request tab not found");
                    }
                }
            });
        }
        Dialog::Cookies { filter, domain, raw, error } => {
            modal(ctx, "Manage Cookies", 720.0, &mut open, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(filter).hint_text("🔍 Filter domain / name"));
                    if ui.button("Clear all cookies").clicked() {
                        crate::http::clear_cookies(&app.jar, None);
                    }
                });
                ui.separator();
                let list = crate::http::list_cookies(&app.jar);
                let f = filter.to_lowercase();
                let mut domains: Vec<String> = list.iter().map(|c| c.domain.clone()).collect();
                domains.sort();
                domains.dedup();
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    if list.is_empty() {
                        ui.label(RichText::new("The cookie jar is empty. Cookies set by servers are stored here automatically.").weak());
                    }
                    for d in domains {
                        let items: Vec<&crate::http::CookieInfo> = list.iter().filter(|c| c.domain == d && (f.is_empty() || d.contains(&f) || c.name.to_lowercase().contains(&f))).collect();
                        if items.is_empty() {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&d).strong());
                            if ui.small_button("Clear domain").clicked() {
                                crate::http::clear_cookies(&app.jar, Some(&d));
                            }
                            if ui.small_button("+ Add").clicked() {
                                *domain = d.clone();
                            }
                        });
                        for c in items {
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                ui.label(RichText::new(format!("{}={}", c.name, crate::runner::truncate(&c.value, 60))).monospace());
                                ui.label(RichText::new(format!("Path={} · Expires={}{}{}", c.path, c.expires, if c.http_only { " · HttpOnly" } else { "" }, if c.secure { " · Secure" } else { "" })).small().weak());
                                if ui.small_button("🗑").clicked() {
                                    crate::http::remove_cookie(&app.jar, &c.domain, &c.path, &c.name);
                                }
                            });
                        }
                    }
                });
                ui.separator();
                ui.label(RichText::new("Add cookie").strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(domain).hint_text("domain (example.com)").desired_width(180.0));
                    ui.add_sized([360.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(raw).hint_text("name=value; Path=/; Expires=…"));
                    if ui.button("Save").clicked() {
                        match crate::http::add_cookie(&app.jar, domain, raw) {
                            Ok(_) => {
                                raw.clear();
                                error.clear();
                            }
                            Err(e) => *error = e,
                        }
                    }
                });
                if !error.is_empty() {
                    ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), error.as_str());
                }
            });
            if !open {
                app.store.save_jar(&app.jar);
            }
        }
        Dialog::Mock { coll_id, port, delay } => {
            modal(ctx, "Start mock server", 380.0, &mut open, |ui| {
                let n: usize = app.coll(coll_id).map(|c| c.requests().iter().map(|(_, i)| i.response.len()).sum()).unwrap_or(0);
                ui.label(format!("The mock server will serve the {n} saved example(s) of this collection on localhost."));
                if n == 0 {
                    ui.colored_label(Color32::from_rgb(0xf0, 0xb0, 0x3e), "This collection has no saved examples. Send a request and use \"Save Response → Save as example\" first.");
                }
                egui::Grid::new("mockg").show(ui, |ui| {
                    ui.label("Port");
                    ui.add_sized([80.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(port));
                    ui.end_row();
                    ui.label("Simulated delay (ms)");
                    ui.add_sized([80.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(delay));
                    ui.end_row();
                });
                if ui.button("Start").clicked() {
                    if let Some(c) = app.coll(coll_id) {
                        match crate::mock::start(c, port.trim().parse().unwrap_or(0), delay.trim().parse().unwrap_or(0)) {
                            Ok(m) => {
                                let name = c.info.name.clone();
                                actions.push(Action::Toast(format!("Mock server running at {}", m.url())));
                                // set {{url}} hint in console
                                actions.push(Action::Console(crate::runner::ConsoleEntry::new("info", format!("Mock server for \"{name}\" listening on {}", m.url()))));
                                app.mocks.push((coll_id.clone(), name, m));
                                app.sidebar = super::SidebarMode::Mocks;
                                close = true;
                            }
                            Err(e) => actions.push(Action::Error(e)),
                        }
                    }
                }
            });
        }
        Dialog::Text { title, text } => {
            let t = title.clone();
            modal(ctx, &t, 640.0, &mut open, |ui| {
                if ui.button("📋 Copy").clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                egui::ScrollArea::both().max_height(500.0).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text.as_str()).monospace()).selectable(true));
                });
            });
        }
        Dialog::Export { name, data, path, msg } => {
            modal(ctx, "Export", 560.0, &mut open, |ui| {
                ui.label(format!("{} ({})", name, crate::runner::human_size(data.len())));
                ui.horizontal(|ui| {
                    ui.add_sized([400.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(path));
                    if super::files::can_browse() && ui.button("Browse…").clicked() {
                        if let Some(p) = super::files::save_file(name, &[]) {
                            *path = p;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Save").strong()).clicked() {
                        match std::fs::write(path.as_str(), &data) {
                            Ok(_) => {
                                actions.push(Action::Toast(format!("Saved to {path}")));
                                close = true;
                            }
                            Err(e) => *msg = e.to_string(),
                        }
                    }
                    if ui.button("Copy to clipboard").clicked() {
                        ui.ctx().copy_text(String::from_utf8_lossy(data).into_owned());
                        actions.push(Action::Toast("Copied".into()));
                    }
                });
                if !msg.is_empty() {
                    ui.colored_label(Color32::from_rgb(0xec, 0x5b, 0x56), msg.as_str());
                }
            });
        }
        Dialog::Settings { draft, section } => {
            modal(ctx, "Settings", 640.0, &mut open, |ui| {
                ui.horizontal(|ui| {
                    for (i, l) in ["General", "Proxy", "Certificates", "Appearance", "Data"].iter().enumerate() {
                        if ui.selectable_label(*section == i as u8, *l).clicked() {
                            *section = i as u8;
                        }
                    }
                });
                ui.separator();
                let s = draft.as_mut();
                egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| match section {
                    0 => {
                        egui::Grid::new("sg").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            ui.label("Request timeout (ms, 0 = infinite)");
                            ui.add(egui::DragValue::new(&mut s.timeout_ms).range(0..=3_600_000));
                            ui.end_row();
                            ui.label("Max response size (MB, 0 = unlimited)");
                            ui.add(egui::DragValue::new(&mut s.max_response_mb).range(0..=10_000));
                            ui.end_row();
                            ui.label("Script timeout (ms)");
                            ui.add(egui::DragValue::new(&mut s.script_timeout_ms).range(0..=3_600_000));
                            ui.end_row();
                            ui.label("SSL certificate verification");
                            ui.checkbox(&mut s.verify_ssl, "");
                            ui.end_row();
                            ui.label("Automatically follow redirects");
                            ui.checkbox(&mut s.follow_redirects, "");
                            ui.end_row();
                            ui.label("Maximum redirects");
                            ui.add(egui::DragValue::new(&mut s.max_redirects).range(0..=100));
                            ui.end_row();
                            ui.label("Send no-cache header");
                            ui.checkbox(&mut s.send_no_cache, "");
                            ui.end_row();
                            ui.label("Send Postman-Token header");
                            ui.checkbox(&mut s.send_postman_token, "");
                            ui.end_row();
                            ui.label("Trim keys and values in request body / headers");
                            ui.checkbox(&mut s.trim_keys_values, "");
                            ui.end_row();
                            ui.label("Default User-Agent");
                            ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s.user_agent));
                            ui.end_row();
                            ui.label("Persist variable changes from scripts");
                            ui.checkbox(&mut s.auto_persist_vars, "");
                            ui.end_row();
                            ui.label("History limit");
                            ui.add(egui::DragValue::new(&mut s.history_limit).range(10..=10000));
                            ui.end_row();
                            ui.label("Working directory (relative file paths)");
                            ui.horizontal(|ui| {
                                ui.add_sized([240.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s.working_dir));
                                if super::files::can_browse() && ui.small_button("…").clicked() {
                                    if let Some(p) = super::files::pick_folder() {
                                        s.working_dir = p;
                                    }
                                }
                            });
                            ui.end_row();
                        });
                    }
                    1 => {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut s.proxy_mode, "none".to_string(), "No proxy");
                            ui.radio_value(&mut s.proxy_mode, "system".to_string(), "Use system proxy");
                            ui.radio_value(&mut s.proxy_mode, "custom".to_string(), "Custom proxy");
                        });
                        if s.proxy_mode == "system" {
                            let sp = crate::settings::system_proxy();
                            ui.label(RichText::new(match sp {
                                Some(p) => format!("Detected: {} (bypass: {})", p.url, p.bypass),
                                None => "No system proxy detected (HTTP_PROXY / HTTPS_PROXY / Windows Internet Settings)".into(),
                            }).small().weak());
                        }
                        if s.proxy_mode == "custom" {
                            egui::Grid::new("pg").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                                ui.label("Proxy URL");
                                ui.add_sized([320.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s.proxy.url).hint_text("http://user:pass@host:8080 or socks5://host:1080"));
                                ui.end_row();
                                ui.label("Use for");
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut s.proxy.http, "HTTP");
                                    ui.checkbox(&mut s.proxy.https, "HTTPS");
                                });
                                ui.end_row();
                                ui.label("Bypass (comma separated)");
                                ui.add_sized([320.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s.proxy.bypass));
                                ui.end_row();
                            });
                        }
                    }
                    2 => {
                        ui.label(RichText::new("CA Certificates").strong());
                        ui.horizontal(|ui| {
                            ui.add_sized([380.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut s.ca_file).hint_text("PEM file with extra trusted CAs"));
                            if super::files::can_browse() && ui.small_button("Select…").clicked() {
                                if let Some(p) = super::files::pick_file(&[("PEM", &["pem", "crt", "cer"])]) {
                                    s.ca_file = p;
                                }
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(RichText::new("Client Certificates (PEM)").strong());
                        let mut del = None;
                        for (i, c) in s.client_certs.iter_mut().enumerate() {
                            ui.group(|ui| {
                                egui::Grid::new(("cc", i)).num_columns(2).show(ui, |ui| {
                                    ui.label("Enabled");
                                    ui.checkbox(&mut c.enabled, "");
                                    ui.end_row();
                                    ui.label("Host (e.g. api.example.com:443, *.example.com)");
                                    ui.add_sized([260.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut c.host));
                                    ui.end_row();
                                    ui.label("CRT file");
                                    ui.horizontal(|ui| {
                                        ui.add_sized([220.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut c.cert_path));
                                        if super::files::can_browse() && ui.small_button("…").clicked() {
                                            if let Some(p) = super::files::pick_file(&[]) {
                                                c.cert_path = p;
                                            }
                                        }
                                    });
                                    ui.end_row();
                                    ui.label("KEY file");
                                    ui.horizontal(|ui| {
                                        ui.add_sized([220.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut c.key_path));
                                        if super::files::can_browse() && ui.small_button("…").clicked() {
                                            if let Some(p) = super::files::pick_file(&[]) {
                                                c.key_path = p;
                                            }
                                        }
                                    });
                                    ui.end_row();
                                });
                                if ui.small_button("Remove").clicked() {
                                    del = Some(i);
                                }
                            });
                        }
                        if let Some(i) = del {
                            s.client_certs.remove(i);
                        }
                        if ui.button("+ Add certificate").clicked() {
                            s.client_certs.push(crate::http::ClientCert { enabled: true, ..Default::default() });
                        }
                    }
                    3 => {
                        egui::Grid::new("ag").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            ui.label("Theme");
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut s.dark, true, "Dark");
                                ui.radio_value(&mut s.dark, false, "Light");
                            });
                            ui.end_row();
                            ui.label("Zoom");
                            ui.add(egui::Slider::new(&mut s.zoom, 0.6..=2.0));
                            ui.end_row();
                            ui.label("Font size");
                            ui.add(egui::Slider::new(&mut s.font_size, 10.0..=20.0));
                            ui.end_row();
                        });
                    }
                    _ => {
                        ui.label(format!("Data directory: {}", app.store.dir.display()));
                        ui.label(RichText::new("Collections and environments are stored as plain Postman v2.1 JSON files — you can version them with git or open them in Postman.\nPortable mode: create a folder named \"postmini-data\" next to postmini.exe.").small().weak());
                        if ui.button("Open data folder").clicked() {
                            super::files::open_folder(&app.store.dir);
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Save").strong()).clicked() {
                        app.settings = *draft.clone();
                        app.store.save_settings(&app.settings);
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Reset to defaults").clicked() {
                        **draft = Settings::default();
                    }
                });
            });
        }
    }
    if open && !close {
        app.dialog = Some(d);
    }
}
