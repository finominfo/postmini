use egui::{Color32, RichText, Ui};

use super::widgets::{method_color, short_method};
use super::{Action, App, DropPos, SidebarMode, TabKind, Target};
use crate::model::Item;

#[derive(Clone)]
struct Drag(Target);

pub fn show(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (m, l) in [(SidebarMode::Collections, "Collections"), (SidebarMode::Environments, "Environments"), (SidebarMode::History, "History"), (SidebarMode::Mocks, "Mocks")] {
            if ui.selectable_label(app.sidebar == m, l).clicked() {
                app.sidebar = m;
            }
        }
    });
    ui.separator();
    match app.sidebar {
        SidebarMode::Collections => collections(app, ui, actions),
        SidebarMode::Environments => environments(app, ui, actions),
        SidebarMode::History => history(app, ui, actions),
        SidebarMode::Mocks => mocks(app, ui, actions),
    }
}

fn active_origin(app: &App) -> Option<(String, String)> {
    match app.tabs.get(app.active).map(|t| &t.kind) {
        Some(TabKind::Request(r)) => r.origin.clone(),
        Some(TabKind::Coll(c)) => c.item_id.clone().map(|i| (c.coll_id.clone(), i)),
        _ => None,
    }
}

fn matches_filter(it: &Item, f: &str) -> bool {
    if f.is_empty() {
        return true;
    }
    if it.name.to_lowercase().contains(f) {
        return true;
    }
    if let Some(r) = &it.request {
        if r.url.raw.to_lowercase().contains(f) {
            return true;
        }
    }
    it.item.as_ref().map(|ch| ch.iter().any(|c| matches_filter(c, f))).unwrap_or(false)
}

fn collections(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.button("+").on_hover_text("New collection").clicked() {
            actions.push(Action::NewCollection);
        }
        ui.add(egui::TextEdit::singleline(&mut app.filter).hint_text("🔍 Filter").desired_width(f32::INFINITY));
    });
    ui.add_space(2.0);
    let filter = app.filter.to_lowercase();
    let sel = active_origin(app);
    let running: Vec<String> = app.mocks.iter().map(|m| m.0.clone()).collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if app.collections.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No collections yet").weak());
                if ui.button("Create Collection").clicked() {
                    actions.push(Action::NewCollection);
                }
                if ui.button("Import…").clicked() {
                    actions.push(Action::Dialog(super::dialogs::Dialog::import()));
                }
            });
        }
        for c in &app.collections {
            let cid = c.id().to_string();
            if !filter.is_empty() && !c.info.name.to_lowercase().contains(&filter) && !c.item.iter().any(|i| matches_filter(i, &filter)) {
                continue;
            }
            let open = app.expanded.contains(&cid) || !filter.is_empty();
            let row = ui.horizontal(|ui| {
                let arrow = if open { "⏷" } else { "⏵" };
                if ui.add(egui::Button::new(arrow).frame(false)).clicked() {
                    actions.push(Action::Toggle(cid.clone()));
                }
                let mut t = RichText::new(format!("🗀 {}", c.info.name)).strong();
                if running.contains(&cid) {
                    t = RichText::new(format!("🗀 {}  ●", c.info.name)).strong().color(Color32::from_rgb(0x3c, 0xc4, 0x7c));
                }
                let r = ui.add(egui::Button::selectable(false, t).frame(false)).interact(egui::Sense::drag());
                r
            });
            let r = row.inner;
            if r.clicked() {
                actions.push(Action::Toggle(cid.clone()));
            }
            if r.double_clicked() {
                actions.push(Action::OpenCollSettings(cid.clone(), None));
            }
            drop_target(ui, &r, Target::Collection(cid.clone()), true, actions);
            r.context_menu(|ui| {
                if ui.button("Settings (auth, scripts, variables)").clicked() {
                    actions.push(Action::OpenCollSettings(cid.clone(), None));
                    ui.close();
                }
                if ui.button("Run collection").clicked() {
                    actions.push(Action::Run(cid.clone(), None));
                    ui.close();
                }
                ui.separator();
                if ui.button("Add request").clicked() {
                    actions.push(Action::AddRequest(cid.clone(), None));
                    ui.close();
                }
                if ui.button("Add folder").clicked() {
                    actions.push(Action::AddFolder(cid.clone(), None));
                    ui.close();
                }
                ui.separator();
                if running.contains(&cid) {
                    if ui.button("Stop mock server").clicked() {
                        if let Some(i) = app.mocks.iter().position(|m| m.0 == cid) {
                            actions.push(Action::StopMock(i));
                        }
                        ui.close();
                    }
                } else if ui.button("Start mock server…").clicked() {
                    actions.push(Action::StartMock(cid.clone()));
                    ui.close();
                }
                if ui.button("Export…").clicked() {
                    actions.push(Action::Export(Target::Collection(cid.clone())));
                    ui.close();
                }
                ui.separator();
                if ui.button("Rename").clicked() {
                    actions.push(Action::Rename(Target::Collection(cid.clone())));
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    actions.push(Action::Duplicate(Target::Collection(cid.clone())));
                    ui.close();
                }
                if ui.button(RichText::new("Delete").color(Color32::from_rgb(0xec, 0x5b, 0x56))).clicked() {
                    actions.push(Action::Delete(Target::Collection(cid.clone())));
                    ui.close();
                }
            });
            if open {
                if c.item.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(26.0);
                        if ui.link(RichText::new("This collection is empty. Add a request").small()).clicked() {
                            actions.push(Action::AddRequest(cid.clone(), None));
                        }
                    });
                }
                for (idx, it) in c.item.iter().enumerate() {
                    item_row(ui, &cid, it, 1, idx, c.item.len(), &filter, &sel, &app.expanded, actions);
                }
            }
        }
    });
}

fn drop_target(ui: &Ui, r: &egui::Response, target: Target, into: bool, actions: &mut Vec<Action>) {
    if let Some(p) = r.dnd_hover_payload::<Drag>() {
        if p.0 != target {
            let stroke = egui::Stroke::new(2.0, Color32::from_rgb(0xff, 0x6c, 0x37));
            if into {
                ui.painter().rect_stroke(r.rect.expand(1.0), 3.0, stroke, egui::StrokeKind::Outside);
            } else {
                ui.painter().hline(r.rect.x_range(), r.rect.top(), stroke);
            }
        }
    }
    if let Some(p) = r.dnd_release_payload::<Drag>() {
        if p.0 != target {
            actions.push(Action::DragMove { from: p.0.clone(), to: target, pos: if into { DropPos::Into } else { DropPos::Before } });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn item_row(ui: &mut Ui, cid: &str, it: &Item, depth: usize, idx: usize, n: usize, filter: &str, sel: &Option<(String, String)>, expanded: &std::collections::HashSet<String>, actions: &mut Vec<Action>) {
    if !matches_filter(it, filter) {
        return;
    }
    let iid = it.id().to_string();
    let target = Target::Item(cid.to_string(), iid.clone());
    let folder = it.is_folder();
    let open = expanded.contains(&iid) || !filter.is_empty();
    let selected = sel.as_ref().map(|s| s.0 == cid && s.1 == iid).unwrap_or(false);
    let row = ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        let r = if folder {
            let arrow = if open { "⏷" } else { "⏵" };
            ui.add(egui::Button::selectable(selected, format!("{arrow} 🗀 {}", it.name)))
        } else {
            let m = it.request.as_ref().map(|r| r.method.clone()).unwrap_or_else(|| "GET".into());
            ui.add_sized([34.0, 16.0], egui::Label::new(RichText::new(short_method(&m)).monospace().small().strong().color(method_color(&m))));
            ui.add(egui::Button::selectable(selected, it.name.clone())).on_hover_text(it.request.as_ref().map(|r| r.url.raw.clone()).unwrap_or_default())
        };
        r.interact(egui::Sense::drag())
    });
    let r = row.inner;
    if r.drag_started() {
        r.dnd_set_drag_payload(Drag(target.clone()));
    }
    if r.clicked() {
        if folder {
            actions.push(Action::Toggle(iid.clone()));
        } else {
            actions.push(Action::OpenItem(cid.to_string(), iid.clone()));
        }
    }
    if folder && r.double_clicked() {
        actions.push(Action::OpenCollSettings(cid.to_string(), Some(iid.clone())));
    }
    drop_target(ui, &r, target.clone(), folder, actions);
    r.context_menu(|ui| {
        if folder {
            if ui.button("Folder settings").clicked() {
                actions.push(Action::OpenCollSettings(cid.to_string(), Some(iid.clone())));
                ui.close();
            }
            if ui.button("Run folder").clicked() {
                actions.push(Action::Run(cid.to_string(), Some(iid.clone())));
                ui.close();
            }
            ui.separator();
            if ui.button("Add request").clicked() {
                actions.push(Action::AddRequest(cid.to_string(), Some(iid.clone())));
                ui.close();
            }
            if ui.button("Add folder").clicked() {
                actions.push(Action::AddFolder(cid.to_string(), Some(iid.clone())));
                ui.close();
            }
            ui.separator();
        } else {
            if ui.button("Open").clicked() {
                actions.push(Action::OpenItem(cid.to_string(), iid.clone()));
                ui.close();
            }
            if !it.response.is_empty() {
                ui.label(RichText::new(format!("{} saved example(s)", it.response.len())).small().weak());
            }
        }
        if ui.add_enabled(idx > 0, egui::Button::new("Move up")).clicked() {
            actions.push(Action::Move(target.clone(), -1));
            ui.close();
        }
        if ui.add_enabled(idx + 1 < n, egui::Button::new("Move down")).clicked() {
            actions.push(Action::Move(target.clone(), 1));
            ui.close();
        }
        if ui.button("Rename").clicked() {
            actions.push(Action::Rename(target.clone()));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            actions.push(Action::Duplicate(target.clone()));
            ui.close();
        }
        if ui.button(RichText::new("Delete").color(Color32::from_rgb(0xec, 0x5b, 0x56))).clicked() {
            actions.push(Action::Delete(target.clone()));
            ui.close();
        }
    });
    if folder && open {
        if let Some(ch) = &it.item {
            for (i, c) in ch.iter().enumerate() {
                item_row(ui, cid, c, depth + 1, i, ch.len(), filter, sel, expanded, actions);
            }
        }
    }
}

fn environments(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.button("+ New").clicked() {
            actions.push(Action::NewEnvironment);
        }
        if ui.button("Import…").clicked() {
            actions.push(Action::Dialog(super::dialogs::Dialog::import()));
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let r = ui.add(egui::Button::selectable(false, RichText::new(format!("🌐 Globals ({})", app.globals.values.iter().filter(|v| !v.key.is_empty()).count())).strong()).frame(false));
        if r.clicked() {
            actions.push(Action::OpenEnv(super::tabs::GLOBALS_ID.into()));
        }
        r.context_menu(|ui| {
            if ui.button("Export…").clicked() {
                actions.push(Action::ExportEnv(super::tabs::GLOBALS_ID.into()));
                ui.close();
            }
        });
        ui.separator();
        for e in &app.envs {
            let active = app.active_env.as_deref() == Some(&e.id);
            ui.horizontal(|ui| {
                let mut a = active;
                if ui.checkbox(&mut a, "").on_hover_text("Set as active environment").changed() {
                    actions.push(Action::SetActiveEnv(if a { Some(e.id.clone()) } else { None }));
                }
                let r = ui.add(egui::Button::selectable(active, format!("{} ({})", e.name, e.values.iter().filter(|v| !v.key.is_empty()).count())).frame(false));
                if r.clicked() {
                    actions.push(Action::OpenEnv(e.id.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Set active").clicked() {
                        actions.push(Action::SetActiveEnv(Some(e.id.clone())));
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        actions.push(Action::DuplicateEnv(e.id.clone()));
                        ui.close();
                    }
                    if ui.button("Export…").clicked() {
                        actions.push(Action::ExportEnv(e.id.clone()));
                        ui.close();
                    }
                    if ui.button(RichText::new("Delete").color(Color32::from_rgb(0xec, 0x5b, 0x56))).clicked() {
                        actions.push(Action::DeleteEnv(e.id.clone()));
                        ui.close();
                    }
                });
            });
        }
    });
}

fn history(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.add_sized([180.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut app.filter).hint_text("🔍 Filter"));
        if ui.button("Clear all").clicked() {
            actions.push(Action::ClearHistory);
        }
    });
    ui.separator();
    let f = app.filter.to_lowercase();
    let mut last_day = String::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, h) in app.history.iter().enumerate() {
            if !f.is_empty() && !h.url.to_lowercase().contains(&f) && !h.name.to_lowercase().contains(&f) {
                continue;
            }
            let day = h.time.get(..10).unwrap_or("").to_string();
            if day != last_day {
                ui.label(RichText::new(&day).small().strong().weak());
                last_day = day;
            }
            let r = ui
                .horizontal(|ui| {
                    ui.add_sized([34.0, 16.0], egui::Label::new(RichText::new(short_method(&h.method)).monospace().small().strong().color(method_color(&h.method))));
                    let r = ui.add(egui::Button::selectable(false, crate::runner::truncate(&h.url, 60)).frame(false)).on_hover_text(format!("{}\n{} · {} · {:.0} ms", h.url, h.time, h.status, h.time_ms));
                    if h.status > 0 {
                        ui.label(RichText::new(h.status.to_string()).small().color(super::widgets::status_color(h.status)));
                    }
                    r
                })
                .inner;
            if r.clicked() {
                actions.push(Action::OpenHistory(i));
            }
            r.context_menu(|ui| {
                if ui.button("Open in new tab").clicked() {
                    actions.push(Action::OpenHistory(i));
                    ui.close();
                }
                if ui.button("Save to collection…").clicked() {
                    actions.push(Action::SaveHistoryTo(i));
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    actions.push(Action::DeleteHistory(i));
                    ui.close();
                }
            });
        }
        if app.history.is_empty() {
            ui.label(RichText::new("Requests you send will appear here.").weak());
        }
    });
}

fn mocks(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Local mock servers serve the saved examples of a collection. Start one from a collection's context menu.").small().weak());
    ui.separator();
    if app.mocks.is_empty() {
        ui.label(RichText::new("No mock servers running.").weak());
    }
    for (i, (_, name, m)) in app.mocks.iter().enumerate() {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong());
                if ui.small_button("Stop").clicked() {
                    actions.push(Action::StopMock(i));
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(m.url()).monospace());
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(m.url());
                }
            });
            ui.label(RichText::new(format!("{} example routes", m.routes)).small().weak());
            egui::CollapsingHeader::new("Log").id_salt(("mocklog", i)).show(ui, |ui| {
                if let Ok(l) = m.log.lock() {
                    for e in l.iter().rev().take(100) {
                        ui.label(RichText::new(e).monospace().small());
                    }
                }
            });
        });
    }
}
