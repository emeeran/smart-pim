//! Notes repository (T3.28–T3.31 backend): list pages, CRUD with version
//! snapshots on save, favorites, note tabs. The editor contract is HTML
//! (Tiptap output); markdown mode converts at the UI layer.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::Kind;
use crate::repo;

#[derive(Debug, Clone, Serialize)]
pub struct NoteRow {
    pub id: i64,
    pub item_id: i64,
    pub title: String,
    /// Short plain-text extract for list cards.
    pub snippet: String,
    pub favorite: bool,
    pub updated_at: String,
}

/// Newest-updated first, keyset by (`updated_at`, id).
pub fn notes_page(
    conn: &Connection,
    after: Option<(&str, i64)>,
    limit: i64,
) -> Result<Vec<NoteRow>> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.item_id, n.title, n.content, n.favorite, i.updated_at \
         FROM note n JOIN item i ON i.id = n.item_id \
         WHERE i.deleted_at IS NULL \
         AND (?1 IS NULL OR i.updated_at < ?1 OR (i.updated_at = ?1 AND n.id < ?2)) \
         ORDER BY i.updated_at DESC, n.id DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            after.map(|(d, _)| d.to_string()),
            after.map_or(0, |(_, i)| i),
            limit
        ],
        |r| {
            let content: String = r.get(3)?;
            let snippet: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
            Ok(NoteRow {
                id: r.get(0)?,
                item_id: r.get(1)?,
                title: r.get(2)?,
                snippet: snippet.chars().take(120).collect(),
                favorite: r.get::<_, i64>(4)? != 0,
                updated_at: r.get(5)?,
            })
        },
    )?;
    let out = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteDetail {
    pub id: i64,
    pub item_id: i64,
    pub title: String,
    /// HTML (Tiptap output / trusted self-authored content).
    pub content: String,
    pub favorite: bool,
    pub reminder_at: Option<String>,
}

pub fn note_get(conn: &Connection, id: i64) -> Result<NoteDetail> {
    conn.query_row(
        "SELECT n.id, n.item_id, n.title, n.content, n.favorite, n.reminder_at \
         FROM note n WHERE n.id = ?1",
        [id],
        |r| {
            Ok(NoteDetail {
                id: r.get(0)?,
                item_id: r.get(1)?,
                title: r.get(2)?,
                content: r.get(3)?,
                favorite: r.get::<_, i64>(4)? != 0,
                reminder_at: r.get(5)?,
            })
        },
    )
    .optional()?
    .ok_or(Error::NotFound(id))
}

/// Create an empty note (anchor `item` included). Returns the note id.
pub fn note_create(conn: &Connection, title: &str, content: &str) -> Result<i64> {
    let item_id = repo::create_item(conn, Kind::Note, None, false)?;
    conn.execute(
        "INSERT INTO note (item_id, title, content) VALUES (?1, ?2, ?3)",
        params![item_id, title, content],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Save content: snapshots the previous state (version history, S10) when
/// something actually changed.
pub fn note_save(conn: &Connection, id: i64, title: &str, content: &str) -> Result<()> {
    let (item_id, old_title, old_content): (i64, String, String) = conn
        .query_row(
            "SELECT item_id, title, content FROM note WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(Error::NotFound(id))?;
    if old_title == title && old_content == content {
        return Ok(());
    }
    let item = repo::get_item(conn, item_id)?;
    repo::snapshot_item(conn, &item)?;
    conn.execute(
        "UPDATE note SET title = ?2, content = ?3 WHERE id = ?1",
        params![id, title, content],
    )?;
    let _ = repo::touch_item(conn, item_id);
    repo::audit(
        conn,
        Some(item_id),
        "note_save",
        &serde_json::json!({ "note_id": id }),
    )?;
    Ok(())
}

pub fn note_set_favorite(conn: &Connection, id: i64, favorite: bool) -> Result<()> {
    conn.execute(
        "UPDATE note SET favorite = ?2 WHERE id = ?1",
        params![id, i64::from(favorite)],
    )?;
    Ok(())
}

pub fn note_delete(conn: &Connection, id: i64) -> Result<()> {
    let item_id: i64 = conn
        .query_row("SELECT item_id FROM note WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(Error::NotFound(id))?;
    repo::trash_item(conn, item_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    #[test]
    fn create_save_snapshot_and_pages() {
        let s = store();
        let conn = s.conn();
        let id = note_create(conn, "Recipes", "<p>sourdough</p>").unwrap();
        note_create(conn, "Journal", "<p>day one</p>").unwrap();

        // Save with change snapshots + updates ordering (sleep crosses the
        // millisecond resolution of the timestamps).
        std::thread::sleep(std::time::Duration::from_millis(5));
        note_save(conn, id, "Recipes", "<p>sourdough starter</p>").unwrap();
        let (versions, title): (i64, String) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM item_version v JOIN note n ON n.item_id = v.item_id \
                 WHERE n.id = ?1), (SELECT title FROM note WHERE id = ?1)",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(versions, 1);
        assert_eq!(title, "Recipes");

        // No-op save creates no snapshot.
        note_save(conn, id, "Recipes", "<p>sourdough starter</p>").unwrap();
        let versions: i64 = conn
            .query_row(
                "SELECT count(*) FROM item_version v JOIN note n ON n.item_id = v.item_id WHERE n.id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(versions, 1);

        let page = notes_page(conn, None, 10).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].title, "Recipes", "most recently updated first");
        assert!(page[0].snippet.contains("sourdough"));
        assert_eq!(
            note_get(conn, id).unwrap().content,
            "<p>sourdough starter</p>"
        );
        assert!(note_get(conn, 999).is_err());
    }

    #[test]
    fn favorite_and_trash_lifecycle() {
        let s = store();
        let conn = s.conn();
        let id = note_create(conn, "T", "c").unwrap();
        note_set_favorite(conn, id, true).unwrap();
        assert!(note_get(conn, id).unwrap().favorite);

        note_delete(conn, id).unwrap();
        assert!(
            notes_page(conn, None, 10).unwrap().is_empty(),
            "trashed notes leave the list"
        );
        let trashed = repo::query_trash(
            conn,
            &repo::TrashQuery {
                kind: Some(Kind::Note),
                deleted_after: None,
                deleted_before: None,
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(trashed.len(), 1);
    }
}
