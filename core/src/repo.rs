//! Repositories for the universal item model: items, lists, tags, cross-links,
//! version history, audit log, undo log, trash.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};
use crate::model::{now_iso, Item, Kind, List, Tag, Version};

fn row_to_item(r: &Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get("id")?,
        kind: Kind::parse(&r.get::<_, String>("kind")?)
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        deleted_at: r.get("deleted_at")?,
        private: r.get::<_, i64>("private")? != 0,
        list_id: r.get("list_id")?,
        sort_key: r.get("sort_key")?,
    })
}

const ITEM_COLS: &str = "id, kind, created_at, updated_at, deleted_at, private, list_id, sort_key";

// ---------------------------------------------------------------- items

pub fn create_item(
    conn: &Connection,
    kind: Kind,
    list_id: Option<i64>,
    private: bool,
) -> Result<i64> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO item (kind, created_at, updated_at, private, list_id, guid) \
         VALUES (?1, ?2, ?2, ?3, ?4, lower(hex(randomblob(16))))",
        params![kind.as_str(), now, i64::from(private), list_id],
    )?;
    let id = conn.last_insert_rowid();
    audit(
        conn,
        Some(id),
        "create_item",
        &json!({ "kind": kind.as_str() }),
    )?;
    record_undo(conn, "create_item", &json!({ "item_id": id }))?;
    Ok(id)
}

pub fn get_item(conn: &Connection, id: i64) -> Result<Item> {
    conn.query_row(
        &format!("SELECT {ITEM_COLS} FROM item WHERE id = ?1"),
        [id],
        row_to_item,
    )
    .map_err(|_| Error::NotFound(id))
}

fn query_items(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Item>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(p, row_to_item)?;
    let out = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

#[must_use]
pub fn touch_item(conn: &Connection, id: i64) -> bool {
    conn.execute(
        "UPDATE item SET updated_at = ?1 WHERE id = ?2",
        params![now_iso(), id],
    )
    .is_ok_and(|n| n > 0)
}

/// Snapshot the current scalar state into `item_version`, then update.
pub fn update_item_scalars(
    conn: &Connection,
    id: i64,
    list_id: Option<i64>,
    private: bool,
    sort_key: Option<&str>,
) -> Result<()> {
    let before = get_item(conn, id)?;
    snapshot_item(conn, &before)?;
    let now = now_iso();
    conn.execute(
        "UPDATE item SET list_id = ?1, private = ?2, sort_key = ?3, updated_at = ?4 WHERE id = ?5",
        params![list_id, i64::from(private), sort_key, now, id],
    )?;
    audit(
        conn,
        Some(id),
        "update_item",
        &json!({ "private": private }),
    )?;
    record_undo(
        conn,
        "update_item",
        &json!({ "item_id": id, "list_id": before.list_id, "private": before.private,
                "sort_key": before.sort_key }),
    )?;
    Ok(())
}

// ---------------------------------------------------------------- trash

pub fn trash_item(conn: &Connection, id: i64) -> Result<()> {
    let now = now_iso();
    // `updated_at` moves too: the tombstone must win last-writer-wins on
    // other devices (sync), and the change trigger stamps `at` from it.
    let n = conn.execute(
        "UPDATE item SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 \
         AND deleted_at IS NULL",
        params![now, id],
    )?;
    if n == 0 {
        return Err(Error::NotFound(id));
    }
    audit(conn, Some(id), "trash_item", &json!({ "at": now }))?;
    record_undo(conn, "trash_item", &json!({ "item_id": id }))?;
    Ok(())
}

pub fn restore_item(conn: &Connection, id: i64) -> Result<()> {
    let now = now_iso();
    let n = conn.execute(
        "UPDATE item SET deleted_at = NULL, updated_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    if n == 0 {
        return Err(Error::NotFound(id));
    }
    audit(conn, Some(id), "restore_item", &json!({}))?;
    record_undo(conn, "restore_item", &json!({ "item_id": id }))?;
    Ok(())
}

/// Hard-delete an item and everything hanging off it (FK cascades).
pub fn purge_item(conn: &Connection, id: i64) -> Result<()> {
    let n = conn.execute("DELETE FROM item WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(Error::NotFound(id));
    }
    audit(conn, None, "purge_item", &json!({ "item_id": id }))?;
    Ok(())
}

pub struct TrashQuery {
    pub kind: Option<Kind>,
    pub deleted_after: Option<String>,
    pub deleted_before: Option<String>,
    pub limit: i64,
}

pub fn query_trash(conn: &Connection, q: &TrashQuery) -> Result<Vec<Item>> {
    let mut clauses = vec!["deleted_at IS NOT NULL".to_string()];
    if let Some(k) = q.kind {
        clauses.push(format!("kind = '{}'", k.as_str()));
    }
    if let Some(a) = &q.deleted_after {
        clauses.push(format!("deleted_at > '{a}'"));
    }
    if let Some(b) = &q.deleted_before {
        clauses.push(format!("deleted_at < '{b}'"));
    }
    let sql = format!(
        "SELECT {ITEM_COLS} FROM item WHERE {} ORDER BY deleted_at DESC LIMIT {}",
        clauses.join(" AND "),
        q.limit
    );
    query_items(conn, &sql, [])
}

// ---------------------------------------------------------------- lists

pub fn create_list(
    conn: &Connection,
    kind: &str,
    parent_id: Option<i64>,
    name: &str,
    color: Option<&str>,
    sort_order: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO item_list (kind, parent_id, name, color, sort_order) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![kind, parent_id, name, color, sort_order],
    )?;
    Ok(conn.last_insert_rowid())
}

fn row_to_list(r: &Row<'_>) -> rusqlite::Result<List> {
    Ok(List {
        id: r.get(0)?,
        kind: r.get(1)?,
        parent_id: r.get(2)?,
        name: r.get(3)?,
        color: r.get(4)?,
        icon: r.get(5)?,
        sort_order: r.get(6)?,
        settings: r.get(7)?,
    })
}

#[must_use]
pub fn lists_of_kind(conn: &Connection, kind: &str) -> Vec<List> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, kind, parent_id, name, color, icon, sort_order, settings \
         FROM item_list WHERE kind = ?1 ORDER BY sort_order, id",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([kind], row_to_list);
    match rows {
        Ok(rows) => rows.filter_map(std::result::Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- tags

pub fn ensure_tag(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute("INSERT OR IGNORE INTO tag (name) VALUES (?1)", [name])?;
    let id: i64 = conn.query_row("SELECT id FROM tag WHERE name = ?1", [name], |r| r.get(0))?;
    Ok(id)
}

pub fn tag_item(conn: &Connection, item_id: i64, tag: &str) -> Result<()> {
    let tag_id = ensure_tag(conn, tag)?;
    conn.execute(
        "INSERT OR IGNORE INTO item_tag (item_id, tag_id) VALUES (?1, ?2)",
        params![item_id, tag_id],
    )?;
    record_undo(
        conn,
        "tag",
        &json!({ "item_id": item_id, "tag_id": tag_id }),
    )?;
    audit(conn, Some(item_id), "tag", &json!({ "tag": tag }))?;
    Ok(())
}

pub fn untag_item(conn: &Connection, item_id: i64, tag: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM item_tag WHERE item_id = ?1 AND tag_id = (SELECT id FROM tag WHERE name = ?2)",
        params![item_id, tag],
    )?;
    audit(conn, Some(item_id), "untag", &json!({ "tag": tag }))?;
    Ok(())
}

#[must_use]
pub fn tags_of_item(conn: &Connection, item_id: i64) -> Vec<Tag> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT t.id, t.name, t.color FROM tag t \
         JOIN item_tag it ON it.tag_id = t.id WHERE it.item_id = ?1 ORDER BY t.name",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([item_id], |r| {
        Ok(Tag {
            id: r.get(0)?,
            name: r.get(1)?,
            color: r.get(2)?,
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(std::result::Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- links

pub fn link_items(conn: &Connection, from_id: i64, to_id: i64) -> Result<()> {
    if from_id == to_id {
        return Err(Error::Row {
            row: 0,
            message: "cannot link an item to itself".into(),
        });
    }
    for id in [from_id, to_id] {
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM item WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        if exists.is_none() {
            return Err(Error::NotFound(id));
        }
    }
    conn.execute(
        "INSERT OR IGNORE INTO item_link (from_id, to_id) VALUES (?1, ?2)",
        params![from_id, to_id],
    )?;
    record_undo(conn, "link", &json!({ "from_id": from_id, "to_id": to_id }))?;
    audit(conn, Some(from_id), "link", &json!({ "to": to_id }))?;
    Ok(())
}

pub fn unlink_items(conn: &Connection, a: i64, b: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM item_link WHERE (from_id = ?1 AND to_id = ?2) \
             OR (from_id = ?2 AND to_id = ?1)",
        params![a, b],
    )?;
    audit(conn, Some(a), "unlink", &json!({ "to": b }))?;
    Ok(())
}

#[must_use]
pub fn related_items(conn: &Connection, item_id: i64) -> Vec<i64> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT DISTINCT CASE WHEN from_id = ?1 THEN to_id ELSE from_id END \
         FROM item_link WHERE from_id = ?1 OR to_id = ?1",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([item_id], |r| r.get::<_, i64>(0));
    match rows {
        Ok(rows) => {
            let out: Vec<i64> = rows.filter_map(std::result::Result::ok).collect();
            out
        }
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- versions

pub fn snapshot_item(conn: &Connection, item: &Item) -> Result<()> {
    conn.execute(
        "INSERT INTO item_version (item_id, saved_at, snapshot) VALUES (?1, ?2, ?3)",
        params![item.id, now_iso(), serde_json::to_string(item)?],
    )?;
    Ok(())
}

#[must_use]
pub fn versions_of(conn: &Connection, item_id: i64) -> Vec<Version> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, item_id, saved_at, snapshot FROM item_version \
         WHERE item_id = ?1 ORDER BY saved_at DESC, id DESC",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([item_id], |r| {
        Ok(Version {
            id: r.get(0)?,
            item_id: r.get(1)?,
            saved_at: r.get(2)?,
            snapshot: r.get(3)?,
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(std::result::Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- audit + undo

pub fn audit(
    conn: &Connection,
    item_id: Option<i64>,
    action: &str,
    detail: &serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO audit_log (at, item_id, action, detail) VALUES (?1, ?2, ?3, ?4)",
        params![now_iso(), item_id, action, detail.to_string()],
    )?;
    Ok(())
}

fn record_undo(conn: &Connection, kind: &str, payload: &serde_json::Value) -> Result<()> {
    conn.execute(
        "INSERT INTO undo_op (at, kind, payload) VALUES (?1, ?2, ?3)",
        params![now_iso(), kind, payload.to_string()],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoOutcome {
    pub kind: String,
    pub applied: bool,
}

/// Pop and apply the most recent undoable operation. Returns `applied: false`
/// for no-op entries (e.g. undoing a restore-of-trashed-item is a no-op).
pub fn undo_last(conn: &Connection) -> Result<Option<UndoOutcome>> {
    let row: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, kind, payload FROM undo_op ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((op_id, kind, payload)) = row else {
        return Ok(None);
    };
    conn.execute("DELETE FROM undo_op WHERE id = ?1", [op_id])?;
    let v: serde_json::Value = serde_json::from_str(&payload)?;

    let applied = match kind.as_str() {
        "create_item" => {
            let id = v["item_id"].as_i64().unwrap_or_default();
            conn.execute("DELETE FROM item WHERE id = ?1", [id])? > 0
        }
        "trash_item" => {
            let id = v["item_id"].as_i64().unwrap_or_default();
            conn.execute("UPDATE item SET deleted_at = NULL WHERE id = ?1", [id])? > 0
        }
        "update_item" => {
            let id = v["item_id"].as_i64().unwrap_or_default();
            let list_id: Option<i64> = serde_json::from_value(v["list_id"].clone())?;
            let private = i64::from(v["private"].as_bool().unwrap_or(false));
            let sort_key: Option<String> = serde_json::from_value(v["sort_key"].clone())?;
            conn.execute(
                "UPDATE item SET list_id = ?1, private = ?2, sort_key = ?3 WHERE id = ?4",
                params![list_id, private, sort_key, id],
            )? > 0
        }
        "link" => {
            let f = v["from_id"].as_i64().unwrap_or_default();
            let t = v["to_id"].as_i64().unwrap_or_default();
            conn.execute(
                "DELETE FROM item_link WHERE (from_id=?1 AND to_id=?2) OR (from_id=?2 AND to_id=?1)",
                params![f, t],
            )? > 0
        }
        "tag" => {
            let i = v["item_id"].as_i64().unwrap_or_default();
            let t = v["tag_id"].as_i64().unwrap_or_default();
            conn.execute(
                "DELETE FROM item_tag WHERE item_id=?1 AND tag_id=?2",
                params![i, t],
            )? > 0
        }
        _ => false,
    };
    Ok(Some(UndoOutcome { kind, applied }))
}

/// Convenience for tests and callers that want item counts per kind.
#[must_use]
pub fn counts_by_kind(conn: &Connection) -> HashMap<&'static str, i64> {
    let mut map = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT kind, count(*) FROM item GROUP BY kind") {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        {
            for row in rows.flatten() {
                if let Some(kind) = Kind::parse(&row.0) {
                    map.insert(kind.as_str(), row.1);
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::schema::apply_all(&mut conn).unwrap();
        conn
    }

    #[test]
    fn create_trash_restore_purge() {
        let conn = mem();
        let id = create_item(&conn, Kind::Note, None, false).unwrap();
        trash_item(&conn, id).unwrap();
        let q = TrashQuery {
            kind: None,
            deleted_after: None,
            deleted_before: None,
            limit: 10,
        };
        let t = query_trash(&conn, &q).unwrap();
        assert_eq!(t.len(), 1);
        restore_item(&conn, id).unwrap();
        assert!(query_trash(&conn, &q).unwrap().is_empty());
        purge_item(&conn, id).unwrap();
        assert!(matches!(get_item(&conn, id), Err(Error::NotFound(_))));
    }

    #[test]
    fn links_are_symmetric_and_undoable() {
        let conn = mem();
        let a = create_item(&conn, Kind::Task, None, false).unwrap();
        let b = create_item(&conn, Kind::Note, None, false).unwrap();
        link_items(&conn, a, b).unwrap();
        assert_eq!(related_items(&conn, a), vec![b]);
        assert_eq!(related_items(&conn, b), vec![a]);
        undo_last(&conn).unwrap();
        assert_eq!(related_items(&conn, a), [] as [i64; 0]);
    }

    #[test]
    fn undo_create_removes_item() {
        let conn = mem();
        let id = create_item(&conn, Kind::Contact, None, false).unwrap();
        assert_eq!(counts_by_kind(&conn)["contact"], 1);
        let out = undo_last(&conn).unwrap().unwrap();
        assert_eq!(out.kind, "create_item");
        assert!(out.applied);
        assert!(matches!(get_item(&conn, id), Err(Error::NotFound(_))));
    }

    #[test]
    fn versions_record_scalars() {
        let conn = mem();
        let list = create_list(&conn, "task", None, "Local Tasks", None, 0).unwrap();
        let id = create_item(&conn, Kind::Task, None, false).unwrap();
        update_item_scalars(&conn, id, Some(list), true, Some("k")).unwrap();
        assert_eq!(versions_of(&conn, id).len(), 1);
        let item = get_item(&conn, id).unwrap();
        assert!(item.private);
        assert_eq!(item.list_id, Some(list));
    }

    #[test]
    fn tag_roundtrip() {
        let conn = mem();
        let id = create_item(&conn, Kind::Note, None, false).unwrap();
        tag_item(&conn, id, "work").unwrap();
        tag_item(&conn, id, "work").unwrap(); // idempotent
        assert_eq!(tags_of_item(&conn, id).len(), 1);
        untag_item(&conn, id, "work").unwrap();
        assert!(tags_of_item(&conn, id).is_empty());
    }
}
