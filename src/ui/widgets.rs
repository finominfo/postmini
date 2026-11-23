use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, RichText, TextBuffer, Ui};

use crate::model::KV;
use crate::vars::{used_vars, VarContext};

pub fn method_color(m: &str) -> Color32 {
    match m {
        "GET" => Color32::from_rgb(0x2e, 0xb8, 0x72),
        "POST" => Color32::from_rgb(0xe6, 0xa1, 0x17),
        "PUT" => Color32::from_rgb(0x3d, 0x8b, 0xf2),
        "PATCH" => Color32::from_rgb(0xa2, 0x7b, 0xe6),
        "DELETE" => Color32::from_rgb(0xe8, 0x56, 0x4c),
        "HEAD" => Color32::from_rgb(0x2e, 0xb8, 0x72),
        "OPTIONS" => Color32::from_rgb(0xe0, 0x5d, 0xa8),
        "WS" => Color32::from_rgb(0x7c, 0x9c, 0xff),
        _ => Color32::GRAY,
    }
}

pub fn short_method(m: &str) -> String {
    match m {
        "DELETE" => "DEL".into(),
        "OPTIONS" => "OPT".into(),
        "PATCH" => "PATCH".into(),
        o => o.chars().take(5).collect(),
    }
}

pub fn status_color(code: u16) -> Color32 {
    match code {
        200..=299 => Color32::from_rgb(0x3c, 0xc4, 0x7c),
        300..=399 => Color32::from_rgb(0x4f, 0x9c, 0xf2),
        400..=499 => Color32::from_rgb(0xf0, 0x9a, 0x3e),
        _ => Color32::from_rgb(0xec, 0x5b, 0x56),
    }
}

pub struct Palette {
    pub key: Color32,
    pub string: Color32,
    pub number: Color32,
    pub kw: Color32,
    pub comment: Color32,
    pub punct: Color32,
    pub text: Color32,
    pub var_ok: Color32,
    pub var_bad: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            key: Color32::from_rgb(0x9c, 0xdc, 0xfe),
            string: Color32::from_rgb(0xce, 0x91, 0x78),
            number: Color32::from_rgb(0xb5, 0xce, 0xa8),
            kw: Color32::from_rgb(0x56, 0x9c, 0xd6),
            comment: Color32::from_rgb(0x6a, 0x99, 0x55),
            punct: Color32::from_rgb(0xb0, 0xb0, 0xb0),
            text: Color32::from_rgb(0xd4, 0xd4, 0xd4),
            var_ok: Color32::from_rgb(0x4e, 0xc9, 0x8a),
            var_bad: Color32::from_rgb(0xf1, 0x6b, 0x6b),
        }
    } else {
        Palette {
            key: Color32::from_rgb(0x00, 0x51, 0x9e),
            string: Color32::from_rgb(0xa3, 0x15, 0x15),
            number: Color32::from_rgb(0x09, 0x86, 0x58),
            kw: Color32::from_rgb(0x00, 0x00, 0xff),
            comment: Color32::from_rgb(0x00, 0x80, 0x00),
            punct: Color32::from_rgb(0x50, 0x50, 0x50),
            text: Color32::from_rgb(0x20, 0x20, 0x20),
            var_ok: Color32::from_rgb(0x0a, 0x8a, 0x4a),
            var_bad: Color32::from_rgb(0xd0, 0x20, 0x20),
        }
    }
}

fn fmt(font: &FontId, c: Color32) -> TextFormat {
    TextFormat { font_id: font.clone(), color: c, ..Default::default() }
}

/// Layout job coloring {{variables}}
pub fn var_job(text: &str, vars: &VarContext, font: FontId, dark: bool, wrap: f32) -> LayoutJob {
    let p = palette(dark);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut last = 0;
    for (s, e, name) in used_vars(text) {
        if s > last {
            job.append(&text[last..s], 0.0, fmt(&font, p.text));
        }
        let ok = vars.lookup(&name).is_some();
        let mut f = fmt(&font, if ok { p.var_ok } else { p.var_bad });
        f.background = if ok { p.var_ok.gamma_multiply(0.12) } else { p.var_bad.gamma_multiply(0.12) };
        job.append(&text[s..e], 0.0, f);
        last = e;
    }
    if last < text.len() {
        job.append(&text[last..], 0.0, fmt(&font, p.text));
    }
    job
}

/// Single line text edit with variable highlighting and resolved-value tooltip.
pub fn var_edit(ui: &mut Ui, text: &mut String, vars: &VarContext, hint: &str, width: f32, dark: bool) -> egui::Response {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, wrap: f32| -> Arc<Galley> {
        let job = var_job(buf.as_str(), vars, font.clone(), dark, wrap);
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    let resp = ui.add_sized([width.max(40.0), ui.spacing().interact_size.y], egui::TextEdit::singleline(text).hint_text(hint).font(egui::TextStyle::Monospace).layouter(&mut layouter));
    if text.contains("{{") {
        let resolved = vars.resolve(text);
        let names: Vec<String> = used_vars(text).into_iter().map(|(_, _, n)| n).collect();
        resp.clone().on_hover_ui(|ui| {
            for n in names.iter().take(10) {
                let src = vars.source_of(n);
                let val = vars.lookup(n);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{{{{{n}}}}}")).monospace().strong());
                    match (src, val) {
                        (Some(s), Some(v)) => {
                            ui.label(RichText::new(s).small().weak());
                            ui.label(RichText::new(crate::runner::truncate(&v, 200)).monospace());
                        }
                        _ => {
                            ui.colored_label(Color32::from_rgb(0xf1, 0x6b, 0x6b), "Unresolved variable");
                        }
                    }
                });
            }
            ui.separator();
            ui.label(RichText::new(crate::runner::truncate(&resolved, 500)).monospace().small());
        })
    } else {
        resp
    }
}

pub struct KvOpts {
    pub description: bool,
    pub files: bool,
    pub key_hint: &'static str,
    pub value_hint: &'static str,
}

/// Postman-like key/value table. Returns true if changed.
pub fn kv_table(ui: &mut Ui, id: &str, rows: &mut Vec<KV>, opts: &KvOpts, bulk: &mut bool, bulk_text: &mut String, vars: &VarContext, dark: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let lbl = if *bulk { "Key-Value Edit" } else { "Bulk Edit" };
        if ui.small_button(lbl).clicked() {
            if !*bulk {
                *bulk_text = rows
                    .iter()
                    .filter(|r| !(r.key.is_empty() && r.value.is_empty()))
                    .map(|r| format!("{}{}:{}", if r.disabled { "//" } else { "" }, r.key, if r.is_file() { r.src_path() } else { r.value.clone() }))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            *bulk = !*bulk;
        }
    });
    if *bulk {
        ui.label(RichText::new("Format: key:value — one row per line, prefix with // to disable").small().weak());
        let r = ui.add(egui::TextEdit::multiline(bulk_text).code_editor().desired_rows(8).desired_width(f32::INFINITY));
        if r.changed() {
            let mut new = vec![];
            for line in bulk_text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let (dis, l) = match line.trim_start().strip_prefix("//") {
                    Some(r) => (true, r),
                    None => (false, line),
                };
                let (k, v) = l.split_once(':').unwrap_or((l, ""));
                let mut kv = rows.iter().find(|r| r.key == k.trim()).cloned().unwrap_or_default();
                kv.key = k.trim().to_string();
                if kv.is_file() {
                    kv.src = Some(serde_json::Value::String(v.trim().to_string()));
                } else {
                    kv.value = v.trim_start().to_string();
                }
                kv.disabled = dis;
                new.push(kv);
            }
            *rows = new;
            changed = true;
        }
        return changed;
    }
    // always keep one empty trailing row for adding
    if rows.last().map(|r| !(r.key.is_empty() && r.value.is_empty() && r.src.is_none())).unwrap_or(true) {
        rows.push(KV::default());
    }
    let avail = ui.available_width();
    let check_w = 22.0;
    let del_w = 22.0;
    let type_w = if opts.files { 62.0 } else { 0.0 };
    let cols = if opts.description { 3.0 } else { 2.0 };
    let col_w = ((avail - check_w - del_w - type_w - 30.0) / cols).max(80.0);
    let mut remove: Option<usize> = None;
    let n = rows.len();
    egui::Grid::new(id).num_columns(5).spacing([4.0, 3.0]).striped(true).show(ui, |ui| {
        ui.label("");
        ui.label(RichText::new("Key").small().strong());
        if opts.files {
            ui.label("");
        }
        ui.label(RichText::new("Value").small().strong());
        if opts.description {
            ui.label(RichText::new("Description").small().strong());
        }
        ui.end_row();
        for (i, row) in rows.iter_mut().enumerate() {
            let is_last = i == n - 1;
            let mut en = !row.disabled;
            if is_last {
                ui.label("");
            } else if ui.checkbox(&mut en, "").changed() {
                row.disabled = !en;
                changed = true;
            }
            if var_edit(ui, &mut row.key, vars, opts.key_hint, col_w, dark).changed() {
                changed = true;
            }
            if opts.files {
                let mut is_file = row.is_file();
                egui::ComboBox::from_id_salt((id, i, "t")).width(52.0).selected_text(if is_file { "File" } else { "Text" }).show_ui(ui, |ui| {
                    if ui.selectable_value(&mut is_file, false, "Text").clicked() | ui.selectable_value(&mut is_file, true, "File").clicked() {
                        row.kind = Some(if is_file { "file".into() } else { "text".into() });
                        changed = true;
                    }
                });
            }
            if opts.files && row.is_file() {
                ui.horizontal(|ui| {
                    let mut p = row.src_path();
                    if ui.add_sized([col_w - 60.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut p).hint_text("file path")).changed() {
                        row.src = Some(serde_json::Value::String(p.clone()));
                        changed = true;
                    }
                    if super::files::can_browse() && ui.small_button("Select…").clicked() {
                        if let Some(f) = super::files::pick_file(&[]) {
                            row.src = Some(serde_json::Value::String(f));
                            changed = true;
                        }
                    }
                });
            } else if var_edit(ui, &mut row.value, vars, opts.value_hint, col_w, dark).changed() {
                changed = true;
            }
            if opts.description {
                let mut d = row.desc();
                if ui.add_sized([col_w, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut d).hint_text("Description")).changed() {
                    row.description = if d.is_empty() { None } else { Some(crate::model::Description(d)) };
                    changed = true;
                }
            }
            if !is_last && ui.small_button("🗑").on_hover_text("Delete").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        rows.remove(i);
        changed = true;
    }
    changed
}

/// Remove trailing empty rows before saving
pub fn clean_rows(rows: &mut Vec<KV>) {
    rows.retain(|r| !(r.key.is_empty() && r.value.is_empty() && r.src.is_none() && r.description.is_none()));
}

// ------------------------------------------------------------------ syntax highlighting

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Json,
    Js,
    Xml,
    Plain,
}

pub fn lang_for_content_type(ct: &str, body: &str) -> Lang {
    let t = body.trim_start();
    if ct.contains("json") || ((t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<serde::de::IgnoredAny>(body).is_ok()) {
        Lang::Json
    } else if ct.contains("xml") || ct.contains("html") || t.starts_with('<') {
        Lang::Xml
    } else if ct.contains("javascript") {
        Lang::Js
    } else {
        Lang::Plain
    }
}

pub fn highlight_line(job: &mut LayoutJob, line: &str, lang: Lang, font: &FontId, p: &Palette) {
    match lang {
        Lang::Json => highlight_json(job, line, font, p),
        Lang::Js => highlight_js(job, line, font, p),
        Lang::Xml => highlight_xml(job, line, font, p),
        Lang::Plain => job.append(line, 0.0, fmt(font, p.text)),
    }
}

fn highlight_json(job: &mut LayoutJob, line: &str, font: &FontId, p: &Palette) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = |k: usize| chars.get(k).map(|c| c.0).unwrap_or(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        let s = chars[i].0;
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i].1 != '"' {
                if chars[i].1 == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            let e = at(i);
            let col = if line[e..].trim_start().starts_with(':') { p.key } else { p.string };
            job.append(&line[s..e], 0.0, fmt(font, col));
        } else if c == '-' || c.is_ascii_digit() {
            while i < chars.len() && (chars[i].1.is_ascii_digit() || matches!(chars[i].1, '.' | 'e' | 'E' | '+' | '-')) {
                i += 1;
            }
            job.append(&line[s..at(i)], 0.0, fmt(font, p.number));
        } else if line[s..].starts_with("true") || line[s..].starts_with("null") {
            i += 4;
            job.append(&line[s..at(i)], 0.0, fmt(font, p.kw));
        } else if line[s..].starts_with("false") {
            i += 5;
            job.append(&line[s..at(i)], 0.0, fmt(font, p.kw));
        } else {
            i += 1;
            while i < chars.len() && !matches!(chars[i].1, '"' | '-' | '0'..='9' | 't' | 'f' | 'n') {
                i += 1;
            }
            job.append(&line[s..at(i)], 0.0, fmt(font, p.punct));
        }
    }
}

const JS_KW: &[&str] = &["const", "let", "var", "function", "return", "if", "else", "for", "while", "do", "new", "true", "false", "null", "undefined", "this", "typeof", "instanceof", "try", "catch", "finally", "throw", "async", "await", "of", "in", "switch", "case", "break", "continue", "default", "class", "extends", "delete", "void"];

fn highlight_js(job: &mut LayoutJob, line: &str, font: &FontId, p: &Palette) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut i = 0;
    let end_of = |k: usize| chars.get(k).map(|c| c.0).unwrap_or(line.len());
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c == '/' && chars.get(i + 1).map(|x| x.1) == Some('/') {
            job.append(&line[pos..], 0.0, fmt(font, p.comment));
            return;
        }
        if c == '"' || c == '\'' || c == '`' {
            let q = c;
            let s = pos;
            i += 1;
            while i < chars.len() && chars[i].1 != q {
                if chars[i].1 == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            job.append(&line[s..end_of(i)], 0.0, fmt(font, p.string));
            continue;
        }
        if c.is_ascii_digit() {
            let s = pos;
            while i < chars.len() && (chars[i].1.is_ascii_alphanumeric() || chars[i].1 == '.') {
                i += 1;
            }
            job.append(&line[s..end_of(i)], 0.0, fmt(font, p.number));
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let s = pos;
            while i < chars.len() && (chars[i].1.is_alphanumeric() || chars[i].1 == '_' || chars[i].1 == '$') {
                i += 1;
            }
            let w = &line[s..end_of(i)];
            let col = if JS_KW.contains(&w) {
                p.kw
            } else if w == "pm" || w == "console" || w == "postman" {
                p.key
            } else {
                p.text
            };
            job.append(w, 0.0, fmt(font, col));
            continue;
        }
        let s = pos;
        i += 1;
        job.append(&line[s..end_of(i)], 0.0, fmt(font, p.punct));
    }
}

fn highlight_xml(job: &mut LayoutJob, line: &str, font: &FontId, p: &Palette) {
    let mut rest = line;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(0) => {
                let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
                let tag = &rest[..end];
                // tag name vs attributes
                let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
                job.append(&tag[..name_end], 0.0, fmt(font, p.kw));
                let attrs = &tag[name_end..];
                let mut a = attrs;
                while !a.is_empty() {
                    if let Some(q) = a.find('"') {
                        job.append(&a[..q], 0.0, fmt(font, p.key));
                        let close = a[q + 1..].find('"').map(|c| q + 1 + c + 1).unwrap_or(a.len());
                        job.append(&a[q..close], 0.0, fmt(font, p.string));
                        a = &a[close..];
                    } else {
                        job.append(a, 0.0, fmt(font, p.kw));
                        a = "";
                    }
                }
                rest = &rest[end..];
            }
            Some(n) => {
                job.append(&rest[..n], 0.0, fmt(font, p.text));
                rest = &rest[n..];
            }
            None => {
                job.append(rest, 0.0, fmt(font, p.text));
                rest = "";
            }
        }
    }
}

/// Multi-line code editor with syntax highlighting
pub fn code_editor(ui: &mut Ui, text: &mut String, lang: Lang, rows: usize, dark: bool, id: &str) -> egui::Response {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let p = palette(dark);
    let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, wrap: f32| -> Arc<Galley> {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        let s = buf.as_str();
        if s.len() > 300_000 {
            job.append(s, 0.0, fmt(&font, p.text));
        } else {
            for (i, line) in s.split('\n').enumerate() {
                if i > 0 {
                    job.append("\n", 0.0, fmt(&font, p.text));
                }
                highlight_line(&mut job, line, lang, &font, &p);
            }
        }
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    ui.add(egui::TextEdit::multiline(text).id_salt(id).code_editor().desired_rows(rows).desired_width(f32::INFINITY).lock_focus(true).layouter(&mut layouter))
}

/// Cached line index for virtualized text views (4 bytes per line)
#[derive(Default, Clone)]
pub struct TextIndex {
    pub text: String,
    starts: Vec<u32>,
    pub lang: Option<Lang>,
}

impl TextIndex {
    pub fn new(text: String, lang: Lang) -> Self {
        let mut starts = Vec::with_capacity(text.len() / 40 + 1);
        starts.push(0u32);
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push((i + 1) as u32);
            }
        }
        starts.shrink_to_fit();
        TextIndex { text, starts, lang: Some(lang) }
    }
    pub fn len(&self) -> usize {
        self.starts.len()
    }
    pub fn line(&self, i: usize) -> &str {
        let s = self.starts[i] as usize;
        let mut e = if i + 1 < self.starts.len() { self.starts[i + 1] as usize - 1 } else { self.text.len() };
        if e > s && self.text.as_bytes()[e - 1] == b'\r' {
            e -= 1;
        }
        &self.text[s..e.max(s)]
    }
}

/// Virtualized, highlighted, line-numbered view: only visible lines are laid out.
pub fn virtual_text(ui: &mut Ui, idx: &TextIndex, dark: bool, filter: &str, wrap: bool, scroll_to: Option<usize>) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let p = palette(dark);
    let lang = idx.lang.unwrap_or(Lang::Plain);
    let filter_l = filter.to_lowercase();
    // only materialize an index list when filtering (no per-frame allocation otherwise)
    let filtered: Option<Vec<u32>> = if filter.is_empty() { None } else { Some((0..idx.len()).filter(|&i| idx.line(i).to_lowercase().contains(&filter_l)).map(|i| i as u32).collect()) };
    let count = filtered.as_ref().map(|v| v.len()).unwrap_or(idx.len());
    let at = |vi: usize| -> usize { filtered.as_ref().map(|v| v[vi] as usize).unwrap_or(vi) };
    let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font)) + 1.0;
    let digits = format!("{}", idx.len()).len().max(2);
    let mut sa = egui::ScrollArea::both().auto_shrink([false, false]).id_salt("vtext");
    if wrap {
        sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("vtext");
    }
    if let Some(l) = scroll_to {
        sa = sa.vertical_scroll_offset(l as f32 * row_h);
    }
    if wrap {
        // wrapped mode: non-virtual but cheap enough for moderate sizes
        sa.show(ui, |ui| {
            for vi in 0..count.min(20000) {
                let i = at(vi);
                let mut job = LayoutJob::default();
                job.wrap.max_width = ui.available_width() - 50.0;
                job.append(&format!("{:>w$}  ", i + 1, w = digits), 0.0, fmt(&font, p.punct.gamma_multiply(0.5)));
                highlight_line(&mut job, idx.line(i), lang, &font, &p);
                ui.add(egui::Label::new(job).selectable(true));
            }
        });
        return;
    }
    sa.show_rows(ui, row_h, count, |ui, range| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for vi in range {
            let i = at(vi);
            let mut job = LayoutJob::default();
            job.wrap.max_width = f32::INFINITY;
            job.append(&format!("{:>w$}  ", i + 1, w = digits), 0.0, fmt(&font, p.punct.gamma_multiply(0.5)));
            let line = idx.line(i);
            if line.len() > 20_000 {
                let mut e = 20_000;
                while !line.is_char_boundary(e) {
                    e -= 1;
                }
                highlight_line(&mut job, &line[..e], lang, &font, &p);
                job.append(" …", 0.0, fmt(&font, p.comment));
            } else {
                highlight_line(&mut job, line, lang, &font, &p);
            }
            ui.add(egui::Label::new(job).selectable(true).extend());
        }
    });
}

/// Pretty-print JSON without building a value tree (O(n), one output allocation).
pub fn pretty_json(s: &str) -> Option<String> {
    let s = s.trim_start_matches('\u{feff}');
    serde_json::from_str::<serde::de::IgnoredAny>(s).ok()?;
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + b.len() / 2);
    let mut indent = 0usize;
    let mut i = 0;
    let nl = |out: &mut Vec<u8>, n: usize| {
        out.push(b'\n');
        for _ in 0..n {
            out.extend_from_slice(b"  ");
        }
    };
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                let st = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                out.extend_from_slice(&b[st..(i + 1).min(b.len())]);
            }
            b'{' | b'[' => {
                // empty containers stay on one line
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'}' || b[j] == b']') {
                    out.push(c);
                    out.push(b[j]);
                    i = j;
                } else {
                    out.push(c);
                    indent += 1;
                    nl(&mut out, indent);
                }
            }
            b'}' | b']' => {
                indent = indent.saturating_sub(1);
                nl(&mut out, indent);
                out.push(c);
            }
            b',' => {
                out.push(b',');
                nl(&mut out, indent);
            }
            b':' => out.extend_from_slice(b": "),
            c if c.is_ascii_whitespace() => {}
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Simple XML/HTML pretty printer (indentation by tag depth)
pub fn pretty_xml(s: &str) -> String {
    let mut out = String::new();
    let mut depth: i32 = 0;
    let mut rest = s.trim();
    let mut text_buf = String::new();
    while !rest.is_empty() {
        if rest.starts_with('<') {
            let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            let tag = &rest[..end];
            let t = text_buf.trim();
            let closing = tag.starts_with("</");
            let self_closing = tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!");
            if !t.is_empty() {
                if closing && !out.ends_with('\n') {
                    out.push_str(t);
                } else {
                    out.push_str(&"  ".repeat(depth.max(0) as usize));
                    out.push_str(t);
                    out.push('\n');
                }
            }
            text_buf.clear();
            if closing {
                depth -= 1;
                if out.ends_with('\n') {
                    out.push_str(&"  ".repeat(depth.max(0) as usize));
                }
                out.push_str(tag);
                out.push('\n');
            } else {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(depth.max(0) as usize));
                out.push_str(tag);
                // keep short text content inline: <a>text</a>
                let after = &rest[end..];
                let inline = !self_closing && after.find('<').map(|n| after[n..].starts_with("</") && !after[..n].contains('\n') && n < 200).unwrap_or(false);
                if inline {
                    let n = after.find('<').unwrap();
                    out.push_str(after[..n].trim());
                    let close_end = after[n..].find('>').map(|e| n + e + 1).unwrap_or(after.len());
                    out.push_str(&after[n..close_end]);
                    out.push('\n');
                    rest = &after[close_end..];
                    continue;
                }
                out.push('\n');
                if !self_closing && !tag.to_lowercase().starts_with("<br") && !tag.to_lowercase().starts_with("<meta") && !tag.to_lowercase().starts_with("<link") && !tag.to_lowercase().starts_with("<img") && !tag.to_lowercase().starts_with("<input") {
                    depth += 1;
                }
            }
            rest = &rest[end..];
        } else {
            let n = rest.find('<').unwrap_or(rest.len());
            text_buf.push_str(&rest[..n]);
            rest = &rest[n..];
        }
    }
    if !text_buf.trim().is_empty() {
        out.push_str(text_buf.trim());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn pretty() {
        let p = super::pretty_json(r#"{"a":[1,2,{}],"b":"x,y:\"z\"","c":{"d":[]}}"#).unwrap();
        let v1: serde_json::Value = serde_json::from_str(&p).unwrap();
        let v2: serde_json::Value = serde_json::from_str(r#"{"a":[1,2,{}],"b":"x,y:\"z\"","c":{"d":[]}}"#).unwrap();
        assert_eq!(v1, v2);
        assert_eq!(p, "{\n  \"a\": [\n    1,\n    2,\n    {}\n  ],\n  \"b\": \"x,y:\\\"z\\\"\",\n  \"c\": {\n    \"d\": []\n  }\n}");
        assert!(super::pretty_json("{bad").is_none());
        let idx = super::TextIndex::new("a\r\nbb\n\nc".into(), super::Lang::Plain);
        assert_eq!((idx.len(), idx.line(0), idx.line(1), idx.line(2), idx.line(3)), (4, "a", "bb", "", "c"));
    }
}
