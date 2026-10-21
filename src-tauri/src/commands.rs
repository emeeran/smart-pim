//! IPC command surface. Every command is a thin wrapper over a plain function
//! taking `&Connection`, so the shell logic is unit-testable without a
//! running Tauri app. Errors cross the bridge as strings (typed, no panics).

use std::path::Path;

use pfcore::repo::{self, UndoOutcome};
use pfcore::search::{self, SearchHit, SearchScope};
use pfcore::store::Store;
use pfcore::{model, pagination};
use rusqlite::Connection;
use serde::Serialize;

use crate::state::{AUTO_LOCK_DEFAULT, AUTO_LOCK_KEY};

// ------------------------------------------------------------- unlock / lock

/// Outcome of opening the database. `WrongPassphrase` is a retry prompt (S7),
/// not a fatal error.
pub enum UnlockResult {
    Ok { store: Store, created: bool },
    WrongPassphrase,
    Failed(String),
}

impl UnlockResult {
    /// The serializable part for the UI.
    pub fn outcome(&self) -> UnlockOutcome {
        match self {
            UnlockResult::Ok { created, .. } => UnlockOutcome {
                ok: true,
                created: *created,
                error: None,
            },
            UnlockResult::WrongPassphrase => UnlockOutcome {
                ok: false,
                created: false,
                error: Some("wrong passphrase".into()),
            },
            UnlockResult::Failed(e) => UnlockOutcome {
                ok: false,
                created: false,
                error: Some(e.clone()),
            },
        }
    }

    pub fn take_store(self) -> Option<Store> {
        match self {
            UnlockResult::Ok { store, .. } => Some(store),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UnlockOutcome {
    pub ok: bool,
    /// True when the database file did not exist and was just created.
    pub created: bool,
    pub error: Option<String>,
}

/// Open (or create) the store for `path`. Pure, testable; the Tauri command
/// stores the handle in managed state.
pub fn unlock_db(path: &Path, passphrase: &str) -> UnlockResult {
    // Capture existence BEFORE opening: a fresh open creates the file.
    let existed = path.exists();
    let store = match Store::open(path, passphrase) {
        Ok(s) => s,
        Err(pfcore::Error::Sqlite(rusqlite::Error::SqliteFailure(code, _)))
            if code.code == rusqlite::ErrorCode::NotADatabase =>
        {
            return UnlockResult::WrongPassphrase;
        }
        Err(e) => return UnlockResult::Failed(e.to_string()),
    };
    UnlockResult::Ok {
        store,
        created: !existed,
    }
}

// ------------------------------------------------------------- shell queries

/// Badge + status-bar numbers (S2/S5): active items per kind, trash size,
/// unread mail, due reminders.
#[derive(Debug, Serialize)]
pub struct Counts {
    pub notes: i64,
    pub tasks: i64,
    pub diary: i64,
    pub contacts: i64,
    pub passwords: i64,
    pub events: i64,
    pub messages: i64,
    pub unread_messages: i64,
    pub trash: i64,
    pub pending_reminders: i64,
}

pub fn shell_counts(conn: &Connection) -> pfcore::Result<Counts> {
    let mut by_kind = std::collections::HashMap::<String, i64>::new();
    let mut stmt =
        conn.prepare("SELECT kind, count(*) FROM item WHERE deleted_at IS NULL GROUP BY kind")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for (kind, n) in rows.flatten() {
        by_kind.insert(kind, n);
    }
    let get = |k: &str| by_kind.get(k).copied().unwrap_or(0);
    Ok(Counts {
        notes: get("note"),
        tasks: get("task"),
        diary: get("diary"),
        contacts: get("contact"),
        passwords: get("password"),
        events: get("event"),
        messages: get("message"),
        unread_messages: conn.query_row(
            "SELECT count(*) FROM message \
             WHERE json_extract(flags, '$.seen') IS NULL OR json_extract(flags, '$.seen') = 0",
            [],
            |r| r.get(0),
        )?,
        trash: conn.query_row(
            "SELECT count(*) FROM item WHERE deleted_at IS NOT NULL",
            [],
            |r| r.get(0),
        )?,
        pending_reminders: conn.query_row(
            "SELECT count(*) FROM reminder_queue WHERE fired_at IS NULL AND due_at <= ?1",
            [model::now_iso()],
            |r| r.get(0),
        )?,
    })
}

pub fn do_global_search(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> pfcore::Result<Vec<SearchHit>> {
    search::global_search(conn, query, SearchScope::all(), limit)
}

pub fn do_messages_page(
    conn: &Connection,
    folder_id: i64,
    cursor_date: Option<String>,
    cursor_id: Option<i64>,
    limit: i64,
) -> pfcore::Result<Vec<pagination::MessagePageRow>> {
    let cursor = cursor_date
        .zip(cursor_id)
        .map(|(date, id)| pagination::Cursor { date, id });
    pagination::message_page(conn, folder_id, cursor.as_ref(), limit)
}

pub fn do_tasks_page(
    conn: &Connection,
    after_id: Option<i64>,
    limit: i64,
) -> pfcore::Result<Vec<pagination::TaskPageRow>> {
    pagination::task_page(conn, after_id, limit)
}

// ------------------------------------------------------------- trash (S8)

pub fn do_trash(
    conn: &Connection,
    kind: Option<String>,
    limit: i64,
) -> pfcore::Result<Vec<model::Item>> {
    let kind = kind.as_deref().and_then(model::Kind::parse);
    let q = repo::TrashQuery {
        kind,
        deleted_after: None,
        deleted_before: None,
        limit,
    };
    repo::query_trash(conn, &q)
}

// ------------------------------------------------------------- settings

pub fn settings_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

pub fn settings_set(conn: &Connection, key: &str, value: &str) -> pfcore::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Auto-lock timeout in minutes (0 = disabled). Missing key → default (S7).
pub fn auto_lock_minutes(conn: &Connection) -> i64 {
    settings_get(conn, AUTO_LOCK_KEY)
        .and_then(|v| v.parse().ok())
        .unwrap_or(AUTO_LOCK_DEFAULT)
}

pub fn do_undo(conn: &Connection) -> pfcore::Result<Option<UndoOutcome>> {
    repo::undo_last(conn)
}

// ------------------------------------------------------------- related items

/// A linked item resolved to a display label for the detail-pane
/// "Related Items" tab (T2.7).
#[derive(serde::Serialize)]
pub struct LinkedItem {
    pub id: i64,
    pub kind: String,
    pub title: String,
}

fn item_label(conn: &Connection, id: i64) -> Option<(String, String)> {
    let kind: String = conn
        .query_row("SELECT kind FROM item WHERE id = ?1", [id], |r| r.get(0))
        .ok()?;
    let title = match kind.as_str() {
        "task" => conn.query_row("SELECT title FROM task WHERE item_id = ?1", [id], |r| {
            r.get::<_, String>(0)
        }),
        "note" => conn.query_row("SELECT title FROM note WHERE item_id = ?1", [id], |r| {
            r.get::<_, String>(0)
        }),
        "event" => conn.query_row("SELECT title FROM event WHERE item_id = ?1", [id], |r| {
            r.get::<_, String>(0)
        }),
        "contact" => conn.query_row(
            "SELECT display_name FROM contact WHERE item_id = ?1",
            [id],
            |r| r.get::<_, String>(0),
        ),
        "password" => conn.query_row("SELECT title FROM pw_entry WHERE item_id = ?1", [id], |r| {
            r.get::<_, String>(0)
        }),
        "message" => conn.query_row(
            "SELECT subject FROM message WHERE item_id = ?1",
            [id],
            |r| r.get::<_, String>(0),
        ),
        _ => Ok(kind.clone()),
    }
    .unwrap_or_else(|_| format!("Item #{id}"));
    Some((kind, title))
}

pub fn do_item_links(conn: &Connection, item_id: i64) -> pfcore::Result<Vec<LinkedItem>> {
    Ok(repo::related_items(conn, item_id)
        .into_iter()
        .filter_map(|id| item_label(conn, id).map(|(kind, title)| LinkedItem { id, kind, title }))
        .collect())
}

pub fn do_item_link(conn: &Connection, item_id: i64, other_id: i64) -> pfcore::Result<()> {
    repo::link_items(conn, item_id, other_id)
}

pub fn do_item_unlink(conn: &Connection, item_id: i64, other_id: i64) -> pfcore::Result<()> {
    repo::unlink_items(conn, item_id, other_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfcore::model::Kind;

    fn store() -> Store {
        Store::open_memory("t").unwrap()
    }

    #[test]
    fn unlock_wrong_passphrase_is_retry_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let out = unlock_db(&path, "correct horse");
            let UnlockResult::Ok { created, .. } = &out else {
                panic!("first unlock must succeed");
            };
            assert!(*created);
            drop(out.take_store().unwrap());
        }
        let bad = unlock_db(&path, "wrong");
        assert!(matches!(bad, UnlockResult::WrongPassphrase));
        assert_eq!(bad.outcome().error.as_deref(), Some("wrong passphrase"));
        let good = unlock_db(&path, "correct horse");
        assert!(matches!(&good, UnlockResult::Ok { created: false, .. }));
    }

    #[test]
    fn counts_shape_and_trash_math() {
        let s = store();
        let conn = s.conn();
        let a = repo::create_item(conn, Kind::Note, None, false).unwrap();
        let _b = repo::create_item(conn, Kind::Note, None, false).unwrap();
        repo::trash_item(conn, a).unwrap();
        let c = shell_counts(conn).unwrap();
        assert_eq!(c.notes, 1, "only active items count for badges");
        assert_eq!(c.trash, 1);
    }

    #[test]
    fn settings_roundtrip_and_autolock_default() {
        let s = store();
        let conn = s.conn();
        assert_eq!(auto_lock_minutes(conn), AUTO_LOCK_DEFAULT);
        settings_set(conn, AUTO_LOCK_KEY, "0").unwrap();
        assert_eq!(auto_lock_minutes(conn), 0);
        settings_set(conn, AUTO_LOCK_KEY, "5").unwrap();
        assert_eq!(auto_lock_minutes(conn), 5);
        assert!(settings_get(conn, "missing").is_none());
    }

    #[test]
    fn idle_math() {
        let past = std::time::Instant::now() - std::time::Duration::from_secs(3600);
        assert!(crate::state::idle_exceeded(past, 15));
        assert!(
            !crate::state::idle_exceeded(past, 0),
            "0 disables auto-lock"
        );
        assert!(!crate::state::idle_exceeded(std::time::Instant::now(), 15));
    }

    #[test]
    fn search_pages_and_trash_through_commands() {
        let s = store();
        let conn = s.conn();
        let note_item = repo::create_item(conn, Kind::Note, None, false).unwrap();
        conn.execute(
            "INSERT INTO note (item_id, title, content) VALUES (?1, 'Hardware list', 'usb-c dock')",
            [note_item],
        )
        .unwrap();
        let hits = do_global_search(conn, "usb", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(do_messages_page(conn, 999, None, None, 10)
            .unwrap()
            .is_empty());
        assert!(do_tasks_page(conn, None, 10).unwrap().is_empty());
        repo::trash_item(conn, note_item).unwrap();
        let trashed = do_trash(conn, Some("note".into()), 10).unwrap();
        assert_eq!(trashed.len(), 1);
        assert_eq!(do_trash(conn, Some("task".into()), 10).unwrap().len(), 0);
    }
}
