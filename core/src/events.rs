//! Calendar repository: events as parsed by the ICS engine ↔ `event` table.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::ical::EventRecord;
use crate::model::Kind;
use crate::repo;

pub fn ensure_calendar(conn: &Connection, name: &str, color: Option<&str>) -> Result<i64> {
    conn.execute(
        "INSERT INTO calendar (name, color) \
         SELECT ?1, ?2 WHERE NOT EXISTS (SELECT 1 FROM calendar WHERE name = ?1)",
        params![name, color],
    )?;
    let id: i64 = conn.query_row("SELECT id FROM calendar WHERE name = ?1", [name], |r| {
        r.get(0)
    })?;
    Ok(id)
}

/// Insert or update by ICS UID (sync-safe idempotency). Creates the anchor
/// `item` and snapshots the previous state on update.
pub fn upsert_event(conn: &Connection, calendar_id: i64, ev: &EventRecord) -> Result<i64> {
    let existing: Option<i64> = match &ev.uid {
        Some(uid) => conn
            .query_row("SELECT id FROM event WHERE uid = ?1", [uid], |r| r.get(0))
            .optional()?,
        None => None,
    };
    let Some(event_id) = existing else {
        let item_id = repo::create_item(conn, Kind::Event, None, ev.private)?;
        conn.execute(
            "INSERT INTO event (item_id, calendar_id, title, start_utc, end_utc, all_day, \
             location, description, rrule, uid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                item_id,
                calendar_id,
                ev.title,
                ev.start,
                ev.end,
                i64::from(ev.all_day),
                ev.location,
                ev.description,
                ev.rrule,
                ev.uid
            ],
        )?;
        return Ok(conn.last_insert_rowid());
    };
    let unchanged: i64 = conn.query_row(
        "SELECT count(*) FROM event WHERE id = ?1 AND title = ?2 AND start_utc = ?3 \
         AND end_utc IS ?4 AND all_day = ?5 AND location IS ?6 AND description IS ?7 \
         AND rrule IS ?8",
        params![
            event_id,
            ev.title,
            ev.start,
            ev.end,
            i64::from(ev.all_day),
            ev.location,
            ev.description,
            ev.rrule
        ],
        |r| r.get(0),
    )?;
    if unchanged > 0 {
        return Ok(event_id);
    }
    let item_id: i64 =
        conn.query_row("SELECT item_id FROM event WHERE id = ?1", [event_id], |r| {
            r.get(0)
        })?;
    let item = repo::get_item(conn, item_id)?;
    repo::snapshot_item(conn, &item)?;
    conn.execute(
        "UPDATE event SET title=?1, start_utc=?2, end_utc=?3, all_day=?4, location=?5, \
         description=?6, rrule=?7, calendar_id=?8 WHERE id=?9",
        params![
            ev.title,
            ev.start,
            ev.end,
            i64::from(ev.all_day),
            ev.location,
            ev.description,
            ev.rrule,
            calendar_id,
            event_id
        ],
    )?;
    let _ = repo::touch_item(conn, item_id);
    Ok(event_id)
}

/// All events of a calendar as records (for export / round-trip).
#[must_use]
pub fn events_of_calendar(conn: &Connection, calendar_id: i64) -> Vec<EventRecord> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT uid, title, start_utc, end_utc, all_day, location, description, rrule \
         FROM event WHERE calendar_id = ?1 ORDER BY start_utc",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([calendar_id], |r| {
        Ok(EventRecord {
            uid: r.get(0)?,
            title: r.get(1)?,
            start: r.get(2)?,
            end: r.get(3)?,
            all_day: r.get::<_, i64>(4)? != 0,
            location: r.get(5)?,
            description: r.get(6)?,
            rrule: r.get(7)?,
            categories: Vec::new(),
            private: false,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(std::result::Result::ok).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn upsert_is_idempotent_by_uid_and_updates_snapshot() {
        let mut store = Store::open_memory("x").unwrap();
        let conn = store.conn_mut();
        let cal = ensure_calendar(conn, "Work", Some("#1a73e8")).unwrap();
        let ev = EventRecord {
            uid: Some("u1".into()),
            title: "Briefing".into(),
            start: "2026-04-01T09:30:00Z".into(),
            end: Some("2026-04-01T10:30:00Z".into()),
            rrule: Some("FREQ=WEEKLY;BYDAY=WE".into()),
            ..Default::default()
        };
        let id1 = upsert_event(conn, cal, &ev).unwrap();
        let id2 = upsert_event(conn, cal, &ev).unwrap();
        assert_eq!(id1, id2);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let updated = EventRecord {
            title: "Team Briefing".into(),
            ..ev
        };
        upsert_event(conn, cal, &updated).unwrap();
        let (title, n_versions): (String, i64) = conn
            .query_row(
                "SELECT e.title, (SELECT count(*) FROM item_version v WHERE v.item_id = e.item_id) \
                 FROM event e WHERE e.id = ?1",
                [id1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "Team Briefing");
        assert_eq!(n_versions, 1, "previous state snapshotted on update");
        assert_eq!(events_of_calendar(conn, cal).len(), 1);
    }
}
