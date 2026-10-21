//! Mail store (T1.9): accounts, folders, message upsert with threading,
//! EML ingest, and list queries. Protocol clients (IMAP/POP3/SMTP) live
//! above this crate and call into it.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::model::{now_iso, Kind};
use crate::pagination;
use crate::repo;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub color: Option<String>,
    /// Login / from-address for composing.
    pub user: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub role: Option<String>,
    pub last_seen_uid: i64,
}

/// Everything needed to store one message.
#[derive(Debug, Clone, Default)]
pub struct MessageInput {
    pub uid: i64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    /// RFC 3339.
    pub date: String,
    pub size: i64,
    pub flags: String,
    pub body: String,
    pub html_path: Option<String>,
}

pub fn create_account(
    conn: &Connection,
    name: &str,
    kind: &str,
    host: Option<&str>,
    port: Option<i64>,
    color: Option<&str>,
) -> Result<i64> {
    if kind != "imap" && kind != "pop3" {
        return Err(Error::Row {
            row: 0,
            message: format!("unknown account kind {kind}"),
        });
    }
    conn.execute(
        "INSERT INTO mail_account (name, kind, host, port, color) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![name, kind, host, port, color],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn ensure_folder(
    conn: &Connection,
    account_id: i64,
    parent_id: Option<i64>,
    name: &str,
    role: Option<&str>,
) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM mail_folder WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO mail_folder (account_id, parent_id, name, role) VALUES (?1, ?2, ?3, ?4)",
        params![account_id, parent_id, name, role],
    )?;
    Ok(conn.last_insert_rowid())
}

fn normalize_subject(s: &str) -> String {
    let mut out = s.trim().to_lowercase();
    loop {
        let next = out
            .strip_prefix("re:")
            .or_else(|| out.strip_prefix("fw:"))
            .or_else(|| out.strip_prefix("fwd:"))
            .map_or(out.clone(), str::to_string);
        let next = next.trim_start().to_string();
        if next == out {
            break;
        }
        out = next;
    }
    out
}

fn snippet_of(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(200).collect()
}

/// Insert a message, assigning it to a thread: parent found by Message-ID of
/// `in_reply_to`, else a new thread keyed on the normalized subject.
/// Returns the message row id. Idempotent per (folder, uid) — re-upserting
/// the same UID replaces the row.
pub fn upsert_message(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    m: &MessageInput,
) -> Result<i64> {
    if m.date.is_empty() {
        return Err(Error::Row {
            row: 0,
            message: "message without date".into(),
        });
    }
    let now = now_iso();
    let flags = if m.flags.is_empty() { "{}" } else { &m.flags };

    // Thread assignment.
    let thread_id: i64 = if let Some(irt) = m.in_reply_to.as_deref().filter(|s| !s.is_empty()) {
        conn.query_row(
            "SELECT t.id FROM thread t JOIN message msg ON msg.thread_id = t.id \
             WHERE msg.message_id = ?1 AND msg.account_id = ?2 LIMIT 1",
            params![irt, account_id],
            |r| r.get(0),
        )
        .optional()?
        .or_else(|| {
            conn.query_row(
                "SELECT id FROM thread WHERE subject = ?1 AND account_id = ?2 LIMIT 1",
                params![normalize_subject(&m.subject), account_id],
                |r| r.get(0),
            )
            .optional()
            .ok()?
        })
        .unwrap_or(0)
    } else {
        conn.query_row(
            "SELECT id FROM thread WHERE subject = ?1 AND account_id = ?2 LIMIT 1",
            params![normalize_subject(&m.subject), account_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0)
    };

    let thread_id = if thread_id == 0 {
        conn.execute(
            "INSERT INTO thread (account_id, subject, last_date) VALUES (?1, ?2, ?3)",
            params![account_id, normalize_subject(&m.subject), m.date],
        )?;
        conn.last_insert_rowid()
    } else {
        conn.execute(
            "UPDATE thread SET last_date = MAX(COALESCE(last_date, ''), ?1) WHERE id = ?2",
            params![m.date, thread_id],
        )?;
        thread_id
    };

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM message WHERE folder_id = ?1 AND uid = ?2",
            params![folder_id, m.uid],
            |r| r.get(0),
        )
        .optional()?;

    if let Some(id) = existing {
        conn.execute(
            "UPDATE message SET message_id=?1, in_reply_to=?2, thread_id=?3, subject=?4, \
             from_name=?5, from_addr=?6, to_addrs=?7, cc_addrs=?8, date=?9, size=?10, \
             flags=?11, body=?12, snippet=?13, html_path=?14 WHERE id=?15",
            params![
                m.message_id,
                m.in_reply_to,
                thread_id,
                m.subject,
                m.from_name,
                m.from_addr,
                m.to_addrs,
                m.cc_addrs,
                m.date,
                m.size,
                flags,
                m.body,
                snippet_of(&m.body),
                m.html_path,
                id
            ],
        )?;
        return Ok(id);
    }

    conn.execute(
        "INSERT INTO message (item_id, account_id, folder_id, uid, message_id, in_reply_to, \
         thread_id, subject, from_name, from_addr, to_addrs, cc_addrs, date, size, flags, \
         body, snippet, html_path, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            repo::create_item(conn, Kind::Message, None, false)?,
            account_id, folder_id, m.uid, m.message_id, m.in_reply_to, thread_id, m.subject,
            m.from_name, m.from_addr, m.to_addrs, m.cc_addrs, m.date, m.size, flags, m.body,
            snippet_of(&m.body), m.html_path, now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Parse raw RFC 822 bytes into a `MessageInput` (`uid` = 0; callers set it).
/// Single source of header extraction shared by EML ingest, IMAP and POP3.
pub fn parse_message(eml: &[u8]) -> Result<MessageInput> {
    use mail_parser::MessageParser;
    let msg = MessageParser::default()
        .parse(eml)
        .ok_or_else(|| Error::parse("eml", "unparseable message"))?;
    let mut m = MessageInput {
        uid: 0,
        subject: msg.subject().unwrap_or("").to_string(),
        date: msg
            .date()
            .map_or_else(now_iso, mail_parser::DateTime::to_rfc3339),
        body: msg.body_text(0).map(|t| t.to_string()).unwrap_or_default(),
        size: i64::try_from(eml.len()).unwrap_or(i64::MAX),
        ..Default::default()
    };
    if m.body.is_empty() {
        if let Some(html) = msg.body_html(0) {
            m.body = strip_html(&html);
            m.html_path = None;
        }
    }
    if let Some(from) = msg.from() {
        let addr = from.first();
        if let Some(a) = addr {
            m.from_name = a.name().unwrap_or("").to_string();
            m.from_addr = a.address().unwrap_or("").to_string();
        }
    }
    let join_addrs = |vals: Option<&mail_parser::Address<'_>>| -> String {
        vals.map(|a| {
            a.iter()
                .filter_map(|x| x.address().map(str::to_string))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
    };
    m.to_addrs = join_addrs(msg.to());
    m.cc_addrs = join_addrs(msg.cc());
    m.message_id = msg.message_id().map(str::to_string);
    m.in_reply_to = match msg.in_reply_to() {
        mail_parser::HeaderValue::Address(a) => a
            .iter()
            .next()
            .and_then(|x| x.address())
            .map(str::to_string),
        mail_parser::HeaderValue::Text(t) => Some(t.trim().to_string()),
        _ => None,
    };
    Ok(m)
}

/// Parse raw EML bytes and store them. Returns the message row id.
pub fn ingest_eml(conn: &Connection, account_id: i64, folder_id: i64, eml: &[u8]) -> Result<i64> {
    let mut m = parse_message(eml)?;
    m.uid = next_uid(conn, folder_id)?;
    upsert_message(conn, account_id, folder_id, &m)
}

fn next_uid(conn: &Connection, folder_id: i64) -> Result<i64> {
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(uid), 0) + 1 FROM message WHERE folder_id = ?1",
        [folder_id],
        |r| r.get(0),
    )?;
    Ok(next)
}

/// Naive HTML→text for FTS/snippets when only an HTML body exists.
#[must_use]
pub fn strip_html(html: &str) -> String {
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
    out
}

/// Unread/total counts per folder (Today view, folder badges).
#[derive(Debug, serde::Serialize)]
pub struct FolderCounts {
    pub folder_id: i64,
    pub total: i64,
    pub unread: i64,
}

pub fn folder_counts(conn: &Connection, account_id: i64) -> Result<Vec<FolderCounts>> {
    let mut stmt = conn.prepare(
        "SELECT folder_id, count(*), \
         SUM(CASE WHEN json_extract(flags, '$.seen') IS NULL OR json_extract(flags, '$.seen') = 0 \
         THEN 1 ELSE 0 END) FROM message WHERE account_id = ?1 GROUP BY folder_id",
    )?;
    let rows = stmt.query_map([account_id], |r| {
        Ok(FolderCounts {
            folder_id: r.get(0)?,
            total: r.get(1)?,
            unread: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
        })
    })?;
    let out = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn setup() -> Store {
        Store::open_memory("x").unwrap()
    }

    const EML: &str = "From: Ana Ana <ana@example.com>\r\nTo: bob@example.com, carol@example.com\r\nCc: dan@example.com\r\nSubject: Bakery order\r\nDate: Wed, 01 Apr 2026 09:30:00 +0000\r\nMessage-ID: <m1@example.com>\r\n\r\nTwo sourdough loaves please.\r\n";

    #[test]
    fn account_and_folder_crud() {
        let store = setup();
        let acct = create_account(
            store.conn(),
            "Work",
            "imap",
            Some("imap.x.com"),
            Some(993),
            Some("#0066cc"),
        )
        .unwrap();
        let inbox = ensure_folder(store.conn(), acct, None, "INBOX", Some("inbox")).unwrap();
        assert_eq!(
            ensure_folder(store.conn(), acct, None, "INBOX", Some("inbox")).unwrap(),
            inbox
        );
        assert!(create_account(store.conn(), "Bad", "smtp", None, None, None).is_err());
    }

    #[test]
    fn threading_by_in_reply_to_and_subject() {
        let store = setup();
        let conn = store.conn();
        let acct = create_account(conn, "A", "imap", None, None, None).unwrap();
        let inbox = ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();

        let m1 = MessageInput {
            uid: 1,
            message_id: Some("<1@x>".into()),
            subject: "Project sync".into(),
            from_addr: "a@x".into(),
            date: "2026-04-01T10:00:00+00:00".into(),
            body: "first".into(),
            ..Default::default()
        };
        let id1 = upsert_message(conn, acct, inbox, &m1).unwrap();
        let t1: i64 = conn
            .query_row("SELECT thread_id FROM message WHERE id=?1", [id1], |r| {
                r.get(0)
            })
            .unwrap();

        let m2 = MessageInput {
            uid: 2,
            message_id: Some("<2@x>".into()),
            in_reply_to: Some("<1@x>".into()),
            subject: "Re: Project sync".into(),
            from_addr: "b@x".into(),
            date: "2026-04-01T11:00:00+00:00".into(),
            body: "reply".into(),
            ..Default::default()
        };
        let id2 = upsert_message(conn, acct, inbox, &m2).unwrap();
        let t2: i64 = conn
            .query_row("SELECT thread_id FROM message WHERE id=?1", [id2], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(t1, t2, "in-reply-to threads to parent");

        let m3 = MessageInput {
            uid: 3,
            message_id: Some("<3@x>".into()),
            subject: "Re: project SYNC".into(),
            from_addr: "c@x".into(),
            date: "2026-04-01T12:00:00+00:00".into(),
            body: "reply 2".into(),
            ..Default::default()
        };
        let id3 = upsert_message(conn, acct, inbox, &m3).unwrap();
        let t3: i64 = conn
            .query_row("SELECT thread_id FROM message WHERE id=?1", [id3], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(t1, t3, "normalized subject fallback threads");
    }

    #[test]
    fn eml_ingest_extracts_headers_body_and_counts() {
        let store = setup();
        let conn = store.conn();
        let acct = create_account(conn, "A", "imap", None, None, None).unwrap();
        let inbox = ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();
        let id = ingest_eml(conn, acct, inbox, EML.as_bytes()).unwrap();
        let (subject, from_addr, to, cc, body): (String, String, String, String, String) = conn
            .query_row(
                "SELECT subject, from_addr, to_addrs, cc_addrs, body FROM message WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(subject, "Bakery order");
        assert_eq!(from_addr, "ana@example.com");
        assert!(to.contains("bob@example.com") && to.contains("carol@example.com"));
        assert_eq!(cc, "dan@example.com");
        assert!(body.contains("sourdough"));
        let counts = folder_counts(conn, acct).unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].total, 1);
        assert_eq!(counts[0].unread, 1);
    }

    #[test]
    fn strips_html() {
        assert_eq!(strip_html("<p>Hello <b>world</b></p>"), "Hello world");
    }
}

/// All accounts (for the rail / account wizard list).
#[must_use]
pub fn accounts(conn: &Connection) -> Vec<Account> {
    let Ok(mut stmt) = conn
        .prepare("SELECT id, name, kind, host, port, color, user FROM mail_account ORDER BY id")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(Account {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            host: r.get(3)?,
            port: r.get(4)?,
            color: r.get(5)?,
            user: r.get(6)?,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(std::result::Result::ok).collect()
}

/// All folders of an account (tree panel), with total/unread counts joined.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FolderNode {
    #[serde(flatten)]
    pub folder: Folder,
    pub total: i64,
    pub unread: i64,
}

#[must_use]
pub fn folders_tree(conn: &Connection, account_id: i64) -> Vec<FolderNode> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT f.id, f.account_id, f.parent_id, f.name, f.role, f.last_seen_uid, \
         COALESCE(c.total, 0), COALESCE(c.unread, 0) \
         FROM mail_folder f \
         LEFT JOIN (SELECT folder_id, count(*) AS total, \
           SUM(CASE WHEN json_extract(flags, '$.seen') IS NULL \
               OR json_extract(flags, '$.seen') = 0 THEN 1 ELSE 0 END) AS unread \
           FROM message GROUP BY folder_id) c ON c.folder_id = f.id \
         WHERE f.account_id = ?1 ORDER BY f.name",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([account_id], |r| {
        Ok(FolderNode {
            folder: Folder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                parent_id: r.get(2)?,
                name: r.get(3)?,
                role: r.get(4)?,
                last_seen_uid: r.get(5)?,
            },
            total: r.get(6)?,
            unread: r.get(7)?,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(std::result::Result::ok).collect()
}

/// Full message for the reading pane. HTML is sanitized here (allowlist,
/// `htmlsan`) so no caller can render raw mail HTML.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageDetail {
    pub id: i64,
    pub item_id: Option<i64>,
    pub account_id: i64,
    pub folder_id: i64,
    pub thread_id: Option<i64>,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub date: String,
    pub seen: bool,
    pub body_text: String,
    /// Sanitized HTML (empty when the message has no HTML part).
    pub body_html: String,
}

pub fn message_detail(conn: &Connection, message_id: i64) -> Result<MessageDetail> {
    let row = conn
        .query_row(
            "SELECT item_id, account_id, folder_id, thread_id, subject, from_name, from_addr, \
             to_addrs, cc_addrs, date, json_extract(flags, '$.seen'), body, html_path \
             FROM message WHERE id = ?1",
            [message_id],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, Option<i64>>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::NotFound(message_id))?;
    // Plain-text bodies get an HTML rendering; HTML bodies are sanitized.
    let body_text = row.11.clone();
    let body_html = if body_text.is_empty() {
        row.12
            .as_deref()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .map(|h| crate::htmlsan::sanitize_html(&h))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(MessageDetail {
        id: message_id,
        item_id: row.0,
        account_id: row.1,
        folder_id: row.2,
        thread_id: row.3,
        subject: row.4,
        from_name: row.5,
        from_addr: row.6,
        to_addrs: row.7,
        cc_addrs: row.8,
        date: row.9,
        seen: row.10.unwrap_or(0) != 0,
        body_text,
        body_html,
    })
}

/// Mark a message seen/unseen (flags JSON).
pub fn set_seen(conn: &Connection, message_id: i64, seen: bool) -> Result<()> {
    conn.execute(
        "UPDATE message SET flags = json_set(flags, '$.seen', ?2) WHERE id = ?1",
        params![message_id, i64::from(seen)],
    )?;
    Ok(())
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn message_detail_sanitizes_html_and_sets_seen() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let acct = create_account(conn, "a", "imap", None, None, None).unwrap();
        let inbox = ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();
        let id = upsert_message(
            conn,
            acct,
            inbox,
            &MessageInput {
                uid: 1,
                subject: "Html mail".into(),
                from_name: "Evil".into(),
                from_addr: "e@x.io".into(),
                to_addrs: "me@x.io".into(),
                date: "2026-01-01T00:00:00+00:00".into(),
                html_path: None,
                ..Default::default()
            },
        )
        .unwrap();
        // No text body, html_path points at a hostile HTML file.
        let dir = tempfile::tempdir().unwrap();
        let html_path = dir.path().join("m.html");
        std::fs::write(
            &html_path,
            r#"<p>hi</p><script>alert(1)</script><a href="javascript:bad()">x</a>"#,
        )
        .unwrap();
        conn.execute(
            "UPDATE message SET body = '', html_path = ?2 WHERE id = ?1",
            rusqlite::params![id, html_path.display().to_string()],
        )
        .unwrap();

        let detail = message_detail(conn, id).unwrap();
        assert!(!detail.body_text.is_empty() || detail.body_html.contains("<p>hi</p>"));
        assert!(!detail.body_html.contains("script"), "{}", detail.body_html);
        assert!(!detail.body_html.contains("javascript"));
        assert!(!detail.seen);

        set_seen(conn, id, true).unwrap();
        assert!(message_detail(conn, id).unwrap().seen);
        assert!(
            message_detail(conn, 999).is_err(),
            "missing message is NotFound"
        );
    }

    #[test]
    fn folders_tree_reports_counts() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let acct = create_account(conn, "a", "imap", None, None, None).unwrap();
        let inbox = ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();
        ensure_folder(conn, acct, None, "Archive", None).unwrap();
        for uid in 0..3 {
            upsert_message(
                conn,
                acct,
                inbox,
                &MessageInput {
                    uid,
                    subject: format!("m{uid}"),
                    date: "2026-01-01T00:00:00+00:00".into(),
                    flags: if uid == 0 {
                        "{\"seen\":true}".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let tree = folders_tree(conn, acct);
        assert_eq!(tree.len(), 2);
        let inbox_node = tree.iter().find(|f| f.folder.name == "INBOX").unwrap();
        assert_eq!(inbox_node.total, 3);
        assert_eq!(inbox_node.unread, 2);
        let archive_node = tree.iter().find(|f| f.folder.name == "Archive").unwrap();
        assert_eq!((archive_node.total, archive_node.unread), (0, 0));
        assert!(folders_tree(conn, 999).is_empty());
    }

    #[test]
    fn accounts_listing() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        create_account(conn, "One", "imap", Some("h1"), Some(993), None).unwrap();
        create_account(conn, "Two", "pop3", None, None, Some("#fff")).unwrap();
        let all = accounts(conn);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "One");
        assert_eq!(all[1].color.as_deref(), Some("#fff"));
    }
}

// ------------------------------------------------------- message actions

/// Set/clear one flag (`seen`, `flagged`, `pinned`) in the flags JSON.
pub fn set_flag(conn: &Connection, message_id: i64, flag: &str, value: bool) -> Result<()> {
    let allowed = ["seen", "flagged", "pinned"];
    if !allowed.contains(&flag) {
        return Err(Error::Row {
            row: 0,
            message: format!("unknown flag {flag}"),
        });
    }
    conn.execute(
        "UPDATE message SET flags = json_set(flags, ?2, ?3) WHERE id = ?1",
        params![message_id, format!("$.{flag}"), i64::from(value)],
    )?;
    Ok(())
}

/// Move a message to trash via its anchor item (S8).
pub fn delete_message(conn: &Connection, message_id: i64) -> Result<()> {
    let item_id: i64 = conn
        .query_row(
            "SELECT item_id FROM message WHERE id = ?1",
            [message_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound(message_id))?;
    repo::trash_item(conn, item_id)
}

/// Unified view: the newest messages across ALL folders/accounts (S16),
/// keyset-paginated like `message_page`. `folder_id = 0` means all folders.
pub fn unified_page(
    conn: &Connection,
    after: Option<&pagination::Cursor>,
    limit: i64,
) -> Result<Vec<pagination::MessagePageRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.thread_id, m.subject, m.from_name, m.from_addr, m.date, m.flags, \
         m.snippet FROM message m JOIN item i ON i.id = m.item_id AND i.deleted_at IS NULL \
         WHERE (?1 IS NULL OR (m.date < ?1) OR (m.date = ?1 AND m.id < ?2)) \
         ORDER BY m.date DESC, m.id DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            after.map(|c| c.date.clone()),
            after.map_or(0, |c| c.id),
            limit
        ],
        |r| {
            Ok(pagination::MessagePageRow {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                subject: r.get(2)?,
                from_name: r.get(3)?,
                from_addr: r.get(4)?,
                date: r.get(5)?,
                flags: r.get(6)?,
                snippet: r.get(7)?,
            })
        },
    )?;
    Ok(rows.flatten().collect())
}

/// Filters for `mail_search` (S26).
pub struct SearchFilter<'a> {
    pub folder_id: Option<i64>,
    pub account_id: Option<i64>,
    pub unread_only: bool,
    pub flagged_only: bool,
    pub from: &'a str,
}

/// FTS search restricted to mail with SQL filters applied post-match.
pub fn mail_search(
    conn: &Connection,
    query: &str,
    filter: &SearchFilter<'_>,
    limit: i64,
) -> Result<Vec<pagination::MessagePageRow>> {
    let q = crate::search::fts_escape(query);
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT m.id, m.thread_id, m.subject, m.from_name, m.from_addr, m.date, m.flags, \
         m.snippet FROM message_fts f \
         JOIN message m ON m.id = f.rowid \
         JOIN item i ON i.id = m.item_id AND i.deleted_at IS NULL \
         WHERE message_fts MATCH ?1 \
         AND (?2 IS NULL OR m.folder_id = ?2) \
         AND (?3 IS NULL OR m.account_id = ?3) \
         AND (?4 = 0 OR json_extract(m.flags, '$.seen') IS NULL \
              OR json_extract(m.flags, '$.seen') = 0) \
         AND (?5 = 0 OR json_extract(m.flags, '$.flagged') = 1) \
         AND (?6 = '' OR m.from_addr LIKE '%' || ?6 || '%' OR m.from_name LIKE '%' || ?6 || '%') \
         ORDER BY rank LIMIT ?7",
    )?;
    let rows = stmt.query_map(
        params![
            q,
            filter.folder_id,
            filter.account_id,
            i64::from(filter.unread_only),
            i64::from(filter.flagged_only),
            filter.from,
            limit,
        ],
        |r| {
            Ok(pagination::MessagePageRow {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                subject: r.get(2)?,
                from_name: r.get(3)?,
                from_addr: r.get(4)?,
                date: r.get(5)?,
                flags: r.get(6)?,
                snippet: r.get(7)?,
            })
        },
    )?;
    Ok(rows.flatten().collect())
}

/// All messages of one thread (threading UI, S18), oldest first.
pub fn thread_messages(
    conn: &Connection,
    thread_id: i64,
) -> Result<Vec<pagination::MessagePageRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.thread_id, m.subject, m.from_name, m.from_addr, m.date, m.flags, \
         m.snippet FROM message m JOIN item i ON i.id = m.item_id AND i.deleted_at IS NULL \
         WHERE m.thread_id = ?1 ORDER BY m.date, m.id",
    )?;
    let rows = stmt.query_map([thread_id], |r| {
        Ok(pagination::MessagePageRow {
            id: r.get(0)?,
            thread_id: r.get(1)?,
            subject: r.get(2)?,
            from_name: r.get(3)?,
            from_addr: r.get(4)?,
            date: r.get(5)?,
            flags: r.get(6)?,
            snippet: r.get(7)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

/// Convert a message into a Task or Note (S24) with a Related Items
/// backlink (symmetric via trigger). Returns the new entity's row id.
pub fn convert_message(
    conn: &Connection,
    message_id: i64,
    to: ConvertTarget,
) -> Result<(i64, i64)> {
    let m = message_detail(conn, message_id)?;
    let item = m.item_id.ok_or_else(|| Error::Row {
        row: 0,
        message: "message without anchor".into(),
    })?;
    let body = if m.body_text.is_empty() {
        m.body_html.clone()
    } else {
        m.body_text.clone()
    };
    let prefix = format!("From message: {}", m.subject);
    let new_id = match to {
        ConvertTarget::Task => {
            let item_id = repo::create_item(conn, Kind::Task, None, false)?;
            conn.execute(
                "INSERT INTO task (item_id, title, notes) VALUES (?1, ?2, ?3)",
                params![item_id, m.subject, body],
            )?;
            conn.last_insert_rowid()
        }
        ConvertTarget::Note => {
            let item_id = repo::create_item(conn, Kind::Note, None, false)?;
            conn.execute(
                "INSERT INTO note (item_id, title, content) VALUES (?1, ?2, ?3)",
                params![
                    item_id,
                    m.subject,
                    format!("<p>{}</p>", body.replace('\n', "<br>"))
                ],
            )?;
            conn.last_insert_rowid()
        }
        ConvertTarget::Event => {
            let cal = crate::events::ensure_calendar(conn, "From Mail", None)?;
            let record = crate::ical::EventRecord {
                uid: None,
                title: m.subject.clone(),
                start: crate::model::now_iso(),
                end: None,
                all_day: false,
                location: None,
                description: Some(format!("{prefix}\n\n{body}")),
                rrule: None,
                categories: Vec::new(),
                private: false,
            };
            crate::events::upsert_event(conn, cal, &record)?
        }
    };
    let new_item: i64 = match to {
        ConvertTarget::Task => {
            conn.query_row("SELECT item_id FROM task WHERE id = ?1", [new_id], |r| {
                r.get(0)
            })
        }
        ConvertTarget::Note => {
            conn.query_row("SELECT item_id FROM note WHERE id = ?1", [new_id], |r| {
                r.get(0)
            })
        }
        ConvertTarget::Event => {
            conn.query_row("SELECT item_id FROM event WHERE id = ?1", [new_id], |r| {
                r.get(0)
            })
        }
    }?;
    repo::link_items(conn, item, new_item)?;
    Ok((new_id, new_item))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertTarget {
    Task,
    Note,
    Event,
}

#[cfg(test)]
mod action_tests {
    use super::*;
    use crate::store::Store;

    fn seed() -> (Store, i64) {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let acct = create_account(conn, "a", "imap", None, None, None).unwrap();
        let inbox = ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();
        let id = upsert_message(
            conn,
            acct,
            inbox,
            &MessageInput {
                uid: 1,
                subject: "Follow up".into(),
                from_name: "Ana".into(),
                from_addr: "ana@x.io".into(),
                to_addrs: "me@x.io".into(),
                date: "2026-01-01T00:00:00+00:00".into(),
                body: "please review".into(),
                ..Default::default()
            },
        )
        .unwrap();
        (s, id)
    }

    #[test]
    fn flags_and_delete() {
        let (s, id) = seed();
        let conn = s.conn();
        set_flag(conn, id, "flagged", true).unwrap();
        set_flag(conn, id, "seen", true).unwrap();
        let detail = message_detail(conn, id).unwrap();
        assert!(detail.seen);
        let flags: String = conn
            .query_row("SELECT flags FROM message WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert!(flags.contains("\"flagged\":1"));
        assert!(
            set_flag(conn, id, "nope", true).is_err(),
            "unknown flags rejected"
        );

        delete_message(conn, id).unwrap();
        let trash = repo::query_trash(
            conn,
            &repo::TrashQuery {
                kind: Some(Kind::Message),
                deleted_after: None,
                deleted_before: None,
                limit: 5,
            },
        )
        .unwrap();
        assert_eq!(trash.len(), 1);
    }

    #[test]
    fn unified_page_and_search_filters() {
        let (s, id) = seed();
        let conn = s.conn();
        assert_eq!(unified_page(conn, None, 10).unwrap().len(), 1);

        // Unread filter sees it; after marking seen it disappears.
        assert_eq!(
            mail_search(
                conn,
                "review",
                &SearchFilter {
                    folder_id: None,
                    account_id: None,
                    unread_only: true,
                    flagged_only: false,
                    from: ""
                },
                10,
            )
            .unwrap()
            .len(),
            1
        );
        set_flag(conn, id, "seen", true).unwrap();
        assert_eq!(
            mail_search(
                conn,
                "review",
                &SearchFilter {
                    folder_id: None,
                    account_id: None,
                    unread_only: true,
                    flagged_only: false,
                    from: ""
                },
                10,
            )
            .unwrap()
            .len(),
            0
        );
        assert_eq!(
            mail_search(
                conn,
                "review",
                &SearchFilter {
                    folder_id: Some(999),
                    account_id: None,
                    unread_only: false,
                    flagged_only: false,
                    from: ""
                },
                10,
            )
            .unwrap()
            .len(),
            0,
            "folder filter excludes"
        );
        assert_eq!(
            mail_search(
                conn,
                "review",
                &SearchFilter {
                    folder_id: None,
                    account_id: None,
                    unread_only: false,
                    flagged_only: false,
                    from: "ana"
                },
                10,
            )
            .unwrap()
            .len(),
            1,
            "from filter matches"
        );
    }

    #[test]
    fn convert_creates_linked_entities() {
        let (s, id) = seed();
        let conn = s.conn();
        let (task_id, task_item) = convert_message(conn, id, ConvertTarget::Task).unwrap();
        let title: String = conn
            .query_row("SELECT title FROM task WHERE id=?1", [task_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "Follow up");
        let linked = repo::related_items(conn, task_item);
        let msg_item: i64 = conn
            .query_row("SELECT item_id FROM message WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(linked.contains(&msg_item), "backlink present both ways");

        let (note_id, _) = convert_message(conn, id, ConvertTarget::Note).unwrap();
        let content: String = conn
            .query_row("SELECT content FROM note WHERE id=?1", [note_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(content.contains("please review"));

        let (event_id, _) = convert_message(conn, id, ConvertTarget::Event).unwrap();
        let title: String = conn
            .query_row("SELECT title FROM event WHERE id=?1", [event_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "Follow up");
    }
}
