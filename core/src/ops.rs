//! Operations: reminder queue (T1.12) and mail rules (T1.11).
//!
//! Reminder semantics (S30): due reminders fire once; a missed fire (app was
//! closed) fires on next start flagged as `missed`. Snooze re-arms by moving
//! `snoozed_to`; the effective due moment is `snoozed_to ?? due_at`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::Result;
use crate::model::now_iso;

/// A reminder that is due (or overdue) and has not fired yet.
#[derive(Debug, Clone, Serialize)]
pub struct DueReminder {
    pub id: i64,
    pub item_id: i64,
    pub due_at: String,
    /// True when `due_at` is in the past beyond the current poll horizon —
    /// i.e. the app was not running when it came due.
    pub missed: bool,
}

/// Collect due reminders and mark them fired. `missed_horizon_secs`: a due
/// moment older than this many seconds counts as a missed fire (recovered on
/// start, S21/S30 semantics). Fires exactly once — `fired_at` is set, so the
/// same reminder is never returned twice.
pub fn take_due_reminders(
    conn: &Connection,
    now: &str,
    missed_horizon_secs: i64,
    limit: i64,
) -> Result<Vec<DueReminder>> {
    let cutoff = seconds_before(now, missed_horizon_secs);
    let mut stmt = conn.prepare(
        "SELECT id, item_id, due_at, snoozed_to FROM reminder_queue \
         WHERE fired_at IS NULL \
         AND COALESCE(snoozed_to, due_at) <= ?1 \
         ORDER BY COALESCE(snoozed_to, due_at) LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![now, limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, item_id, due_at, snoozed_to) = row?;
        // A snoozed reminder was already seen by the user, so it is never a
        // "missed" fire even when the snooze target is old.
        let missed = match &snoozed_to {
            Some(_) => false,
            None => is_before(&due_at, &cutoff),
        };
        mark_fired(conn, id)?;
        out.push(DueReminder {
            id,
            item_id,
            due_at,
            missed,
        });
    }
    Ok(out)
}

/// Re-arm a fired reminder by snoozing it to `to_utc` (clears `fired_at`).
pub fn snooze_reminder(conn: &Connection, id: i64, to_utc: &str) -> Result<()> {
    conn.execute(
        "UPDATE reminder_queue SET snoozed_to = ?2, fired_at = NULL WHERE id = ?1",
        params![id, to_utc],
    )?;
    Ok(())
}

/// Schedule a reminder for an item. Invalid `due_at` values are rejected by
/// the caller's validation; this stores as-is (ISO 8601).
pub fn schedule_reminder(
    conn: &Connection,
    item_id: i64,
    due_at: &str,
    method: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO reminder_queue (item_id, due_at, method) VALUES (?1, ?2, ?3)",
        params![item_id, due_at, method],
    )?;
    Ok(conn.last_insert_rowid())
}

fn mark_fired(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE reminder_queue SET fired_at = ?2 WHERE id = ?1",
        params![id, now_iso()],
    )?;
    Ok(())
}

// ------------------------------------------------------------ time helpers
// String comparison on ISO-8601 UTC timestamps (the storage format); both
// sides are normalized to `YYYY-MM-DDTHH:MM:SS+00:00`-style fixed-width text
// by `now_iso`/callers, so lexicographic order == chronological order.

fn seconds_before(now: &str, secs: i64) -> String {
    let parsed =
        chrono::DateTime::parse_from_rfc3339(now).unwrap_or_else(|_| chrono::Utc::now().into());
    let shifted = parsed - chrono::Duration::seconds(secs);
    shifted.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn is_before(a: &str, b: &str) -> bool {
    a < b
}

/// Count of not-yet-fired reminders that are due — status-bar bell (S5).
pub fn pending_due_count(conn: &Connection, now: &str) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT count(*) FROM reminder_queue \
             WHERE fired_at IS NULL AND COALESCE(snoozed_to, due_at) <= ?1",
            params![now],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    #[test]
    fn fires_once_and_flags_missed() {
        let s = store();
        let conn = s.conn();
        let item = crate::repo::create_item(conn, crate::model::Kind::Task, None, false).unwrap();
        // Due 2 hours before "now" — beyond a 60 s missed horizon.
        schedule_reminder(conn, item, "2026-01-01T08:00:00+00:00", "popup").unwrap();
        let now = "2026-01-01T10:00:00+00:00";

        let due = take_due_reminders(conn, now, 60, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert!(due[0].missed, "old due moment = missed fire");

        let again = take_due_reminders(conn, now, 60, 10).unwrap();
        assert!(again.is_empty(), "fires exactly once");

        let pending = pending_due_count(conn, now).unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn recent_due_is_not_missed() {
        let s = store();
        let conn = s.conn();
        let item = crate::repo::create_item(conn, crate::model::Kind::Task, None, false).unwrap();
        schedule_reminder(conn, item, "2026-01-01T09:59:30+00:00", "popup").unwrap();
        let due = take_due_reminders(conn, "2026-01-01T10:00:00+00:00", 60, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert!(!due[0].missed);
    }

    #[test]
    fn future_and_snoozed_are_not_due() {
        let s = store();
        let conn = s.conn();
        let item = crate::repo::create_item(conn, crate::model::Kind::Task, None, false).unwrap();
        let id = schedule_reminder(conn, item, "2026-01-01T12:00:00+00:00", "popup").unwrap();
        assert!(
            take_due_reminders(conn, "2026-01-01T10:00:00+00:00", 60, 10)
                .unwrap()
                .is_empty()
        );

        snooze_reminder(conn, id, "2026-01-01T13:00:00+00:00").unwrap();
        assert!(
            take_due_reminders(conn, "2026-01-01T12:30:00+00:00", 60, 10)
                .unwrap()
                .is_empty()
        );
        let due = take_due_reminders(conn, "2026-01-01T13:00:00+00:00", 60, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert!(!due[0].missed, "snoozed reminders are never missed");
    }

    #[test]
    fn limit_and_ordering() {
        let s = store();
        let conn = s.conn();
        let item = crate::repo::create_item(conn, crate::model::Kind::Task, None, false).unwrap();
        for h in [3i64, 1, 2] {
            schedule_reminder(conn, item, &format!("2026-01-01T0{h}:00:00+00:00"), "popup")
                .unwrap();
        }
        let first = take_due_reminders(conn, "2026-01-01T10:00:00+00:00", 60_000, 2).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first[0].due_at < first[1].due_at, "earliest first");
        assert_eq!(
            take_due_reminders(conn, "2026-01-01T10:00:00+00:00", 60_000, 10)
                .unwrap()
                .len(),
            1
        );
    }
}
