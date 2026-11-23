use egui::{Color32, RichText, Ui};

use super::widgets::{self, status_color, Lang, TextIndex};
use super::{Action, BodyView, ReqTab, RespSection};
use crate::runner::human_size;

fn timing_row(ui: &mut Ui, label: &str, ms: f64, total: f64, offset: f64, color: Color32) {
    ui.label(RichText::new(label).small());
    let w = 160.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), egui::Sense::hover());
    let t = total.max(0.001);
    let x0 = rect.left() + (offset / t) as f32 * w;
    let x1 = (x0 + ((ms / t) as f32 * w).max(1.5)).min(rect.right());
    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())), 2.0, color);
    ui.label(RichText::new(format!("{ms:.2} ms")).small().monospace());
    ui.end_row();
}

pub fn show(ui: &mut Ui, tab: &mut ReqTab, tab_id: u64, dark: bool, actions: &mut Vec<Action>) {
    if let Some((_, start)) = &tab.inflight {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.spinner();
            ui.label(format!("Sending request… {:.1}s", start.elapsed().as_secs_f32()));
            if ui.button("Cancel").clicked() {
                actions.push(Action::Cancel(tab_id));
            }
        });
        return;
    }
    let Some(res) = tab.result.take() else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Response").strong());
            ui.label(RichText::new("Enter the URL and click Send to get a response").weak());
        });
        return;
    };
    show_result(ui, tab, &res, tab_id, dark, actions);
    tab.result = Some(res);
}

fn show_result(ui: &mut Ui, tab: &mut ReqTab, res: &super::SendResult, tab_id: u64, dark: bool, actions: &mut Vec<Action>) {
    let exec = &res.exec;
    let tests_total = exec.tests.len();
    let tests_pass = exec.passed();
    let has_vis = exec.visualizer.is_some();

    // ------------------------------------------------------------ header bar
    ui.horizontal(|ui| {
        let ncook = exec.response.as_ref().map(|r| r.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).count()).unwrap_or(0);
        let nh = exec.response.as_ref().map(|r| r.headers.len()).unwrap_or(0);
        let test_lbl = if tests_total > 0 { format!("Test Results ({tests_pass}/{tests_total})") } else { "Test Results".to_string() };
        let mut secs = vec![(RespSection::Body, "Body".to_string()), (RespSection::Cookies, if ncook > 0 { format!("Cookies ({ncook})") } else { "Cookies".into() }), (RespSection::Headers, if nh > 0 { format!("Headers ({nh})") } else { "Headers".into() }), (RespSection::Tests, test_lbl), (RespSection::Timeline, "Timeline".into())];
        if has_vis {
            secs.push((RespSection::Visualize, "Visualize".into()));
        }
        for (s, l) in secs {
            let mut t = RichText::new(l);
            if s == RespSection::Tests && tests_total > 0 {
                t = t.color(if tests_pass == tests_total { Color32::from_rgb(0x3c, 0xc4, 0x7c) } else { Color32::from_rgb(0xec, 0x5b, 0x56) });
            }
            if ui.selectable_label(tab.resp_section == s, t).clicked() {
                tab.resp_section = s;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(r) = &exec.response {
                ui.menu_button("Save Response ⏷", |ui| {
                    if ui.button("Save as example").clicked() {
                        actions.push(Action::SaveExample(tab_id));
                        ui.close();
                    }
                    if ui.button("Save response to file…").clicked() {
                        let ext = if r.content_type().contains("json") { "json" } else if r.content_type().contains("xml") { "xml" } else if r.content_type().contains("html") { "html" } else { "txt" };
                        actions.push(Action::Dialog(super::dialogs::Dialog::save_bytes(format!("response.{ext}"), r.body.clone())));
                        ui.close();
                    }
                    if ui.button("Copy body").clicked() {
                        ui.ctx().copy_text(r.text());
                        ui.close();
                    }
                });
                let size = r.size_total();
                ui.label(RichText::new(human_size(size)).color(Color32::from_rgb(0x3c, 0xc4, 0x7c))).on_hover_ui(|ui| {
                    egui::Grid::new("sizes").show(ui, |ui| {
                        ui.label("Response Size");
                        ui.label(human_size(size));
                        ui.end_row();
                        ui.label("  Headers");
                        ui.label(human_size(r.header_bytes));
                        ui.end_row();
                        ui.label("  Body (wire)");
                        ui.label(human_size(r.wire_body_bytes));
                        ui.end_row();
                        ui.label("  Body (uncompressed)");
                        ui.label(human_size(r.body.len()));
                        ui.end_row();
                        ui.label("Request body");
                        ui.label(human_size(r.sent.body_len));
                        ui.end_row();
                    });
                });
                let t = &r.timings;
                ui.label(RichText::new(format!("{:.0} ms", t.total_ms)).color(Color32::from_rgb(0x3c, 0xc4, 0x7c))).on_hover_ui(|ui| {
                    egui::Grid::new("timings").num_columns(3).show(ui, |ui| {
                        let total = t.total_ms;
                        let mut off = 0.0;
                        for (l, v, c) in [("DNS Lookup", t.dns_ms, Color32::from_rgb(0x5a, 0xa9, 0xf5)), ("TCP Handshake", t.tcp_ms, Color32::from_rgb(0xf0, 0xb0, 0x3e)), ("SSL Handshake", t.tls_ms, Color32::from_rgb(0xa2, 0x7b, 0xe6)), ("Transfer Start (TTFB)", t.ttfb_ms + t.request_ms, Color32::from_rgb(0x3c, 0xc4, 0x7c)), ("Download", t.download_ms, Color32::from_rgb(0xec, 0x5b, 0x56))] {
                            timing_row(ui, l, v, total, off, c);
                            off += v;
                        }
                        ui.label(RichText::new("Total").strong());
                        ui.label("");
                        ui.label(RichText::new(format!("{total:.2} ms")).strong());
                        ui.end_row();
                    });
                    ui.separator();
                    ui.label(RichText::new(format!("Remote: {}   Local: {}", r.remote_addr, r.local_addr)).small());
                    if let Some(tls) = &r.tls {
                        ui.label(RichText::new(format!("TLS: {tls}")).small());
                    }
                    ui.label(RichText::new(format!("HTTP/{}", r.version)).small());
                });
                ui.label(RichText::new(format!("{} {}", r.status, r.reason)).strong().color(status_color(r.status)));
                if r.truncated {
                    ui.colored_label(Color32::from_rgb(0xf0, 0xb0, 0x3e), "⚠ truncated").on_hover_text("Response exceeded the maximum response size (Settings)");
                }
            } else if exec.skipped {
                ui.label(RichText::new("Request skipped").weak());
            }
        });
    });
    ui.separator();

    if let Some(e) = &exec.error {
        ui.add_space(6.0);
        egui::Frame::group(ui.style()).fill(Color32::from_rgb(0x5a, 0x1d, 0x1d).gamma_multiply(if dark { 1.0 } else { 0.25 })).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Could not send request").strong().color(Color32::from_rgb(0xff, 0x9c, 0x9c)));
            ui.add(egui::Label::new(RichText::new(e).monospace()).selectable(true));
        });
        if exec.response.is_none() && tab.resp_section != RespSection::Tests {
            return;
        }
    }
    let Some(r) = exec.response.as_ref().or(None) else {
        if tab.resp_section == RespSection::Tests {
            tests_view(ui, exec);
        }
        return;
    };

    match tab.resp_section {
        RespSection::Body => {
            let ct = r.content_type();
            let is_image = ct.starts_with("image/");
            ui.horizontal(|ui| {
                for (v, l) in [(BodyView::Pretty, "Pretty"), (BodyView::Raw, "Raw"), (BodyView::Preview, "Preview")] {
                    if ui.selectable_label(tab.body_view == v, l).clicked() {
                        tab.body_view = v;
                        tab.resp_index = None;
                    }
                }
                ui.separator();
                ui.add_sized([180.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut tab.resp_filter).hint_text("🔍 Filter lines"));
                if ui.checkbox(&mut tab.wrap, "Wrap").changed() {}
                if ui.small_button("Copy").on_hover_text("Copy response body").clicked() {
                    ui.ctx().copy_text(r.text());
                }
            });
            if tab.body_view == BodyView::Preview {
                if is_image {
                    let uri = format!("bytes://resp-{}-{}.{}", tab_id, r.timings.total_ms as u64, ct.split('/').nth(1).unwrap_or("png").split(';').next().unwrap_or("png"));
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        ui.add(egui::Image::from_bytes(uri, r.body.clone()).max_width(ui.available_width()).fit_to_original_size(1.0));
                    });
                } else if ct.contains("html") {
                    let text = html_to_text(&r.text());
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.add(egui::Label::new(text).selectable(true).wrap());
                    });
                } else {
                    tab.body_view = BodyView::Pretty;
                }
                return;
            }
            if is_image && tab.body_view == BodyView::Pretty {
                tab.body_view = BodyView::Preview;
                return;
            }
            if tab.resp_index.is_none() {
                let text = r.text();
                let lang = widgets::lang_for_content_type(&ct, &text);
                let shown = if tab.body_view == BodyView::Pretty {
                    match lang {
                        Lang::Json => widgets::pretty_json(&text).unwrap_or(text),
                        Lang::Xml if text.len() < 2_000_000 => widgets::pretty_xml(&text),
                        _ => text,
                    }
                } else {
                    text
                };
                let lang = if tab.body_view == BodyView::Raw { Lang::Plain } else { lang };
                tab.resp_index = Some(TextIndex::new(shown, lang));
            }
            if let Some(idx) = &tab.resp_index {
                if idx.text.is_empty() {
                    ui.label(RichText::new("(empty body)").weak());
                } else {
                    widgets::virtual_text(ui, idx, dark, &tab.resp_filter, tab.wrap, None);
                }
            }
        }
        RespSection::Cookies => {
            let cookies = crate::script::parse_set_cookies(r);
            let list = cookies.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                ui.label(RichText::new("No cookies received from the server").weak());
            } else {
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("cookies").striped(true).num_columns(7).show(ui, |ui| {
                        for h in ["Name", "Value", "Domain", "Path", "Expires", "HttpOnly", "Secure"] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for c in list {
                            for k in ["name", "value", "domain", "path", "expires", "httpOnly", "secure"] {
                                let v = c.get(k).map(crate::model::value_to_string).unwrap_or_default();
                                ui.add(egui::Label::new(RichText::new(crate::runner::truncate(&v, 120)).monospace()).selectable(true));
                            }
                            ui.end_row();
                        }
                    });
                });
            }
        }
        RespSection::Headers => {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("rheaders").striped(true).num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new("Key").strong());
                    ui.label(RichText::new("Value").strong());
                    ui.end_row();
                    for (k, v) in &r.headers {
                        ui.add(egui::Label::new(RichText::new(k).monospace()).selectable(true));
                        ui.add(egui::Label::new(RichText::new(v).monospace()).selectable(true));
                        ui.end_row();
                    }
                });
            });
        }
        RespSection::Tests => tests_view(ui, exec),
        RespSection::Timeline => {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Request").strong());
                ui.add(egui::Label::new(RichText::new(format!("{} {}", r.sent.method, r.sent.url)).monospace()).selectable(true));
                for (k, v) in &r.sent.headers {
                    ui.add(egui::Label::new(RichText::new(format!("{k}: {v}")).monospace().small()).selectable(true));
                }
                if r.sent.body_len > 0 {
                    ui.label(RichText::new(format!("(body: {})", human_size(r.sent.body_len))).small().weak());
                }
                if !r.redirects.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Redirects").strong());
                    for (code, url) in &r.redirects {
                        ui.label(RichText::new(format!("{code} ← {url}")).monospace().small());
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Response  HTTP/{} {} {}", r.version, r.status, r.reason)).strong());
                for (k, v) in &r.headers {
                    ui.add(egui::Label::new(RichText::new(format!("{k}: {v}")).monospace().small()).selectable(true));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Network").strong());
                ui.label(RichText::new(format!("Remote address: {}   Local address: {}", r.remote_addr, r.local_addr)).small());
                if let Some(tls) = &r.tls {
                    ui.label(RichText::new(format!("TLS: {tls}")).small());
                }
                let t = &r.timings;
                ui.label(RichText::new(format!("DNS {:.2} ms · TCP {:.2} ms · TLS {:.2} ms · Request {:.2} ms · TTFB {:.2} ms · Download {:.2} ms · Total {:.2} ms", t.dns_ms, t.tcp_ms, t.tls_ms, t.request_ms, t.ttfb_ms, t.download_ms, t.total_ms)).small());
            });
        }
        RespSection::Visualize => {
            if let Some(v) = &exec.visualizer {
                ui.label(RichText::new("pm.visualizer output (postmini shows the template and data; HTML rendering is not available)").small().weak());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(RichText::new("Template").strong());
                    ui.add(egui::Label::new(RichText::new(v.get("template").map(crate::model::value_to_string).unwrap_or_default()).monospace()).selectable(true));
                    ui.label(RichText::new("Data").strong());
                    ui.add(egui::Label::new(RichText::new(serde_json::to_string_pretty(v.get("data").unwrap_or(&serde_json::Value::Null)).unwrap_or_default()).monospace()).selectable(true));
                });
            }
        }
    }
}

fn tests_view(ui: &mut Ui, exec: &crate::runner::Execution) {
    let id = ui.id().with("testfilter");
    let mut f: u8 = ui.data(|d| d.get_temp(id).unwrap_or(0));
    ui.horizontal(|ui| {
        for (v, l) in [(0u8, "All"), (1, "Passed"), (2, "Skipped"), (3, "Failed")] {
            if ui.selectable_label(f == v, l).clicked() {
                f = v;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(id, f));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for e in &exec.script_errors {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(" ERROR ").strong().background_color(Color32::from_rgb(0xec, 0x5b, 0x56)).color(Color32::WHITE));
                ui.add(egui::Label::new(RichText::new(e).monospace()).selectable(true));
            });
        }
        if exec.tests.is_empty() && exec.script_errors.is_empty() {
            ui.label(RichText::new("There are no tests for this request. Write a test script (Tests tab) to automate debugging.").weak());
        }
        for t in &exec.tests {
            let show = match f {
                1 => t.passed && !t.skipped,
                2 => t.skipped,
                3 => !t.passed && !t.skipped,
                _ => true,
            };
            if !show {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                let (lbl, col) = if t.skipped {
                    (" SKIP ", Color32::GRAY)
                } else if t.passed {
                    (" PASS ", Color32::from_rgb(0x2e, 0x9e, 0x62))
                } else {
                    (" FAIL ", Color32::from_rgb(0xd0, 0x45, 0x40))
                };
                ui.label(RichText::new(lbl).strong().small().background_color(col).color(Color32::WHITE));
                ui.label(&t.name);
                if let Some(e) = &t.error {
                    if !t.passed {
                        ui.add(egui::Label::new(RichText::new(format!("| {e}")).color(Color32::from_rgb(0xec, 0x7b, 0x76))).selectable(true));
                    }
                }
            });
        }
    });
}

pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    let mut skip = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag.to_lowercase();
                let name = t.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_string();
                if name == "script" || name == "style" {
                    skip = !t.starts_with('/');
                }
                if matches!(name.as_str(), "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "title" | "table") {
                    out.push('\n');
                }
            } else {
                tag.push(c);
            }
            continue;
        }
        if !skip {
            out.push(c);
        }
    }
    let out = out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    let mut lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}
