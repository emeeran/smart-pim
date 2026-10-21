//! Calendar repository (T3.13–T3.20 backend): event CRUD for the editor,
//! window queries with RRULE expansion (computed, never stored), and
//! reminder scheduling on events.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::ical;
use crate::repo;

/// One occurrence in a rendered window: expanded from recurring events.
#[derive(Debug, Clone, Serialize)]
pub struct Occurrence {
    pub event_id: i64,
    pub item_id: i64,
    pub title: String,
    /// RFC 3339 UTC (or `YYYY-MM-DD` for all-day).
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub calendar_id: i64,
    pub calendar_color: Option<String>,
}

fn parse_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    let nd = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    nd.and_hms_opt(0, 0, 0).map(|t| t.and_utc())
}

/// Events overlapping `[from, to]`, recurring rules expanded (cap 730,
/// matching the data-model invariant). All-day occurrences keep their
/// `YYYY-MM-DD` strings; timed ones are RFC 3339 UTC.
pub fn events_window(
    conn: &Connection,
    calendar_id: Option<i64>,
    from: &str,
    to: &str,
) -> Result<Vec<Occurrence>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.item_id, e.title, e.start_utc, e.end_utc, e.all_day, e.location, \
         e.rrule, c.color \
         FROM event e JOIN calendar c ON c.id = e.calendar_id \
         JOIN item i ON i.id = e.item_id AND i.deleted_at IS NULL \
         WHERE (?1 IS NULL OR e.calendar_id = ?1) \
         AND (e.rrule IS NOT NULL \
              OR (e.end_utc IS NOT NULL AND e.end_utc >= ?2 AND e.start_utc <= ?3)) \
         ORDER BY e.start_utc",
    )?;
    let rows = stmt.query_map(params![calendar_id, from, to], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, i64>(5)? != 0,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<String>>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (event_id, item_id, title, start, end, all_day, location, rrule, color) = row;
        let end = end.unwrap_or_else(|| start.clone());
        let cal_id = calendar_id.unwrap_or(0);
        let Some(from_t) = parse_utc(from) else {
            continue;
        };
        let Some(to_t) = parse_utc(to) else { continue };
        if let Some(pattern) = &rrule {
            let starts = ical::expand_rrule(pattern, &start, from, to, 730)
                .unwrap_or_else(|_| vec![start.clone()]);
            let duration = parse_utc(&end)
                .and_then(|e| parse_utc(&start).map(|s| e - s))
                .unwrap_or(chrono::Duration::hours(1));
            for occ_start in starts {
                let occ_end = if all_day {
                    // All-day rules expand to date strings; +1 day for end.
                    chrono::NaiveDate::parse_from_str(&occ_start, "%Y-%m-%d").map_or_else(
                        |_| occ_start.clone(),
                        |d| {
                            (d + chrono::Duration::days(1))
                                .format("%Y-%m-%d")
                                .to_string()
                        },
                    )
                } else {
                    parse_utc(&occ_start).map_or_else(
                        || occ_start.clone(),
                        |s| (s + duration).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    )
                };
                out.push(Occurrence {
                    event_id,
                    item_id,
                    title: title.clone(),
                    start: occ_start,
                    end: occ_end,
                    all_day,
                    location: location.clone(),
                    calendar_id: cal_id,
                    calendar_color: color.clone(),
                });
            }
        } else {
            // Single occurrence: include when it overlaps the window.
            let overlaps = all_day
                || (parse_utc(&end).is_some_and(|e| e >= from_t)
                    && parse_utc(&start).is_some_and(|s| s <= to_t));
            if overlaps {
                out.push(Occurrence {
                    event_id,
                    item_id,
                    title,
                    start,
                    end,
                    all_day,
                    location,
                    calendar_id: cal_id,
                    calendar_color: color,
                });
            }
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(out)
}

/// Editor payload for create/update (S32). `id = None` creates.
#[derive(Debug, Clone)]
pub struct EventInput {
    pub id: Option<i64>,
    pub calendar_id: i64,
    pub title: String,
    /// RFC 3339 UTC, or `YYYY-MM-DD` for all-day.
    pub start: String,
    pub end: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    /// RRULE string (e.g. `FREQ=WEEKLY;BYDAY=MO`); None = one-off.
    pub rrule: Option<String>,
    /// Minutes before start for a popup reminder; None = none. Applied via
    /// the shared reminder queue (S30).
    pub reminder_minutes: Option<i64>,
    /// Attendees (EPIM participants). Replaced wholesale on save; emails are
    /// the key (`event_participant` PK), contact links optional.
    pub participants: Vec<EventParticipantInput>,
}

/// One attendee row for [`EventInput`].
#[derive(Debug, Clone)]
pub struct EventParticipantInput {
    pub contact_id: Option<i64>,
    pub email: String,
}

/// Stored attendees of an event, for the editor.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EventParticipant {
    pub contact_id: Option<i64>,
    pub email: String,
    pub status: String,
}

/// Read an event's attendees.
pub fn event_participants(conn: &Connection, event_id: i64) -> Result<Vec<EventParticipant>> {
    let mut stmt = conn.prepare(
        "SELECT contact_id, email, status FROM event_participant \
         WHERE event_id = ?1 ORDER BY email",
    )?;
    let rows = stmt.query_map([event_id], |r| {
        Ok(EventParticipant {
            contact_id: r.get(0)?,
            email: r.get(1)?,
            status: r.get(2)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

/// Replace the attendee set (editor saves the full list each time).
fn save_participants(
    conn: &Connection,
    event_id: i64,
    participants: &[EventParticipantInput],
) -> Result<()> {
    conn.execute(
        "DELETE FROM event_participant WHERE event_id = ?1",
        [event_id],
    )?;
    for p in participants {
        let email = p.email.trim().to_lowercase();
        if email.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO event_participant (event_id, contact_id, email) \
             VALUES (?1, ?2, ?3)",
            params![event_id, p.contact_id, email],
        )?;
    }
    Ok(())
}

pub fn save_event(conn: &Connection, input: &EventInput) -> Result<i64> {
    // Stable UID: new events get one; edits keep theirs so ICS round-trips.
    let uid: Option<String> = match input.id {
        Some(id) => conn.query_row("SELECT uid FROM event WHERE id = ?1", [id], |r| r.get(0))?,
        None => Some(format!(
            "pf-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )),
    };
    let record = ical::EventRecord {
        uid,
        title: input.title.clone(),
        start: input.start.clone(),
        end: input.end.clone(),
        all_day: input.all_day,
        location: input.location.clone(),
        description: input.description.clone(),
        rrule: input.rrule.clone(),
        categories: Vec::new(),
        private: false,
    };
    let existing: Option<i64> = match input.id {
        Some(id) => conn
            .query_row("SELECT id FROM event WHERE id = ?1", [id], |r| r.get(0))
            .optional()?,
        None => None,
    };
    let event_id = if let Some(id) = existing {
        // Preserve calendar/uid, replace fields (upsert_event is UID-keyed).
        let item_id: i64 =
            conn.query_row("SELECT item_id FROM event WHERE id = ?1", [id], |r| {
                r.get(0)
            })?;
        let item = repo::get_item(conn, item_id)?;
        repo::snapshot_item(conn, &item)?;
        conn.execute(
            "UPDATE event SET title=?1, start_utc=?2, end_utc=?3, all_day=?4, location=?5, \
             description=?6, rrule=?7, calendar_id=?8 WHERE id=?9",
            params![
                record.title,
                record.start,
                record.end,
                i64::from(record.all_day),
                record.location,
                record.description,
                record.rrule,
                input.calendar_id,
                id
            ],
        )?;
        let _ = repo::touch_item(conn, item_id);
        id
    } else {
        crate::events::upsert_event(conn, input.calendar_id, &record)?
    };

    // Reminder: re-arm the queue entry for this event's anchor item.
    let item_id: i64 =
        conn.query_row("SELECT item_id FROM event WHERE id = ?1", [event_id], |r| {
            r.get(0)
        })?;
    conn.execute("DELETE FROM reminder_queue WHERE item_id = ?1", [item_id])?;
    if let Some(minutes) = input.reminder_minutes {
        if let Some(start) = parse_utc(&input.start) {
            let due = (start - chrono::Duration::minutes(minutes))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            crate::ops::schedule_reminder(conn, item_id, &due, "popup")?;
        }
    }
    save_participants(conn, event_id, &input.participants)?;
    Ok(event_id)
}

pub fn delete_event(conn: &Connection, event_id: i64) -> Result<()> {
    let item_id: i64 = conn
        .query_row("SELECT item_id FROM event WHERE id = ?1", [event_id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(Error::NotFound(event_id))?;
    conn.execute("DELETE FROM reminder_queue WHERE item_id = ?1", [item_id])?;
    repo::trash_item(conn, item_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    #[test]
    fn window_query_expands_recurrence_and_filters() {
        let s = store();
        let conn = s.conn();
        let cal = crate::events::ensure_calendar(conn, "Work", Some("#1a73e8")).unwrap();
        save_event(
            conn,
            &EventInput {
                id: None,
                calendar_id: cal,
                title: "Weekly".into(),
                start: "2026-03-02T10:00:00+00:00".into(),
                end: Some("2026-03-02T11:00:00+00:00".into()),
                all_day: false,
                location: Some("Room 1".into()),
                description: None,
                rrule: Some("FREQ=WEEKLY;COUNT=4".into()),
                reminder_minutes: Some(15),
                participants: Vec::new(),
            },
        )
        .unwrap();

        let occ = events_window(
            conn,
            Some(cal),
            "2026-03-01T00:00:00+00:00",
            "2026-04-01T00:00:00+00:00",
        )
        .unwrap();
        assert_eq!(occ.len(), 4, "weekly rule expands to 4 occurrences");
        assert!(occ[0].start.starts_with("2026-03-02T10:00:00Z"));
        assert!(occ[1].start.starts_with("2026-03-09T10:00:00Z"));
        assert_eq!(occ[0].calendar_color.as_deref(), Some("#1a73e8"));

        // Narrow window: only the occurrence inside it.
        let one = events_window(
            conn,
            Some(cal),
            "2026-03-09T00:00:00+00:00",
            "2026-03-10T00:00:00+00:00",
        )
        .unwrap();
        assert_eq!(one.len(), 1);

        // Other calendar filter excludes everything.
        assert!(events_window(
            conn,
            Some(999),
            "2026-03-01T00:00:00+00:00",
            "2026-04-01T00:00:00+00:00"
        )
        .unwrap()
        .is_empty());

        // Reminder was scheduled 15 min before the series start.
        let n: i64 = conn
            .query_row("SELECT count(*) FROM reminder_queue", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let due: String = conn
            .query_row("SELECT due_at FROM reminder_queue", [], |r| r.get(0))
            .unwrap();
        assert_eq!(due, "2026-03-02T09:45:00Z");
    }

    #[test]
    fn participants_round_trip_and_link_to_contacts() {
        let s = store();
        let conn = s.conn();
        let cal = crate::events::ensure_calendar(conn, "Work", None).unwrap();
        let contact_id = crate::contacts::upsert_contact(
            conn,
            &crate::vcard::ContactRecord {
                uid: None,
                id: 0,
                item_id: 0,
                first_name: "Ana".into(),
                last_name: "Vera".into(),
                display_name: "Ana Vera".into(),
                organization: None,
                emails: vec!["ana@example.com".into()],
                phones: vec![],
                addresses: vec![],
                urls: vec![],
                birthday: None,
                notes: None,
                categories: vec![],
            },
        )
        .unwrap();
        let id = save_event(
            conn,
            &EventInput {
                id: None,
                calendar_id: cal,
                title: "Sync".into(),
                start: "2026-03-02T10:00:00+00:00".into(),
                end: None,
                all_day: false,
                location: None,
                description: None,
                rrule: None,
                reminder_minutes: None,
                participants: vec![
                    EventParticipantInput {
                        contact_id: Some(contact_id),
                        email: "ANA@example.com".into(),
                    },
                    EventParticipantInput {
                        contact_id: None,
                        email: "bob@example.com".into(),
                    },
                    EventParticipantInput {
                        contact_id: None,
                        email: "   ".into(), // blank rows are dropped
                    },
                ],
            },
        )
        .unwrap();

        let parts = event_participants(conn, id).unwrap();
        assert_eq!(parts.len(), 2, "blank dropped, emails lowercased");
        assert_eq!(parts[0].email, "ana@example.com");
        assert_eq!(parts[0].contact_id, Some(contact_id));

        // Saving an empty list clears the set.
        save_event(
            conn,
            &EventInput {
                id: Some(id),
                calendar_id: cal,
                title: "Sync".into(),
                start: "2026-03-02T10:00:00+00:00".into(),
                end: None,
                all_day: false,
                location: None,
                description: None,
                rrule: None,
                reminder_minutes: None,
                participants: Vec::new(),
            },
        )
        .unwrap();
        assert!(event_participants(conn, id).unwrap().is_empty());
    }

    #[test]
    fn edit_updates_and_delete_trashes() {
        let s = store();
        let conn = s.conn();
        let cal = crate::events::ensure_calendar(conn, "Personal", None).unwrap();
        let id = save_event(
            conn,
            &EventInput {
                id: None,
                calendar_id: cal,
                title: "Dentist".into(),
                start: "2026-05-10T09:00:00+00:00".into(),
                end: Some("2026-05-10T10:00:00+00:00".into()),
                all_day: false,
                location: None,
                description: None,
                rrule: None,
                reminder_minutes: None,
                participants: Vec::new(),
            },
        )
        .unwrap();

        save_event(
            conn,
            &EventInput {
                id: Some(id),
                calendar_id: cal,
                title: "Dentist (moved)".into(),
                start: "2026-05-11T09:00:00+00:00".into(),
                end: Some("2026-05-11T10:00:00+00:00".into()),
                all_day: false,
                location: None,
                description: None,
                rrule: None,
                reminder_minutes: None,
                participants: Vec::new(),
            },
        )
        .unwrap();
        let (title, versions): (String, i64) = conn
            .query_row(
                "SELECT title, (SELECT count(*) FROM item_version v JOIN event e ON e.item_id = v.item_id \
                 WHERE e.id = ?1) FROM event WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "Dentist (moved)");
        assert_eq!(versions, 1, "edit snapshots the previous state");

        delete_event(conn, id).unwrap();
        assert!(events_window(
            conn,
            Some(cal),
            "2026-05-01T00:00:00+00:00",
            "2026-06-01T00:00:00+00:00"
        )
        .unwrap()
        .is_empty());
        let trashed = repo::query_trash(
            conn,
            &repo::TrashQuery {
                kind: Some(Kind::Event),
                deleted_after: None,
                deleted_before: None,
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(trashed.len(), 1);
    }
}

/// Calendar list for the sidebar (S31).
#[derive(Debug, Clone, Serialize)]
pub struct CalendarInfo {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

#[must_use]
pub fn calendars_list(conn: &Connection) -> Vec<CalendarInfo> {
    let Ok(mut stmt) = conn.prepare("SELECT id, name, color FROM calendar ORDER BY name") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(CalendarInfo {
            id: r.get(0)?,
            name: r.get(1)?,
            color: r.get(2)?,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(std::result::Result::ok).collect()
}

pub fn calendar_create(conn: &Connection, name: &str, color: Option<&str>) -> Result<i64> {
    crate::events::ensure_calendar(conn, name, color)
}

/// Import ICS text (S13 import path for calendar); returns (imported, warnings).
pub fn import_ics(conn: &Connection, text: &str, calendar_id: i64) -> Result<(usize, Vec<String>)> {
    let parsed = ical::parse_ics(text);
    let mut n = 0;
    for ev in &parsed.events {
        crate::events::upsert_event(conn, calendar_id, ev)?;
        n += 1;
    }
    Ok((n, parsed.warnings))
}

/// Export every event as ICS to `path`; returns the event count.
pub fn export_ics(conn: &Connection, path: &std::path::Path) -> Result<usize> {
    let mut events = Vec::new();
    for cal in calendars_list(conn) {
        events.extend(crate::events::events_of_calendar(conn, cal.id));
    }
    std::fs::write(path, ical::write_ics(&events, "Smart PIM"))?;
    Ok(events.len())
}

#[cfg(test)]
mod io_tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn ics_import_export_roundtrip_through_repo() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let cal = calendar_create(conn, "Imported", Some("#0aa")).unwrap();
        let (n, warnings) = import_ics(
            conn,
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:ext-1\r\nSUMMARY:From file\r\nDTSTART:20260701T080000Z\r\nDTEND:20260701T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            cal,
        )
        .unwrap();
        assert_eq!((n, warnings.len()), (1, 0));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.ics");
        assert_eq!(export_ics(conn, &path).unwrap(), 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("SUMMARY:From file"));
        // Re-import is idempotent (same UID).
        let (n2, _) = import_ics(conn, &text, cal).unwrap();
        assert_eq!(n2, 1);
        assert_eq!(calendars_list(conn).len(), 1);
    }
}
