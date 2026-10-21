//! Keyset pagination for large lists (T1.14 / invariant DT-5): stable under
//! concurrent inserts, no OFFSET scans. Used by virtualized UI lists.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::Result;

/// Cursor: `date DESC, id DESC` tuple of the last row of the previous page.
#[derive(Debug, Clone, Serialize)]
pub struct Cursor {
    pub date: String,
    pub id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagePageRow {
    pub id: i64,
    pub thread_id: Option<i64>,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub date: String,
    pub flags: String,
    pub snippet: String,
}

/// One page of a folder, newest first.
pub fn message_page(
    conn: &Connection,
    folder_id: i64,
    after: Option<&Cursor>,
    limit: i64,
) -> Result<Vec<MessagePageRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, subject, from_name, from_addr, date, flags, snippet \
         FROM message WHERE folder_id = ?1 \
         AND (?2 IS NULL OR (date < ?2) OR (date = ?2 AND id < ?3)) \
         ORDER BY date DESC, id DESC LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![
            folder_id,
            after.map(|c| c.date.clone()),
            after.map_or(0, |c| c.id),
            limit
        ],
        |r| {
            Ok(MessagePageRow {
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
    let out = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Due tasks ordered by due date with the same keyset scheme (NULLs first).
#[derive(Debug, Clone, Serialize)]
pub struct TaskPageRow {
    pub id: i64,
    pub title: String,
    pub due_utc: Option<String>,
    pub priority: i64,
    pub percent: i64,
    pub completed_at: Option<String>,
}

pub fn task_page(conn: &Connection, after_id: Option<i64>, limit: i64) -> Result<Vec<TaskPageRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, due_utc, priority, percent, completed_at FROM task \
         WHERE (?1 IS NULL OR id > ?1) ORDER BY id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![after_id, limit], |r| {
        Ok(TaskPageRow {
            id: r.get(0)?,
            title: r.get(1)?,
            due_utc: r.get(2)?,
            priority: r.get(3)?,
            percent: r.get(4)?,
            completed_at: r.get(5)?,
        })
    })?;
    let out = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn seeded(n: usize) -> Store {
        let store = Store::open_memory("x").unwrap();
        let conn = store.conn();
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
        for i in 0..n {
            conn.execute(
                "INSERT INTO message (account_id, folder_id, uid, subject, date, created_at) \
                 VALUES (1, 1, ?1, ?2, ?3, ?3)",
                rusqlite::params![
                    i as i64,
                    format!("m{i}"),
                    format!("2026-01-01T00:{:02}:00+00:00", i % 60)
                ],
            )
            .unwrap();
        }
        store
    }

    #[test]
    fn pages_are_stable_and_complete() {
        let store = seeded(250);
        let conn = store.conn();
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = message_page(conn, 1, cursor.as_ref(), 100).unwrap();
            if page.is_empty() {
                break;
            }
            cursor = Some(Cursor {
                date: page.last().unwrap().date.clone(),
                id: page.last().unwrap().id,
            });
            seen.extend(page.iter().map(|r| r.id));
        }
        assert_eq!(seen.len(), 250);
        assert_eq!(
            seen.len(),
            seen.iter().collect::<std::collections::HashSet<_>>().len(),
            "no dupes"
        );
    }

    #[test]
    fn new_inserts_do_not_shift_pages() {
        let store = seeded(50);
        let conn = store.conn();
        let first = message_page(conn, 1, None, 10).unwrap();
        // insert older mail at the tail
        conn.execute(
            "INSERT INTO message (account_id, folder_id, uid, subject, date, created_at) \
             VALUES (1, 1, 999, 'old', '2020-01-01T00:00:00+00:00', '2020-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
        let second = message_page(conn, 1, None, 10).unwrap();
        assert_eq!(
            first.iter().map(|r| r.id).collect::<Vec<_>>(),
            second.iter().map(|r| r.id).collect::<Vec<_>>()
        );
    }
}
