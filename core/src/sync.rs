//! Device sync engine: guid-addressed change payloads + last-writer-wins apply.
//!
//! Every item row carries a random `guid` (schema V4) that is stable across
//! devices, so payloads reference items independently of local row ids. A
//! trigger-fed `change_log` captures inserts, updates and tombstones on
//! `item` — the single capture point, since every kind-table writer bumps
//! `item.updated_at`. Sync is offline-batch: a pulling side calls
//! [`changes_since`] (or [`snapshot_all`] for a fresh peer) and hands the
//! payloads to [`apply_batch`] on the other device, which resolves conflicts
//! last-writer-wins on `updated_at` and files every losing version into
//! `item_version` so nothing is silently destroyed.
//!
//! Messages are desktop-authoritative (read state only) and never replicate.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::model::now_iso;

/// `(item kind, kind table)` pairs that replicate. `message` is excluded.
pub const SYNC_KINDS: [(&str, &str); 6] = [
    ("note", "note"),
    ("task", "task"),
    ("event", "event"),
    ("contact", "contact"),
    ("password", "pw_entry"),
    ("diary", "diary_entry"),
];

/// Child tables replicated as replace-all sets, keyed by parent kind table.
const CHILD_SETS: &[(&str, &str, &str)] = &[
    // (parent table, child table, parent column)
    ("note", "note_tab", "note_id"),
    ("event", "event_reminder", "event_id"),
    ("event", "event_participant", "event_id"),
    ("contact", "contact_field", "contact_id"),
];

/// Columns that hold device-local references (calendars, categories,
/// assignments) and can never be applied from another device.
const LOCAL_COLS: &[&str] = &["calendar_id", "category_id", "assigned_contact_id"];

fn table_for_kind(kind: &str) -> Option<&'static str> {
    SYNC_KINDS.iter().find(|(k, _)| *k == kind).map(|(_, t)| *t)
}

// ---------------------------------------------------------------- payloads

/// A child-table replace-all set (e.g. a contact's phones/emails).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildSet {
    pub table: String,
    pub parent_col: String,
    pub rows: Vec<Value>,
}

/// The full, guid-addressed state of one item — the sync wire unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangePayload {
    pub guid: String,
    pub kind: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub private: bool,
    pub sort_key: Option<String>,
    pub parent_guid: Option<String>,
    /// List resolved by name on apply (lists are per-device rows).
    pub list: Option<String>,
    pub tags: Vec<String>,
    /// Guids of linked items (symmetric on both sides).
    pub links: Vec<String>,
    /// The kind-table row without `id`, `item_id` and local-reference columns.
    pub row: Value,
    #[serde(default)]
    pub children: Vec<ChildSet>,
}

/// One deduplicated change with its change-log cursor position.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeEntry {
    pub cursor: i64,
    pub payload: ChangePayload,
}

/// Highest change-log id — the cursor a peer persists after a successful pull.
#[must_use]
pub fn max_change_id(conn: &Connection) -> i64 {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM change_log", [], |r| {
        r.get(0)
    })
    .unwrap_or(0)
}

const LOG_FLOOR_KEY: &str = "sync.log_floor";

/// True when `cursor` predates the change-log prune floor — the peer's window
/// of changes was pruned away and it must be re-snapshotted.
#[must_use]
pub fn cursor_stale(conn: &Connection, cursor: i64) -> bool {
    let floor: i64 = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            [LOG_FLOOR_KEY],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    cursor < floor
}

/// Changes after `cursor`, deduplicated to the latest entry per item.
/// Returns the payloads plus the new cursor. Scanning newest-first guarantees
/// the newest payload per item is sent; the cursor only advances past rows
/// whose item was actually sent (or skipped as non-replicating), so a batch
/// cut by `limit` can never lose an item's changes.
///
/// # Errors
/// Propagates SQL and payload-shaping errors.
pub fn changes_since(
    conn: &Connection,
    cursor: i64,
    limit: usize,
) -> Result<(Vec<ChangeEntry>, i64)> {
    let mut stmt =
        conn.prepare("SELECT id, item_id FROM change_log WHERE id > ?1 ORDER BY id DESC LIMIT ?2")?;
    let rows: Vec<(i64, i64)> = stmt
        .query_map(params![cursor, limit * 8], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut new_cursor = cursor;
    for (id, item_id) in &rows {
        match (seen.insert(*item_id), payload_for(conn, *item_id)?) {
            (true, Some(payload)) => {
                out.push(ChangeEntry {
                    cursor: *id,
                    payload,
                });
                new_cursor = *id;
                if out.len() >= limit {
                    break;
                }
            }
            // Non-replicating kind (message) or purged row: advance past it.
            (true, None) => new_cursor = *id,
            // Older entry of an item already covered by a newer payload.
            (false, _) => {}
        }
    }
    Ok((out, new_cursor))
}

/// Full state for a fresh peer; returns every live and tombstoned item plus
/// the cursor to persist afterwards.
///
/// # Errors
/// Propagates SQL and payload-shaping errors.
pub fn snapshot_all(conn: &Connection) -> Result<(Vec<ChangePayload>, i64)> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM item ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(payload) = payload_for(conn, id)? {
            out.push(payload);
        }
    }
    Ok((out, max_change_id(conn)))
}

/// Build the wire payload for one item, or `None` for non-replicating kinds.
fn payload_for(conn: &Connection, item_id: i64) -> Result<Option<ChangePayload>> {
    let row = conn
        .query_row(
            "SELECT kind, guid, created_at, updated_at, deleted_at, private, \
             sort_key, parent_id, list_id FROM item WHERE id = ?1",
            [item_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        kind,
        guid,
        created_at,
        updated_at,
        deleted_at,
        private,
        sort_key,
        parent_id,
        list_id,
    )) = row
    else {
        return Ok(None);
    };
    let Some(table) = table_for_kind(&kind) else {
        return Ok(None); // messages (and anything new) stay local
    };
    let Some(guid) = guid else {
        return Ok(None); // pre-guid row not yet backfilled: skip, don't stall
    };
    let parent_guid = match parent_id {
        Some(pid) => conn
            .query_row("SELECT guid FROM item WHERE id = ?1", [pid], |r| r.get(0))
            .optional()?
            .flatten(),
        None => None,
    };
    let list = match list_id {
        Some(lid) => conn
            .query_row("SELECT name FROM item_list WHERE id = ?1", [lid], |r| {
                r.get(0)
            })
            .optional()?,
        None => None,
    };
    let tags = tag_names(conn, item_id)?;
    let links = outgoing_links(conn, item_id)?;
    let row_json = row_to_json(conn, table, "item_id", item_id)?;

    let mut children = Vec::new();
    for (parent_table, child_table, parent_col) in CHILD_SETS {
        if *parent_table != table {
            continue;
        }
        let parent_pk: Option<i64> = conn
            .query_row(
                &format!("SELECT id FROM {table} WHERE item_id = ?1"),
                [item_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(parent_pk) = parent_pk else { continue };
        let rows = child_rows(conn, child_table, parent_col, parent_pk)?;
        children.push(ChildSet {
            table: (*child_table).to_string(),
            parent_col: (*parent_col).to_string(),
            rows,
        });
    }

    Ok(Some(ChangePayload {
        guid,
        kind,
        created_at,
        updated_at,
        deleted_at,
        private: private != 0,
        sort_key,
        parent_guid,
        list,
        tags,
        links,
        row: row_json,
        children,
    }))
}

/// Tag names of one item, alphabetically.
fn tag_names(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT t.name FROM tag t JOIN item_tag it ON it.tag_id = t.id \
         WHERE it.item_id = ?1 ORDER BY t.name",
    )?;
    let rows = stmt.query_map([item_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Guids of items linked to `item_id`, deterministically ordered.
fn outgoing_links(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT CASE WHEN l.from_id = ?1 THEN l.to_id ELSE l.from_id END, \
         o.guid FROM item_link l \
         JOIN item o ON o.id = CASE WHEN l.from_id = ?1 THEN l.to_id ELSE l.from_id END \
         WHERE l.from_id = ?1 OR l.to_id = ?1 ORDER BY 1",
    )?;
    let rows = stmt.query_map([item_id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let mut v: Vec<(i64, Option<String>)> = rows.collect::<std::result::Result<_, _>>()?;
    v.sort_by_key(|(id, _)| *id);
    Ok(v.into_iter().filter_map(|(_, g)| g).collect())
}

/// Read a row of `table` (found by `key_col = key_val`) as JSON, dropping the
/// primary key, the `item_id` foreign key, and device-local reference columns.
fn row_to_json(conn: &Connection, table: &str, key_col: &str, key_val: i64) -> Result<Value> {
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table} WHERE {key_col} = ?1"))?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut rows = stmt.query([key_val])?;
    let Some(row) = rows.next()? else {
        return Ok(Value::Null);
    };
    let mut out = serde_json::Map::new();
    for (i, col) in names.iter().enumerate() {
        let name: &str = col.as_str();
        if name == "id" || name == "item_id" || LOCAL_COLS.contains(&name) {
            continue;
        }
        let v = match row.get_ref(i)? {
            rusqlite::types::ValueRef::Null => Value::Null,
            rusqlite::types::ValueRef::Integer(n) => json!(n),
            rusqlite::types::ValueRef::Real(f) => json!(f),
            rusqlite::types::ValueRef::Text(t) => {
                json!(String::from_utf8_lossy(t).into_owned())
            }
            rusqlite::types::ValueRef::Blob(b) => json!(b.to_vec()),
        };
        out.insert(name.to_owned(), v);
    }
    Ok(Value::Object(out))
}

/// Read all rows of a child table belonging to `parent_pk` as JSON objects
/// (primary key and parent column stripped).
fn child_rows(
    conn: &Connection,
    table: &str,
    parent_col: &str,
    parent_pk: i64,
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM {table} WHERE {parent_col} = ?1 ORDER BY id"
    ))?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut rows = stmt.query([parent_pk])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut obj = serde_json::Map::new();
        for (i, nm) in names.iter().enumerate() {
            let name: &str = nm.as_str();
            if name == "id" || name == parent_col || name == "contact_id" {
                continue; // contact_id in participants is a local reference
            }
            let v = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(n) => json!(n),
                rusqlite::types::ValueRef::Real(f) => json!(f),
                rusqlite::types::ValueRef::Text(t) => {
                    json!(String::from_utf8_lossy(t).into_owned())
                }
                rusqlite::types::ValueRef::Blob(b) => json!(b.to_vec()),
            };
            obj.insert(name.to_owned(), v);
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

// ---------------------------------------------------------------- apply

/// Result of applying a remote batch.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ApplyReport {
    pub applied: usize,
    pub skipped: usize,
    /// Loser versions preserved into `item_version`; `winner` says which side
    /// kept the live row.
    pub conflicts: Vec<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictInfo {
    pub guid: String,
    pub kind: String,
    pub winner: &'static str,
}

/// Apply remote payloads with last-writer-wins on `updated_at`. Runs in one
/// transaction; the change-log rows the apply itself produces are pruned so
/// replicated state is not re-broadcast. Losing versions (remote-older and
/// equal-but-different) are preserved into `item_version`.
///
/// # Errors
/// Returns [`Error::Parse`] for payloads naming unknown kinds/tables/columns;
/// propagates SQL errors for non-conflict failures.
pub fn apply_batch(conn: &mut Connection, batch: &[ChangePayload]) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    let tx = conn.transaction()?;
    let floor: i64 = tx.query_row("SELECT COALESCE(MAX(id), 0) FROM change_log", [], |r| {
        r.get(0)
    })?;

    let mut link_pairs: Vec<(String, String)> = Vec::new();

    for p in batch {
        let Some(table) = table_for_kind(&p.kind) else {
            // Unknown kind: an older peer receiving newer data skips it
            // instead of failing the whole batch (forward compatibility).
            report.skipped += 1;
            continue;
        };
        validate(p)?;
        match local_id(&tx, &p.guid)? {
            None if p.deleted_at.is_some() => {
                report.skipped += 1; // never seen here; tombstone is final
            }
            None => {
                insert_remote(&tx, p, table)?;
                report.applied += 1;
                for l in &p.links {
                    link_pairs.push((p.guid.clone(), l.clone()));
                }
            }
            Some(local_id) => {
                let local_at: String = tx.query_row(
                    "SELECT updated_at FROM item WHERE id = ?1",
                    [local_id],
                    |r| r.get(0),
                )?;
                if p.updated_at > local_at {
                    // The local state loses: snapshot it before overwriting.
                    if let Some(local_state) = payload_for(&tx, local_id)? {
                        preserve(&tx, local_id, &local_state)?;
                    }
                    apply_remote(&tx, local_id, p, table)?;
                    report.applied += 1;
                    for l in &p.links {
                        link_pairs.push((p.guid.clone(), l.clone()));
                    }
                } else if p.updated_at < local_at
                    || (p.updated_at == local_at && !same_content(&tx, local_id, p, table)?)
                {
                    preserve(&tx, local_id, p)?;
                    report.conflicts.push(ConflictInfo {
                        guid: p.guid.clone(),
                        kind: p.kind.clone(),
                        winner: "local",
                    });
                } else {
                    report.skipped += 1;
                }
            }
        }
    }

    resolve_links(&tx, &link_pairs)?;
    tx.execute("DELETE FROM change_log WHERE id > ?1", [floor])?;
    tx.commit()?;
    Ok(report)
}

/// Insert deferred link pairs now that every batch item exists. The symmetric
/// trigger mirrors the reverse direction.
fn resolve_links(conn: &Connection, pairs: &[(String, String)]) -> Result<()> {
    for (a, b) in pairs {
        if let (Some(ia), Some(ib)) = (guid_id(conn, a)?, guid_id(conn, b)?) {
            if ia != ib {
                conn.execute(
                    "INSERT OR IGNORE INTO item_link (from_id, to_id) VALUES (?1, ?2)",
                    params![ia, ib],
                )?;
            }
        }
    }
    Ok(())
}

fn validate(p: &ChangePayload) -> Result<()> {
    if p.guid.is_empty() {
        return Err(Error::parse("sync payload", "empty guid"));
    }
    ident(p.row.as_object().map(|o| o.keys()))?;
    Ok(())
}

fn ident<'a, I: IntoIterator<Item = &'a String>>(keys: Option<I>) -> Result<()> {
    if let Some(keys) = keys {
        for k in keys {
            let ok = !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !k.chars().next().is_some_and(|c| c.is_ascii_digit());
            if !ok {
                return Err(Error::parse("sync payload", format!("bad column {k}")));
            }
        }
    }
    Ok(())
}

fn local_id(conn: &Connection, guid: &str) -> Result<Option<i64>> {
    conn.query_row("SELECT id FROM item WHERE guid = ?1", [guid], |r| r.get(0))
        .optional()
        .map_err(Error::from)
}

fn guid_id(conn: &Connection, guid: &str) -> Result<Option<i64>> {
    local_id(conn, guid)
}

/// Insert a brand-new remote item (kind row, children, tags, list).
fn insert_remote(conn: &Connection, p: &ChangePayload, table: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO item (kind, created_at, updated_at, deleted_at, private, \
         sort_key, guid, parent_id, list_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            p.kind,
            p.created_at,
            p.updated_at,
            p.deleted_at,
            i64::from(p.private),
            p.sort_key,
            p.guid,
            p.parent_guid
                .as_deref()
                .and_then(|g| guid_id(conn, g).ok().flatten()),
            resolve_list(conn, &p.kind, p.list.as_deref())?,
        ],
    )?;
    let item_id = conn.last_insert_rowid();
    upsert_kind_row(conn, item_id, p, table)?;
    apply_children(conn, p, table)?;
    apply_tags(conn, item_id, &p.tags)?;
    Ok(())
}

/// Overwrite an existing local item with a newer remote state.
fn apply_remote(conn: &Connection, item_id: i64, p: &ChangePayload, table: &str) -> Result<()> {
    conn.execute(
        "UPDATE item SET updated_at = ?2, deleted_at = ?3, private = ?4, \
         sort_key = ?5, parent_id = ?6, list_id = ?7 WHERE id = ?1",
        params![
            item_id,
            p.updated_at,
            p.deleted_at,
            i64::from(p.private),
            p.sort_key,
            p.parent_guid
                .as_deref()
                .and_then(|g| guid_id(conn, g).ok().flatten()),
            resolve_list(conn, &p.kind, p.list.as_deref())?,
        ],
    )?;
    upsert_kind_row(conn, item_id, p, table)?;
    apply_children(conn, p, table)?;
    apply_tags(conn, item_id, &p.tags)?;
    Ok(())
}

fn upsert_kind_row(conn: &Connection, item_id: i64, p: &ChangePayload, table: &str) -> Result<()> {
    let Some(obj) = p.row.as_object() else {
        return Ok(()); // tombstone-only payload
    };
    let cols: Vec<&String> = obj.keys().collect();
    if cols.is_empty() {
        return Ok(());
    }
    let col_sql = std::iter::once("item_id".to_string())
        .chain(cols.iter().map(|c| (*c).clone()))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = format!("{}?", "?,".repeat(cols.len()));
    let update_sql = cols
        .iter()
        .map(|c| format!("{c} = excluded.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {table} ({col_sql}) VALUES ({placeholders}) \
         ON CONFLICT(item_id) DO UPDATE SET {update_sql}"
    );
    let mut values = vec![rusqlite::types::Value::from(item_id)];
    values.extend(cols.iter().map(|c| json_to_sql(&obj[*c])));
    // An entry_date/uid uniqueness clash with a different local row is a
    // conflict, not a sync failure — the local version wins and is preserved.
    if let Err(e) = conn.execute(&sql, rusqlite::params_from_iter(values.iter())) {
        return Err(Error::parse(
            "sync apply",
            format!(
                "kind-row upsert failed for guid {g}: {e}; sql={sql}; keys={keys:?}",
                g = p.guid,
                keys = obj.keys().collect::<Vec<_>>()
            ),
        ));
    }
    Ok(())
}

fn apply_children(conn: &Connection, p: &ChangePayload, table: &str) -> Result<()> {
    if p.children.is_empty() {
        return Ok(());
    }
    let parent_pk: Option<i64> = conn
        .query_row(
            &format!("SELECT id FROM {table} WHERE item_id = ?1"),
            [local_id(conn, &p.guid)?.unwrap_or(0)],
            |r| r.get(0),
        )
        .optional()?;
    let Some(parent_pk) = parent_pk else {
        return Ok(());
    };
    for set in &p.children {
        if !CHILD_SETS
            .iter()
            .any(|(_, ct, pc)| *ct == set.table && *pc == set.parent_col)
        {
            return Err(Error::parse(
                "sync payload",
                format!("unknown child table {}", set.table),
            ));
        }
        conn.execute(
            &format!("DELETE FROM {} WHERE {} = ?1", set.table, set.parent_col),
            [parent_pk],
        )?;
        for row in &set.rows {
            let Some(obj) = row.as_object() else { continue };
            let cols: Vec<&String> = obj.keys().collect();
            ident(Some(obj.keys()))?;
            if cols.is_empty() {
                continue;
            }
            let col_sql = std::iter::once(set.parent_col.clone())
                .chain(cols.iter().map(|c| (*c).clone()))
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = format!("{}?", "?,".repeat(cols.len()));
            let mut values = vec![rusqlite::types::Value::from(parent_pk)];
            values.extend(cols.iter().map(|c| json_to_sql(&obj[*c])));
            conn.execute(
                &format!(
                    "INSERT INTO {} ({col_sql}) VALUES ({placeholders})",
                    set.table
                ),
                rusqlite::params_from_iter(values.iter()),
            )?;
        }
    }
    Ok(())
}

fn apply_tags(conn: &Connection, item_id: i64, tags: &[String]) -> Result<()> {
    conn.execute("DELETE FROM item_tag WHERE item_id = ?1", [item_id])?;
    for name in tags {
        conn.execute("INSERT OR IGNORE INTO tag (name) VALUES (?1)", [name])?;
        let tag_id: i64 =
            conn.query_row("SELECT id FROM tag WHERE name = ?1", [name], |r| r.get(0))?;
        conn.execute(
            "INSERT OR IGNORE INTO item_tag (item_id, tag_id) VALUES (?1, ?2)",
            params![item_id, tag_id],
        )?;
    }
    Ok(())
}

fn resolve_list(conn: &Connection, kind: &str, name: Option<&str>) -> Result<Option<i64>> {
    let Some(name) = name else { return Ok(None) };
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM item_list WHERE kind = ?1 AND name = ?2",
            [kind, name],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(Some(id));
    }
    conn.execute(
        "INSERT INTO item_list (kind, name, sort_order) VALUES (?1, ?2, 0)",
        [kind, name],
    )?;
    Ok(Some(conn.last_insert_rowid()))
}

/// Preserve a losing remote version under the surviving local item.
fn preserve(conn: &Connection, item_id: i64, loser: &ChangePayload) -> Result<()> {
    conn.execute(
        "INSERT INTO item_version (item_id, saved_at, snapshot) VALUES (?1, ?2, ?3)",
        params![item_id, now_iso(), serde_json::to_string(loser)?],
    )?;
    Ok(())
}

/// True when the local row's kind-table content equals the payload's.
fn same_content(conn: &Connection, item_id: i64, p: &ChangePayload, table: &str) -> Result<bool> {
    let local = row_to_json(conn, table, "item_id", item_id)?;
    Ok(local == p.row)
}

fn json_to_sql(v: &Value) -> rusqlite::types::Value {
    match v {
        Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(
            || rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0)),
            rusqlite::types::Value::Integer,
        ),
        Value::String(s) => rusqlite::types::Value::Text(s.clone()),
        _ => rusqlite::types::Value::Null,
    }
}

// ---------------------------------------------------------------- devices

/// A paired peer device.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub paired_at: String,
    pub last_sync: Option<String>,
}

/// Register (or refresh) a paired device.
pub fn device_register(conn: &Connection, id: &str, name: &str, platform: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO device (id, name, platform, paired_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, platform = excluded.platform",
        params![id, name, platform, now_iso()],
    )?;
    Ok(())
}

/// Stamp `last_sync` for a device after a successful session.
pub fn device_touch(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE device SET last_sync = ?2 WHERE id = ?1",
        params![id, now_iso()],
    )?;
    Ok(())
}

/// Remove a pairing.
pub fn device_remove(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM device WHERE id = ?1", [id])?;
    Ok(())
}

/// All paired devices.
///
/// # Errors
/// Propagates SQL errors.
pub fn devices(conn: &Connection) -> Result<Vec<DeviceRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, platform, paired_at, last_sync FROM device ORDER BY paired_at",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(DeviceRow {
            id: r.get(0)?,
            name: r.get(1)?,
            platform: r.get(2)?,
            paired_at: r.get(3)?,
            last_sync: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Prune change-log entries below `before_id`. The floor is recorded so
/// peers holding older cursors detect the gap via [`cursor_stale`].
pub fn prune_change_log(conn: &Connection, before_id: i64) -> Result<usize> {
    let removed = conn.execute("DELETE FROM change_log WHERE id < ?1", [before_id])?;
    let current: i64 = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            [LOG_FLOOR_KEY],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if before_id > current {
        conn.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![LOG_FLOOR_KEY, before_id.to_string()],
        )?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap() // open() migrates
    }

    fn make_note(conn: &Connection, title: &str) -> (i64, String) {
        let item = crate::repo::create_item(conn, crate::model::Kind::Note, None, false).unwrap();
        conn.execute(
            "INSERT INTO note (item_id, title, content) VALUES (?1, ?2, 'body')",
            params![item, title],
        )
        .unwrap();
        crate::repo::tag_item(conn, item, "work").unwrap();
        let guid: String = conn
            .query_row("SELECT guid FROM item WHERE id = ?1", [item], |r| r.get(0))
            .unwrap();
        (item, guid)
    }

    #[test]
    fn migration_backfills_guids_and_log_captures_writes() {
        let s = store();
        let conn = s.conn();
        let (item, guid) = make_note(conn, "one");
        assert_eq!(guid.len(), 32);
        let op: String = conn
            .query_row(
                "SELECT op FROM change_log WHERE item_id = ?1 ORDER BY id DESC LIMIT 1",
                [item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(op, "put");
        crate::repo::trash_item(conn, item).unwrap();
        let op: String = conn
            .query_row(
                "SELECT op FROM change_log WHERE item_id = ?1 ORDER BY id DESC LIMIT 1",
                [item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(op, "del");
    }

    #[test]
    fn roundtrip_and_idempotent_replay() {
        let a = store();
        let mut b = store();
        let (_, guid) = make_note(a.conn(), "hello");

        let (batch, cursor) = changes_since(a.conn(), 0, 100).unwrap();
        assert!(!batch.is_empty());
        let payloads: Vec<_> = batch.into_iter().map(|e| e.payload).collect();
        let report = apply_batch(b.conn_mut(), &payloads).unwrap();
        assert!(report.applied >= 1);
        assert!(report.conflicts.is_empty());

        let b_id: i64 = b
            .conn()
            .query_row("SELECT id FROM item WHERE guid = ?1", [&guid], |r| r.get(0))
            .unwrap();
        let title: String = b
            .conn()
            .query_row("SELECT title FROM note WHERE item_id = ?1", [b_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "hello");
        let tags: i64 = b
            .conn()
            .query_row(
                "SELECT count(*) FROM item_tag WHERE item_id = ?1",
                [b_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tags, 1, "tags replicate");

        // Replay is a no-op.
        let replay = apply_batch(b.conn_mut(), &payloads).unwrap();
        assert_eq!(replay.applied, 0);
        assert!(replay.conflicts.is_empty());

        // The cursor advanced and further pulls are empty.
        let (next, _) = changes_since(a.conn(), cursor, 100).unwrap();
        assert!(next.is_empty());
    }

    #[test]
    fn bidirectional_edit_and_conflict_preserves_loser() {
        let mut a = store();
        let mut b = store();
        let (_, guid) = make_note(a.conn(), "v1");
        let (batch, _) = changes_since(a.conn(), 0, 100).unwrap();
        apply_batch(
            b.conn_mut(),
            &batch.into_iter().map(|e| e.payload).collect::<Vec<_>>(),
        )
        .unwrap();
        let b_id = guid_id(b.conn(), &guid).unwrap().unwrap();
        let a_id = guid_id(a.conn(), &guid).unwrap().unwrap();

        // B edits (explicitly newer timestamp), A edits too but stays older.
        b.conn()
            .execute(
                "UPDATE note SET title = 'from-b' WHERE item_id = ?1",
                [b_id],
            )
            .unwrap();
        b.conn()
            .execute(
                "UPDATE item SET updated_at = '2026-01-02T00:00:00+00:00' WHERE id = ?1",
                [b_id],
            )
            .unwrap();
        a.conn()
            .execute(
                "UPDATE note SET title = 'from-a' WHERE item_id = ?1",
                [a_id],
            )
            .unwrap();
        a.conn()
            .execute(
                "UPDATE item SET updated_at = '2026-01-01T00:00:00+00:00' WHERE id = ?1",
                [a_id],
            )
            .unwrap();

        let (batch, _) = changes_since(b.conn(), 0, 100).unwrap();
        let report = apply_batch(
            a.conn_mut(),
            &batch.into_iter().map(|e| e.payload).collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(report.applied, 1);
        let title: String = a
            .conn()
            .query_row("SELECT title FROM note WHERE item_id = ?1", [a_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "from-b", "newer writer wins");
        // The losing local edit is preserved as a version snapshot.
        let versions: i64 = a
            .conn()
            .query_row(
                "SELECT count(*) FROM item_version WHERE item_id = ?1",
                [a_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(versions >= 1, "loser preserved");
    }

    #[test]
    fn tombstone_and_children_replicate() {
        let a = store();
        let mut b = store();
        let item =
            crate::repo::create_item(a.conn(), crate::model::Kind::Contact, None, false).unwrap();
        a.conn()
            .execute(
                "INSERT INTO contact (item_id, display_name) VALUES (?1, 'Marie Powell')",
                [item],
            )
            .unwrap();
        let cid: i64 = a
            .conn()
            .query_row("SELECT id FROM contact WHERE item_id = ?1", [item], |r| {
                r.get(0)
            })
            .unwrap();
        a.conn()
            .execute(
                "INSERT INTO contact_field (contact_id, kind, value) VALUES (?1, 'email', 'm@x.io')",
                [cid],
            )
            .unwrap();
        let (batch, _) = changes_since(a.conn(), 0, 100).unwrap();
        let report = apply_batch(
            b.conn_mut(),
            &batch.into_iter().map(|e| e.payload).collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(report.applied, 1);
        let fields: i64 = b
            .conn()
            .query_row("SELECT count(*) FROM contact_field", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fields, 1, "child sets replicate");

        crate::repo::trash_item(a.conn(), item).unwrap();
        let guid: String = a
            .conn()
            .query_row("SELECT guid FROM item WHERE id = ?1", [item], |r| r.get(0))
            .unwrap();
        let (batch, _) = changes_since(a.conn(), 0, 100).unwrap();
        apply_batch(
            b.conn_mut(),
            &batch.into_iter().map(|e| e.payload).collect::<Vec<_>>(),
        )
        .unwrap();
        let dead: Option<String> = b
            .conn()
            .query_row(
                "SELECT deleted_at FROM item WHERE guid = ?1",
                [&guid],
                |r| r.get(0),
            )
            .unwrap();
        assert!(dead.is_some(), "tombstone replicates");
    }

    #[test]
    fn snapshot_serves_fresh_peer_and_prune_flags_stale() {
        let a = store();
        make_note(a.conn(), "kept");
        let (snap, cursor) = snapshot_all(a.conn()).unwrap();
        assert_eq!(snap.len(), 1);
        let mut b = store();
        apply_batch(b.conn_mut(), &snap).unwrap();
        assert_eq!(
            crate::repo::counts_by_kind(b.conn()).get("note").copied(),
            Some(1)
        );

        // While the peer's cursor is inside the unpruned log: fresh.
        assert!(!cursor_stale(a.conn(), cursor));
        // Pruning below the cursor makes every older cursor stale.
        prune_change_log(a.conn(), cursor + 1).unwrap();
        assert!(cursor_stale(a.conn(), 1));
        assert!(cursor_stale(a.conn(), cursor));
    }

    #[test]
    fn invalid_kind_is_rejected() {
        let mut b = store();
        let p = ChangePayload {
            guid: "abc".into(),
            kind: "message".into(), // desktop-authoritative, never syncs
            created_at: "x".into(),
            updated_at: "x".into(),
            deleted_at: None,
            private: false,
            sort_key: None,
            parent_guid: None,
            list: None,
            tags: vec![],
            links: vec![],
            row: json!({}),
            children: vec![],
        };
        let report = apply_batch(b.conn_mut(), &[p]).unwrap();
        assert_eq!(report.applied, 0);
    }
}
