//! Export bridge (EPIM interchange): turns Smart PIM records into standalone
//! document formats that `EssentialPIM` can import — Markdown (for humans and
//! previews), HTML, RTF, and XML.
//!
//! Every writer is pure (no I/O, no clock, no RNG) and deterministic: kinds are
//! emitted in a fixed order and the only timestamp is a `generated` string the
//! caller supplies, so identical inputs produce byte-identical output. Input
//! types are deliberately local to this module (`*Exp` structs + [`ExportRow`])
//! so export stays decoupled from the storage row types above it.
//!
//! All text is escaped per target format; note/diary HTML bodies are passed
//! through the crate's allowlist sanitizer before being embedded as rich HTML,
//! and stripped to plain text for the text-only writers.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// A task as seen by the exporter (`tasks.rs` row, storage fields dropped).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskExp {
    pub title: String,
    /// UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) or `None` when undated.
    pub due_utc: Option<String>,
    pub priority: i64,
    /// Completion percentage, `0..=100` by convention (not clamped here).
    pub percent: i64,
    pub completed: bool,
    pub tags: Vec<String>,
}

/// A note as seen by the exporter; `html` is editor HTML (Tiptap pipeline).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteExp {
    pub title: String,
    pub html: String,
    pub tags: Vec<String>,
}

/// One diary day; `entry_date` is a local `YYYY-MM-DD` string.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiaryExp {
    pub entry_date: String,
    pub mood: Option<String>,
    pub html: String,
}

/// A contact as seen by the exporter (`vcard::ContactRecord`, narrowed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactExp {
    pub display_name: String,
    pub organization: Option<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub urls: Vec<String>,
    pub address: Option<String>,
    /// `YYYY-MM-DD` when known.
    pub birthday: Option<String>,
    pub notes: Option<String>,
}

/// An email message as seen by the exporter (headers + plain-text snippet).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageExp {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub date: String,
    pub snippet: String,
}

/// One record of any exportable kind, so the shell can hand a whole mixed
/// selection to a single writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ExportRow {
    Task(TaskExp),
    Note(NoteExp),
    Diary(DiaryExp),
    Contact(ContactExp),
    Message(MessageExp),
}

/// Render `rows` as a Markdown document (H1 per section kind, tables for
/// tasks/contacts/messages, bullet lists for notes/diary).
#[must_use]
pub fn to_markdown(rows: &[ExportRow], generated: &str) -> String {
    let mut out = String::from("# Smart PIM Export\n");
    if !generated.trim().is_empty() {
        let _ = writeln!(out, "\n_Generated: {}_", md_text(generated));
    }
    if let Some(group) = md_task_table(rows) {
        out.push_str("\n## Tasks\n\n");
        out.push_str(&group);
    }
    if let Some(group) = md_note_list(rows) {
        out.push_str("\n## Notes\n\n");
        out.push_str(&group);
    }
    if let Some(group) = md_diary_list(rows) {
        out.push_str("\n## Diary\n\n");
        out.push_str(&group);
    }
    if let Some(group) = md_contact_table(rows) {
        out.push_str("\n## Contacts\n\n");
        out.push_str(&group);
    }
    if let Some(group) = md_message_table(rows) {
        out.push_str("\n## Messages\n\n");
        out.push_str(&group);
    }
    out
}

fn md_task_table(rows: &[ExportRow]) -> Option<String> {
    let tasks: Vec<&TaskExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Task(t) => Some(t),
            _ => None,
        })
        .collect();
    if tasks.is_empty() {
        return None;
    }
    let mut out = String::from("| Title | Due (UTC) | Priority | Percent | Completed | Tags |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for t in tasks {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            md_text(&t.title),
            md_text(t.due_utc.as_deref().unwrap_or("")),
            t.priority,
            t.percent,
            if t.completed { "yes" } else { "no" },
            md_text(&t.tags.join(", "))
        );
    }
    Some(out)
}

fn md_contact_table(rows: &[ExportRow]) -> Option<String> {
    let contacts: Vec<&ContactExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Contact(c) => Some(c),
            _ => None,
        })
        .collect();
    if contacts.is_empty() {
        return None;
    }
    let mut out = String::from(
        "| Name | Organization | Emails | Phones | URLs | Address | Birthday | Notes |\n",
    );
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for c in contacts {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            md_text(&c.display_name),
            md_text(c.organization.as_deref().unwrap_or("")),
            md_text(&c.emails.join(", ")),
            md_text(&c.phones.join(", ")),
            md_text(&c.urls.join(", ")),
            md_text(c.address.as_deref().unwrap_or("")),
            md_text(c.birthday.as_deref().unwrap_or("")),
            md_text(c.notes.as_deref().unwrap_or(""))
        );
    }
    Some(out)
}

fn md_message_table(rows: &[ExportRow]) -> Option<String> {
    let msgs: Vec<&MessageExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Message(m) => Some(m),
            _ => None,
        })
        .collect();
    if msgs.is_empty() {
        return None;
    }
    let mut out = String::from("| Subject | From | Date | Snippet |\n| --- | --- | --- | --- |\n");
    for m in msgs {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            md_text(&m.subject),
            md_text(&format!("{} <{}>", m.from_name, m.from_addr)),
            md_text(&m.date),
            md_text(&m.snippet)
        );
    }
    Some(out)
}

fn md_note_list(rows: &[ExportRow]) -> Option<String> {
    let notes: Vec<&NoteExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Note(n) => Some(n),
            _ => None,
        })
        .collect();
    if notes.is_empty() {
        return None;
    }
    let mut out = String::new();
    for n in notes {
        let _ = writeln!(
            out,
            "- **{}**: {}",
            md_text(&n.title),
            md_text(&plain_text(&n.html))
        );
    }
    Some(out)
}

fn md_diary_list(rows: &[ExportRow]) -> Option<String> {
    let entries: Vec<&DiaryExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Diary(d) => Some(d),
            _ => None,
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    let mut out = String::new();
    for d in entries {
        let mood = d.mood.as_deref().unwrap_or("no mood");
        let _ = writeln!(
            out,
            "- **{}** _({})_: {}",
            md_text(&d.entry_date),
            md_text(mood),
            md_text(&plain_text(&d.html))
        );
    }
    Some(out)
}

/// Markdown text: backslash-escape `|` (the only metacharacter in the tables
/// we emit) and flatten newlines so one record stays on one line.
fn md_text(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// Render `rows` as a standalone HTML document (doctype, charset, inline style,
/// one table per tabular kind, headed sections for notes and diary).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn to_html(rows: &[ExportRow], generated: &str) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<title>Smart PIM Export</title>\n");
    out.push_str(
        "<style>\nbody{background:#f5f7fa;color:#243b53;font-family:sans-serif;margin:2rem;}\n\
         h1{color:#102a43;} h2{border-bottom:1px solid #d9e2ec;padding-bottom:.2rem;}\n\
         table{border-collapse:collapse;width:100%;margin-bottom:1.5rem;background:#ffffff;}\n\
         th,td{border:1px solid #d9e2ec;padding:.35rem .5rem;text-align:left;vertical-align:top;}\n\
         th{background:#f0f4f8;} .muted{color:#627d98;}\n</style>\n",
    );
    out.push_str("</head>\n<body>\n<h1>Smart PIM Export</h1>\n");
    if !generated.trim().is_empty() {
        let _ = writeln!(
            out,
            "<p class=\"muted\">Generated: {}</p>",
            esc_html(generated)
        );
    }

    let tasks: Vec<&TaskExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Task(t) => Some(t),
            _ => None,
        })
        .collect();
    if !tasks.is_empty() {
        out.push_str("<h2>Tasks</h2>\n<table>\n<thead><tr><th>Title</th><th>Due (UTC)</th><th>Priority</th><th>Percent</th><th>Completed</th><th>Tags</th></tr></thead>\n<tbody>\n");
        for t in &tasks {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc_html(&t.title),
                esc_html(t.due_utc.as_deref().unwrap_or("")),
                t.priority,
                t.percent,
                if t.completed { "yes" } else { "no" },
                esc_html(&t.tags.join(", "))
            );
        }
        out.push_str("</tbody>\n</table>\n");
    }

    let notes: Vec<&NoteExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Note(n) => Some(n),
            _ => None,
        })
        .collect();
    if !notes.is_empty() {
        out.push_str("<h2>Notes</h2>\n");
        for n in &notes {
            let _ = writeln!(
                out,
                "<section>\n<h3>{}</h3>\n{}</section>\n",
                esc_html(&n.title),
                sanitized(&n.html)
            );
        }
    }

    let diary: Vec<&DiaryExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Diary(d) => Some(d),
            _ => None,
        })
        .collect();
    if !diary.is_empty() {
        out.push_str("<h2>Diary</h2>\n");
        for d in &diary {
            let mood = d.mood.as_deref().unwrap_or("no mood");
            let _ = writeln!(
                out,
                "<section>\n<h3>{} <span class=\"muted\">({})</span></h3>\n{}</section>\n",
                esc_html(&d.entry_date),
                esc_html(mood),
                sanitized(&d.html)
            );
        }
    }

    let contacts: Vec<&ContactExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Contact(c) => Some(c),
            _ => None,
        })
        .collect();
    if !contacts.is_empty() {
        out.push_str("<h2>Contacts</h2>\n<table>\n<thead><tr><th>Name</th><th>Organization</th><th>Emails</th><th>Phones</th><th>URLs</th><th>Address</th><th>Birthday</th><th>Notes</th></tr></thead>\n<tbody>\n");
        for c in &contacts {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc_html(&c.display_name),
                esc_html(c.organization.as_deref().unwrap_or("")),
                esc_html(&c.emails.join(", ")),
                esc_html(&c.phones.join(", ")),
                esc_html(&c.urls.join(", ")),
                esc_html(c.address.as_deref().unwrap_or("")),
                esc_html(c.birthday.as_deref().unwrap_or("")),
                esc_html(c.notes.as_deref().unwrap_or(""))
            );
        }
        out.push_str("</tbody>\n</table>\n");
    }

    let msgs: Vec<&MessageExp> = rows
        .iter()
        .filter_map(|r| match r {
            ExportRow::Message(m) => Some(m),
            _ => None,
        })
        .collect();
    if !msgs.is_empty() {
        out.push_str("<h2>Messages</h2>\n<table>\n<thead><tr><th>Subject</th><th>From</th><th>Date</th><th>Snippet</th></tr></thead>\n<tbody>\n");
        for m in &msgs {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{} &lt;{}&gt;</td><td>{}</td><td>{}</td></tr>",
                esc_html(&m.subject),
                esc_html(&m.from_name),
                esc_html(&m.from_addr),
                esc_html(&m.date),
                esc_html(&m.snippet)
            );
        }
        out.push_str("</tbody>\n</table>\n");
    }

    out.push_str("</body>\n</html>\n");
    out
}

/// Code point → signed 16-bit RTF escape value (negative once `> 32767`).
fn signed16(cp: u32) -> i32 {
    let n = cp as i32;
    if n > 32_767 {
        n - 65_536
    } else {
        n
    }
}

/// Render `rows` as minimal RTF: one paragraph group per record, bold titles,
/// `\\uN?` escapes for anything outside ASCII.
#[must_use]
pub fn to_rtf(rows: &[ExportRow], generated: &str) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Helvetica;}}\\f0\\fs20\n");
    if !generated.trim().is_empty() {
        let _ = writeln!(out, "{{\\i {}\\par}}", rtf_text(generated));
    }
    let _ = writeln!(out, "{{\\b Smart PIM Export\\par}}");
    for row in rows {
        match row {
            ExportRow::Task(t) => {
                let _ = write!(
                    out,
                    "{{\\b Task: {}\\par}}\n{{Due: {} | Priority: {} | Percent: {} | Completed: {} | Tags: {}\\par}}\n",
                    rtf_text(&t.title),
                    rtf_text(t.due_utc.as_deref().unwrap_or("-")),
                    t.priority,
                    t.percent,
                    if t.completed { "yes" } else { "no" },
                    rtf_text(&t.tags.join(", "))
                );
            }
            ExportRow::Note(n) => {
                let _ = write!(
                    out,
                    "{{\\b Note: {}\\par}}\n{{{}}}\\par\n",
                    rtf_text(&n.title),
                    rtf_text(&plain_text(&n.html))
                );
            }
            ExportRow::Diary(d) => {
                let _ = write!(
                    out,
                    "{{\\b Diary: {} ({})\\par}}\n{{{}}}\\par\n",
                    rtf_text(&d.entry_date),
                    rtf_text(d.mood.as_deref().unwrap_or("no mood")),
                    rtf_text(&plain_text(&d.html))
                );
            }
            ExportRow::Contact(c) => {
                let _ = write!(
                    out,
                    "{{\\b Contact: {}\\par}}\n{{Organization: {} | Emails: {} | Phones: {} | URLs: {} | Address: {} | Birthday: {} | Notes: {}\\par}}\n",
                    rtf_text(&c.display_name),
                    rtf_text(c.organization.as_deref().unwrap_or("-")),
                    rtf_text(&c.emails.join(", ")),
                    rtf_text(&c.phones.join(", ")),
                    rtf_text(&c.urls.join(", ")),
                    rtf_text(c.address.as_deref().unwrap_or("-")),
                    rtf_text(c.birthday.as_deref().unwrap_or("-")),
                    rtf_text(c.notes.as_deref().unwrap_or("-"))
                );
            }
            ExportRow::Message(m) => {
                let _ = write!(
                    out,
                    "{{\\b Message: {}\\par}}\n{{From: {} <{}> | Date: {}\\par}}\n{{{}}}\\par\n",
                    rtf_text(&m.subject),
                    rtf_text(&m.from_name),
                    rtf_text(&m.from_addr),
                    rtf_text(&m.date),
                    rtf_text(&m.snippet)
                );
            }
        }
    }
    out.push_str("}\n");
    out
}

/// RTF body text: escape the two control characters, braces, and emit every
/// non-ASCII char as a signed `\\uN?` escape (negative above 32767, per spec;
/// astral chars become a surrogate pair).
fn rtf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\par "),
            '\r' => {}
            c if c.is_ascii() => out.push(c),
            c if (c as u32) < 0x1_0000 => {
                let _ = write!(out, "\\u{}?", signed16(c as u32));
            }
            c => {
                // Above the BMP, RTF wants a UTF-16 surrogate pair.
                let v = c as u32 - 0x1_0000;
                let _ = write!(
                    out,
                    "\\u{}?\\u{}?",
                    signed16(0xD800 + (v >> 10)),
                    signed16(0xDC00 + (v & 0x3FF))
                );
            }
        }
    }
    out
}

/// Render `rows` as a `smartpim-export` XML document with one container
/// element per kind (`tasks`, `notes`, `diary`, `contacts`, `messages`).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn to_xml(rows: &[ExportRow], generated: &str) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<smartpim-export generated=\"{}\">",
        esc_xml(generated)
    );

    out.push_str("  <tasks>\n");
    for r in rows {
        if let ExportRow::Task(t) = r {
            let _ = writeln!(
                out,
                "    <task><title>{}</title><due>{}</due><priority>{}</priority><percent>{}</percent><completed>{}</completed><tags>{}</tags></task>",
                esc_xml(&t.title),
                esc_xml(t.due_utc.as_deref().unwrap_or("")),
                t.priority,
                t.percent,
                t.completed,
                esc_xml(&t.tags.join(", "))
            );
        }
    }
    out.push_str("  </tasks>\n");

    out.push_str("  <notes>\n");
    for r in rows {
        if let ExportRow::Note(n) = r {
            let _ = writeln!(
                out,
                "    <note><title>{}</title><html>{}</html><tags>{}</tags></note>",
                esc_xml(&n.title),
                esc_xml(&n.html),
                esc_xml(&n.tags.join(", "))
            );
        }
    }
    out.push_str("  </notes>\n");

    out.push_str("  <diary>\n");
    for r in rows {
        if let ExportRow::Diary(d) = r {
            let _ = writeln!(
                out,
                "    <entry><date>{}</date><mood>{}</mood><html>{}</html></entry>",
                esc_xml(&d.entry_date),
                esc_xml(d.mood.as_deref().unwrap_or("")),
                esc_xml(&d.html)
            );
        }
    }
    out.push_str("  </diary>\n");

    out.push_str("  <contacts>\n");
    for r in rows {
        if let ExportRow::Contact(c) = r {
            let _ = writeln!(
                out,
                "    <contact><name>{}</name><organization>{}</organization><emails>{}</emails><phones>{}</phones><urls>{}</urls><address>{}</address><birthday>{}</birthday><notes>{}</notes></contact>",
                esc_xml(&c.display_name),
                esc_xml(c.organization.as_deref().unwrap_or("")),
                esc_xml(&c.emails.join(", ")),
                esc_xml(&c.phones.join(", ")),
                esc_xml(&c.urls.join(", ")),
                esc_xml(c.address.as_deref().unwrap_or("")),
                esc_xml(c.birthday.as_deref().unwrap_or("")),
                esc_xml(c.notes.as_deref().unwrap_or(""))
            );
        }
    }
    out.push_str("  </contacts>\n");

    out.push_str("  <messages>\n");
    for r in rows {
        if let ExportRow::Message(m) = r {
            let _ = writeln!(
                out,
                "    <message><subject>{}</subject><fromName>{}</fromName><fromAddress>{}</fromAddress><date>{}</date><snippet>{}</snippet></message>",
                esc_xml(&m.subject),
                esc_xml(&m.from_name),
                esc_xml(&m.from_addr),
                esc_xml(&m.date),
                esc_xml(&m.snippet)
            );
        }
    }
    out.push_str("  </messages>\n</smartpim-export>\n");
    out
}

/// XML text escaping: the four predefined entities, plus newlines as `&#10;`
/// so a record can never spill across lines and break the layout.
fn esc_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
}

/// HTML text escaping (same set as `htmlsan::esc`, plus `'` for safety in
/// single-quoted attribute contexts).
fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('\n', "<br>")
}

/// Note/diary bodies are editor HTML: allowlist-sanitize before embedding in
/// the HTML writer so a malicious note cannot smuggle scripts into the export.
fn sanitized(html: &str) -> String {
    crate::htmlsan::sanitize_html(html)
}

/// Strip tags for text-only writers (Markdown, RTF): allowlist-sanitize first
/// so `script`/`style` bodies are dropped entirely, then peel the tags with the
/// same minimal scanner `diary::word_count` uses (these bodies are editor HTML).
fn plain_text(html: &str) -> String {
    let html = sanitized(html);
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Title hitting every HTML/XML metacharacter, plus unicode and a newline.
    fn nasty() -> String {
        "A <b>&</b> \"quote\" 'single' é\nsecond line".into()
    }

    fn sample() -> Vec<ExportRow> {
        vec![
            ExportRow::Task(TaskExp {
                title: nasty(),
                due_utc: Some("2026-10-20T09:00:00Z".into()),
                priority: 3,
                percent: 50,
                completed: false,
                tags: vec!["work|urgent".into(), "émail".into()],
            }),
            ExportRow::Note(NoteExp {
                title: nasty(),
                html: "<p>café <script>alert(1)</script><b>bold</b></p>".into(),
                tags: vec!["ideas".into()],
            }),
            ExportRow::Diary(DiaryExp {
                entry_date: "2026-10-19".into(),
                mood: Some("calm".into()),
                html: "<p>rainy day é</p>".into(),
            }),
            ExportRow::Contact(ContactExp {
                display_name: "Doe, Jane".into(),
                organization: Some("Acme <Inc>".into()),
                emails: vec!["jane@acme.example".into()],
                phones: vec!["+1 555 0100".into()],
                urls: vec!["https://acme.example".into()],
                address: Some("1 Main St".into()),
                birthday: Some("1990-01-02".into()),
                notes: Some("met at café".into()),
            }),
            ExportRow::Message(MessageExp {
                subject: nasty(),
                from_name: "Jane Doe".into(),
                from_addr: "jane@acme.example".into(),
                date: "2026-10-19T08:00:00Z".into(),
                snippet: "plain text & <snip>".into(),
            }),
        ]
    }

    fn assert_balanced_xml(doc: &str) {
        // Naive well-formedness spot check: no raw '<' inside text other than
        // tags, and every open has a matching close (no self-closing tags are
        // emitted by `to_xml`).
        let mut stack: Vec<&str> = Vec::new();
        let mut rest = doc;
        while let Some(p) = rest.find('<') {
            let after = &rest[p + 1..];
            if let Some(q) = after.find('>') {
                let tag = &after[..q];
                if let Some(name) = tag.strip_prefix('/') {
                    let open = stack.pop().unwrap_or("");
                    assert_eq!(
                        open, name,
                        "mismatched close </{name}>, stack was {stack:?}"
                    );
                } else if !tag.starts_with('?') && !tag.contains('/') {
                    let name = tag.split_whitespace().next().unwrap_or("");
                    stack.push(name);
                }
                rest = &after[q + 1..];
            } else {
                panic!("unterminated tag in: {rest}");
            }
        }
        assert!(stack.is_empty(), "unclosed tags: {stack:?}");
    }

    #[test]
    fn markdown_escapes_pipes_and_uses_tables_and_lists() {
        let md = to_markdown(&sample(), "2026-10-19");
        assert!(md.starts_with("# Smart PIM Export\n"));
        for heading in [
            "## Tasks",
            "## Notes",
            "## Diary",
            "## Contacts",
            "## Messages",
        ] {
            assert!(md.contains(heading), "missing {heading} in:\n{md}");
        }
        // Pipe in a tag must not open a new column.
        assert!(md.contains("work\\|urgent"));
        assert!(
            !md.contains("|urgent |"),
            "raw pipe leaked into table:\n{md}"
        );
        // Tables stay row-shaped: 6 columns = 7 delimiters, plus the one
        // escaped pipe inside the tag cell.
        let task_row = md
            .lines()
            .find(|l| l.starts_with("| A <b>") || l.starts_with("| A "))
            .expect("task row");
        assert_eq!(
            task_row.replace("\\|", "").matches('|').count(),
            7,
            "task row: {task_row}"
        );
        // Newlines flattened, tags stripped for notes/diary lists.
        assert!(md.contains("- **A <b>&</b> \"quote\" 'single' é second line**:"));
        assert!(md.contains("- **2026-10-19** _(calm)_: rainy day é"));
        assert!(!md.contains("alert(1)"));
    }

    #[test]
    fn html_is_standalone_document_with_escaped_text() {
        let html = to_html(&sample(), "2026-10-19");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<title>Smart PIM Export</title>"));
        assert!(html.contains("background:#f5f7fa"));
        assert!(html.contains("Generated: 2026-10-19"));
        // Raw metacharacters survive only as entities; tags are stripped from
        // note bodies and scripts dropped entirely by the sanitizer.
        assert!(html.contains("<td>A &lt;b&gt;&amp;&lt;/b&gt; &quot;quote&quot; &#39;single&#39; é<br>second line</td>"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("alert(1)"));
        assert!(html.contains("<b>bold</b>"), "allowlisted tags kept");
        assert!(html.ends_with("</body>\n</html>\n"));
        // Style block stays asset-free (no external fonts/images).
        let style = html.split("<style>").nth(1).expect("style block");
        let style = style.split("</style>").next().expect("style body");
        assert!(!style.contains("http") && !style.contains("url("));
    }

    #[test]
    fn rtf_escapes_controls_braces_and_non_ascii() {
        let rtf = to_rtf(&sample(), "2026-10-19");
        assert!(rtf.starts_with("{\\rtf1\\ansi\\deff0"));
        assert!(rtf.ends_with("}\n"));
        // Balanced group braces: the escapes use backslash, so a naive count of
        // unescaped braces must pair up.
        let mut depth = 0i64;
        let mut chars = rtf.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    chars.next(); // skip escaped char
                }
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "unbalanced RTF groups:\n{rtf}");
        }
        assert_eq!(depth, 0, "unbalanced RTF groups:\n{rtf}");
        // é (U+00E9 = 233) is a \u escape, not a raw byte.
        assert!(rtf.contains("\\u233?"));
        assert!(!rtf.contains("é"));
        // Newlines become \par.
        assert!(rtf.contains("second line"));
        // Control chars, braces, and astral chars via the escaper directly.
        assert_eq!(rtf_text("a\\b"), "a\\\\b");
        assert_eq!(rtf_text("{x}"), "\\{x\\}");
        assert_eq!(rtf_text("line\nbreak"), "line\\par break");
        assert_eq!(rtf_text("\u{1F600}"), "\\u-10179?\\u-8704?"); // surrogate pair
    }

    #[test]
    fn xml_is_well_formed_and_escaped() {
        let xml = to_xml(&sample(), "2026-10-19");
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<smartpim-export generated=\"2026-10-19\">"));
        assert!(xml.ends_with("</smartpim-export>\n"));
        for container in ["<tasks>", "<notes>", "<diary>", "<contacts>", "<messages>"] {
            assert!(xml.contains(container), "missing {container}");
        }
        assert_balanced_xml(&xml);
        assert!(xml.contains("<title>A &lt;b&gt;&amp;&lt;/b&gt; &quot;quote&quot; &apos;single&apos; é&#10;second line</title>"));
        assert!(!xml.contains("<b>&</b>"), "raw markup must be escaped");
        // Fixed kind order regardless of input order.
        let reversed: Vec<ExportRow> = sample().into_iter().rev().collect();
        let xml_rev = to_xml(&reversed, "2026-10-19");
        assert!(xml_rev.find("<tasks>").unwrap() < xml_rev.find("<messages>").unwrap());
        // Same input, same output.
        assert_eq!(xml, to_xml(&sample(), "2026-10-19"));
    }

    #[test]
    fn empty_slice_yields_valid_empty_documents() {
        let md = to_markdown(&[], "");
        assert_eq!(md, "# Smart PIM Export\n");
        let html = to_html(&[], "2026-10-19");
        assert!(html.starts_with("<!DOCTYPE html>") && html.ends_with("</html>\n"));
        assert!(!html.contains("<table>"));
        let rtf = to_rtf(&[], "");
        assert!(rtf.starts_with("{\\rtf1\\ansi\\deff0") && rtf.ends_with("}\n"));
        let xml = to_xml(&[], "");
        assert_balanced_xml(&xml);
        assert!(xml.contains("<tasks>\n  </tasks>"));
        // Blank `generated` emits no timestamp line anywhere.
        assert!(!md.contains("Generated:"));
        assert!(!rtf.contains("\\i "));
    }

    #[test]
    fn output_is_deterministic_across_runs() {
        let rows = sample();
        assert_eq!(to_markdown(&rows, "d"), to_markdown(&rows, "d"));
        assert_eq!(to_html(&rows, "d"), to_html(&rows, "d"));
        assert_eq!(to_rtf(&rows, "d"), to_rtf(&rows, "d"));
        assert_eq!(to_xml(&rows, "d"), to_xml(&rows, "d"));
    }
}
