//! Unified full-text search across modules (FTS5 external-content tables).
//! The 100k-message <200 ms budget (spec N1 / AT-5) is exercised in tests.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::Result;

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub kind: &'static str,
    pub id: i64,
    pub title: String,
    /// FTS `snippet()` extract around the first match, when available.
    pub snippet: Option<String>,
}

/// Sanitize user input into an FTS5 query: keep word characters, quote the
/// result, and AND the terms. Prevents malformed-MATCH errors from raw input
/// like `help:` or `"unbalanced`.
#[must_use]
pub fn fts_escape(input: &str) -> String {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| {
            let cleaned: String = t
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            format!("\"{cleaned}\"")
        })
        .filter(|t| t != "\"\"")
        .collect();
    terms.join(" ")
}

#[derive(Debug, Default, Clone, Copy)]
// The scope flags are genuinely independent module filters, not states.
#[allow(clippy::struct_excessive_bools)]
pub struct SearchScope {
    pub mail: bool,
    pub tasks: bool,
    pub notes: bool,
    pub diary: bool,
}

impl SearchScope {
    #[must_use]
    pub fn all() -> Self {
        Self {
            mail: true,
            tasks: true,
            notes: true,
            diary: true,
        }
    }
}

pub fn global_search(
    conn: &Connection,
    query: &str,
    scope: SearchScope,
    limit: i64,
) -> Result<Vec<SearchHit>> {
    let q = fts_escape(query);
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    if scope.mail {
        collect(
            conn,
            &mut hits,
            "message",
            "SELECT m.id, m.subject, snippet(message_fts, 4, '[', ']', '…', 12) \
             FROM message_fts f JOIN message m ON m.id = f.rowid \
             WHERE message_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            &q,
            limit,
        )?;
    }
    if scope.tasks {
        collect(
            conn,
            &mut hits,
            "task",
            "SELECT t.id, t.title, NULL FROM task_fts f JOIN task t ON t.id = f.rowid \
             WHERE task_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            &q,
            limit,
        )?;
    }
    if scope.notes {
        collect(
            conn,
            &mut hits,
            "note",
            "SELECT n.id, n.title, snippet(note_fts, 1, '[', ']', '…', 12) \
             FROM note_fts f JOIN note n ON n.id = f.rowid \
             WHERE note_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            &q,
            limit,
        )?;
    }
    if scope.diary {
        collect(
            conn,
            &mut hits,
            "diary",
            "SELECT d.id, d.entry_date, snippet(diary_fts, 0, '[', ']', '…', 12) \
             FROM diary_fts f JOIN diary_entry d ON d.id = f.rowid \
             JOIN item i ON i.id = d.item_id \
             WHERE diary_fts MATCH ?1 AND i.deleted_at IS NULL ORDER BY rank LIMIT ?2",
            &q,
            limit,
        )?;
    }
    hits.truncate(limit as usize);
    Ok(hits)
}

fn collect(
    conn: &Connection,
    out: &mut Vec<SearchHit>,
    kind: &'static str,
    sql: &str,
    query: &str,
    limit: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![query, limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (id, title, snippet) = row?;
        out.push(SearchHit {
            kind,
            id,
            title,
            snippet,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::schema::apply_all(&mut conn).unwrap();
        conn
    }

    #[test]
    fn escape_handles_hostile_input() {
        assert_eq!(fts_escape("help: \"unbalanced"), "\"help\" \"unbalanced\"");
        assert_eq!(fts_escape("   "), "");
        assert_eq!(fts_escape("hello_world 42"), "\"hello_world\" \"42\"");
    }

    #[test]
    fn search_across_modules() {
        let conn = seed();
        conn.execute(
            "INSERT INTO mail_account (name, kind) VALUES ('a', 'imap')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mail_folder (account_id, name) VALUES (1, 'INBOX')",
            [],
        )
        .unwrap();
        let note_item =
            crate::repo::create_item(&conn, crate::model::Kind::Note, None, false).unwrap();
        let task_item =
            crate::repo::create_item(&conn, crate::model::Kind::Task, None, false).unwrap();
        conn.execute(
            "INSERT INTO note (item_id, title, content) VALUES (?1, 'Recipes', 'sourdough starter tips')",
            [note_item],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task (item_id, title, notes) VALUES (?1, 'sourdough schedule', NULL)",
            [task_item],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (account_id, folder_id, uid, subject, from_name, body, date, created_at) \
             VALUES (1, 1, 1, 'bakery order', 'Ana', 'sourdough loaves available', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        let hits = global_search(&conn, "sourdough", SearchScope::all(), 10).unwrap();
        assert_eq!(hits.len(), 3);
        let kinds: Vec<_> = hits.iter().map(|h| h.kind).collect();
        assert!(kinds.contains(&"mail") || hits.iter().any(|h| h.kind == "message"));
        // empty / hostile queries are safe
        assert!(global_search(&conn, "", SearchScope::all(), 10)
            .unwrap()
            .is_empty());
        assert!(global_search(&conn, "help: (", SearchScope::all(), 10)
            .unwrap()
            .is_empty());
    }
}
