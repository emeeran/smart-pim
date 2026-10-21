//! Find-a-slot (`docs/ai_integ_plan.md`, "Calendar"): pure computation over the
//! event table — merge busy intervals, scan work hours for gaps of at least
//! the requested duration. The LLM, if any, only phrases the result.

use rusqlite::Connection;

use crate::calrepo;
use crate::error::Result;

/// One free interval, ISO-8601 UTC strings.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FreeSlot {
    pub start: String,
    pub end: String,
}

/// Busy intervals for `calendar_id` (or all calendars) overlapping `[from,
/// to]`, merged into disjoint, sorted intervals. Deleted events excluded by
/// the `events_window` query.
fn busy_intervals(
    conn: &Connection,
    calendar_id: Option<i64>,
    from: &str,
    to: &str,
) -> Result<Vec<(String, String)>> {
    let mut busy: Vec<(String, String)> = calrepo::events_window(conn, calendar_id, from, to)?
        .into_iter()
        .map(|o| (o.start, o.end))
        .collect();
    busy.sort();
    let mut merged: Vec<(String, String)> = Vec::with_capacity(busy.len());
    for (start, end) in busy {
        match merged.last_mut() {
            Some(last) if start <= last.1 => {
                if end > last.1 {
                    last.1 = end;
                }
            }
            _ => merged.push((start, end)),
        }
    }
    Ok(merged)
}

/// Propose up to `max` free windows of `duration_min` inside work hours
/// (default 08:00–18:00 UTC) between `from` and `to` (ISO instants or dates;
/// parsed leniently — a bare `YYYY-MM-DD` means midnight).
///
/// # Errors
/// Propagates SQL errors and [`crate::error::Error::Parse`] for unparseable
/// bounds.
pub fn free_slots(
    conn: &Connection,
    calendar_id: Option<i64>,
    from: &str,
    to: &str,
    duration_min: i64,
    work_hours: Option<(u8, u8)>,
    max: usize,
) -> Result<Vec<FreeSlot>> {
    let (start_dt, end_dt) = (parse_bound(from)?, parse_bound(to)?);
    if end_dt <= start_dt {
        return Err(crate::error::Error::parse(
            "free_slots",
            "end is not after start",
        ));
    }
    let (wh_start, wh_end) = work_hours.unwrap_or((8, 18));
    let busy = busy_intervals(conn, calendar_id, from, to)?;
    let mut out = Vec::new();
    let mut day = start_dt.date_naive();
    while day <= end_dt.date_naive() && out.len() < max {
        let day_start: Option<DateTime> = day
            .and_hms_opt(u32::from(wh_start), 0, 0)
            .map(utc_from_naive);
        let day_end: Option<DateTime> =
            day.and_hms_opt(u32::from(wh_end), 0, 0).map(utc_from_naive);
        if let (Some(day_start), Some(day_end)) = (day_start, day_end) {
            let mut cursor = max_time(day_start, start_dt);
            let limit = min_time(day_end, end_dt);
            for (b_start, b_end) in &busy {
                let (b_start, b_end) = (parse_bound(b_start)?, parse_bound(b_end)?);
                if b_end <= cursor {
                    continue;
                }
                if b_start >= limit {
                    break;
                }
                if let Some(slot) = gap(&cursor, &min_time(b_start, limit), duration_min) {
                    out.push(slot);
                    if out.len() >= max {
                        break;
                    }
                }
                if b_end > cursor {
                    cursor = b_end;
                }
            }
            if out.len() < max {
                if let Some(slot) = gap(&cursor, &limit, duration_min) {
                    out.push(slot);
                }
            }
        }
        day = day.succ_opt().unwrap_or(day);
    }
    Ok(out)
}

fn gap(
    a: &chrono::DateTime<chrono::Utc>,
    b: &chrono::DateTime<chrono::Utc>,
    mins: i64,
) -> Option<FreeSlot> {
    let secs = b.timestamp() - a.timestamp();
    if secs >= mins.max(1) * 60 {
        return Some(FreeSlot {
            start: fmt(a),
            end: fmt(&(*a + chrono::Duration::minutes(mins.max(1)))),
        });
    }
    None
}

type DateTime = chrono::DateTime<chrono::Utc>;

fn parse_bound(s: &str) -> Result<DateTime> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.into());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(&s[..10.min(s.len())], "%Y-%m-%d") {
        return Ok(utc_from_naive(d.and_hms_opt(0, 0, 0).expect("midnight")));
    }
    Err(crate::error::Error::parse(
        "free_slots",
        format!("unparseable bound {s}"),
    ))
}

fn utc_from_naive(n: chrono::NaiveDateTime) -> DateTime {
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(n, chrono::Utc)
}

fn fmt(d: &DateTime) -> String {
    d.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

fn max_time(a: DateTime, b: DateTime) -> DateTime {
    if a > b {
        a
    } else {
        b
    }
}

fn min_time(a: DateTime, b: DateTime) -> DateTime {
    if a < b {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    fn add_event(s: &Store, cal: i64, title: &str, start: &str, end: &str) {
        let item =
            crate::repo::create_item(s.conn(), crate::model::Kind::Event, None, false).unwrap();
        s.conn()
            .execute(
                "INSERT INTO event (item_id, calendar_id, title, start_utc, end_utc) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![item, cal, title, start, end],
            )
            .unwrap();
    }

    #[test]
    fn finds_gap_between_and_around_events() {
        let s = store();
        let cal = crate::calrepo::calendar_create(s.conn(), "work", None).unwrap();
        // Busy 10:00–11:00 and 13:00–14:00 on 2026-10-20.
        add_event(
            &s,
            cal,
            "a",
            "2026-10-20T10:00:00+00:00",
            "2026-10-20T11:00:00+00:00",
        );
        add_event(
            &s,
            cal,
            "b",
            "2026-10-20T13:00:00+00:00",
            "2026-10-20T14:00:00+00:00",
        );

        let slots = free_slots(
            s.conn(),
            Some(cal),
            "2026-10-20T08:00:00+00:00",
            "2026-10-20T18:00:00+00:00",
            60,
            None,
            10,
        )
        .unwrap();
        let starts: Vec<&str> = slots.iter().map(|x| x.start.as_str()).collect();
        assert_eq!(
            starts,
            vec![
                "2026-10-20T08:00:00+00:00",
                "2026-10-20T11:00:00+00:00",
                "2026-10-20T14:00:00+00:00",
            ]
        );
    }

    #[test]
    fn merges_overlapping_busy_and_skips_short_gaps() {
        let s = store();
        let cal = crate::calrepo::calendar_create(s.conn(), "work", None).unwrap();
        add_event(
            &s,
            cal,
            "a",
            "2026-10-20T09:00:00+00:00",
            "2026-10-20T10:30:00+00:00",
        );
        add_event(
            &s,
            cal,
            "b",
            "2026-10-20T10:00:00+00:00",
            "2026-10-20T12:00:00+00:00",
        );
        // Only 08:00–09:00 (60 min) and 12:00–18:00 fit; a 30-min meeting
        // inside the merged block must not appear.
        let slots = free_slots(
            s.conn(),
            Some(cal),
            "2026-10-20T08:00:00+00:00",
            "2026-10-20T18:00:00+00:00",
            30,
            None,
            10,
        )
        .unwrap();
        assert_eq!(slots[0].start, "2026-10-20T08:00:00+00:00");
        assert!(slots
            .iter()
            .all(|x| x.start.as_str() < "2026-10-20T09:00:00+00:00"
                || x.start.as_str() >= "2026-10-20T12:00:00+00:00"));
    }

    #[test]
    fn respects_work_hours_and_max() {
        let s = store();
        let slots = free_slots(
            s.conn(),
            None,
            "2026-10-20T00:00:00+00:00",
            "2026-10-22T00:00:00+00:00",
            45,
            Some((9, 10)),
            2,
        )
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert!(slots.iter().all(|x| x.start.ends_with("T09:00:00+00:00")));
    }

    #[test]
    fn rejects_inverted_range_and_garbage() {
        let s = store();
        assert!(free_slots(s.conn(), None, "2026-10-20", "2026-10-19", 30, None, 5).is_err());
        assert!(free_slots(s.conn(), None, "nope", "2026-10-19", 30, None, 5).is_err());
    }
}
