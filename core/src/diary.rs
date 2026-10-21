//! Diary module ("Diary" parity with the reference PIM): one entry per local calendar
//! day. Content is HTML (same Tiptap pipeline as notes); dates are local
//! `YYYY-MM-DD` strings, never UTC conversions.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::Kind;
use crate::repo;

#[derive(Debug, Clone, Serialize)]
pub struct DiaryRow {
    pub item_id: i64,
    pub entry_date: String,
    pub content: String,
    pub mood: Option<String>,
    pub word_count: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiaryMonth {
    pub year: i32,
    pub month: u8,
    pub entry_count: i64,
}

#[must_use]
pub fn word_count(html: &str) -> i64 {
    // Minimal tag stripper: word counting only, not a sanitizer.
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.split_whitespace().count() as i64
}

/// Entries between two local dates (inclusive), ascending.
pub fn diary_range(conn: &Connection, from: &str, to: &str) -> Result<Vec<DiaryRow>> {
    let mut stmt = conn.prepare(
        "SELECT d.item_id, d.entry_date, d.content, d.mood, i.updated_at \
         FROM diary_entry d JOIN item i ON i.id = d.item_id \
         WHERE i.deleted_at IS NULL AND d.entry_date BETWEEN ?1 AND ?2 \
         ORDER BY d.entry_date ASC",
    )?;
    let rows = stmt.query_map(params![from, to], map_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn diary_get(conn: &Connection, date: &str) -> Result<Option<DiaryRow>> {
    let mut stmt = conn.prepare(
        "SELECT d.item_id, d.entry_date, d.content, d.mood, i.updated_at \
         FROM diary_entry d JOIN item i ON i.id = d.item_id \
         WHERE d.entry_date = ?1 AND i.deleted_at IS NULL",
    )?;
    let mut rows = stmt.query_map([date], map_row)?;
    rows.next().transpose().map_err(Error::from)
}

/// Create or update the entry for `date`. Returns the entry's item id.
/// Snapshots the previous content into version history on real changes.
pub fn diary_upsert(
    conn: &Connection,
    date: &str,
    content: &str,
    mood: Option<&str>,
) -> Result<i64> {
    let existing: Option<(i64, String, Option<String>)> = conn
        .query_row(
            "SELECT item_id, content, mood FROM diary_entry WHERE entry_date = ?1",
            [date],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((item_id, old_content, old_mood)) = existing {
        if old_content == content && old_mood.as_deref() == mood {
            return Ok(item_id);
        }
        let item = repo::get_item(conn, item_id)?;
        repo::snapshot_item(conn, &item)?;
        conn.execute(
            "UPDATE diary_entry SET content = ?2, mood = ?3 WHERE entry_date = ?1",
            params![date, content, mood],
        )?;
        let _ = repo::touch_item(conn, item_id);
        repo::audit(
            conn,
            Some(item_id),
            "diary_save",
            &serde_json::json!({ "date": date }),
        )?;
        return Ok(item_id);
    }
    let item_id = repo::create_item(conn, Kind::Diary, None, false)?;
    conn.execute(
        "INSERT INTO diary_entry (item_id, entry_date, content, mood) VALUES (?1, ?2, ?3, ?4)",
        params![item_id, date, content, mood],
    )?;
    repo::audit(
        conn,
        Some(item_id),
        "diary_create",
        &serde_json::json!({ "date": date }),
    )?;
    Ok(item_id)
}

/// Move the entry for `date` to trash (restorable via `restore_item`).
pub fn diary_delete(conn: &Connection, date: &str) -> Result<()> {
    let item_id: i64 = conn
        .query_row(
            "SELECT item_id FROM diary_entry WHERE entry_date = ?1",
            [date],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(0))?;
    repo::trash_item(conn, item_id)
}

/// Per-month entry counts for the tree navigator's year view.
pub fn diary_months(conn: &Connection, year: i32) -> Result<Vec<DiaryMonth>> {
    let mut stmt = conn.prepare(
        "SELECT substr(entry_date, 1, 4) AS y, substr(entry_date, 6, 2) AS m, count(*) \
         FROM diary_entry d JOIN item i ON i.id = d.item_id \
         WHERE i.deleted_at IS NULL AND y = ?1 GROUP BY m ORDER BY m",
    )?;
    let rows = stmt.query_map([year.to_string()], |r| {
        Ok(DiaryMonth {
            year: r.get::<_, String>(0)?.parse().unwrap_or(year),
            month: r.get::<_, String>(1)?.parse().unwrap_or(1),
            entry_count: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Consecutive days with entries ending at `today` (or yesterday when today
/// has no entry yet — the streak is not broken before the day is over).
pub fn diary_streak(conn: &Connection, today: &str) -> Result<i64> {
    let mut stmt = conn.prepare(
        "SELECT entry_date FROM diary_entry d JOIN item i ON i.id = d.item_id \
         WHERE i.deleted_at IS NULL",
    )?;
    let dates: std::collections::HashSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(std::result::Result::ok)
        .collect();
    let mut cursor = parse_date(today);
    if cursor.is_none() {
        return Ok(0);
    }
    if !dates.contains(today) {
        cursor = Some(prev_day(cursor.expect("checked above")));
    }
    let mut streak = 0i64;
    while let Some(d) = cursor {
        let key = fmt_date(d.0, d.1, d.2);
        if dates.contains(&key) {
            streak += 1;
            cursor = Some(prev_day(d));
        } else {
            break;
        }
    }
    Ok(streak)
}

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DiaryRow> {
    let content: String = r.get(2)?;
    Ok(DiaryRow {
        item_id: r.get(0)?,
        entry_date: r.get(1)?,
        word_count: word_count(&content),
        mood: r.get(3)?,
        updated_at: r.get(4)?,
        content,
    })
}

/// `YYYY-MM-DD` → `(y, m, d)`; `None` on anything else.
fn parse_date(s: &str) -> Option<(i32, u8, u8)> {
    let mut it = s.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some((y, m, d))
}

fn fmt_date(y: i32, m: u8, d: u8) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn prev_day(s: (i32, u8, u8)) -> (i32, u8, u8) {
    let (mut y, mut m, mut d) = s;
    if d > 1 {
        return (y, m, d - 1);
    }
    if m > 1 {
        m -= 1;
        d = days_in_month(y, m);
        return (y, m, d);
    }
    y -= 1;
    (y, 12, 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    #[test]
    fn upsert_get_roundtrip_and_unique() {
        let s = store();
        let conn = s.conn();
        let id1 = diary_upsert(conn, "2026-10-19", "<p>first day</p>", Some("calm")).unwrap();
        let id2 = diary_upsert(conn, "2026-10-19", "<p>first day revised</p>", None).unwrap();
        assert_eq!(id1, id2); // update, not duplicate
        let n: i64 = conn
            .query_row("SELECT count(*) FROM diary_entry", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let row = diary_get(conn, "2026-10-19").unwrap().unwrap();
        assert_eq!(row.content, "<p>first day revised</p>");
        assert_eq!(row.mood, None);
        assert_eq!(row.word_count, 3);
        assert!(diary_get(conn, "2026-10-18").unwrap().is_none());
    }

    #[test]
    fn range_and_months() {
        let s = store();
        let conn = s.conn();
        diary_upsert(conn, "2026-01-02", "<p>a</p>", None).unwrap();
        diary_upsert(conn, "2026-01-31", "<p>b</p>", None).unwrap();
        diary_upsert(conn, "2026-02-01", "<p>c</p>", None).unwrap();
        let jan = diary_range(conn, "2026-01-01", "2026-01-31").unwrap();
        assert_eq!(jan.len(), 2);
        assert_eq!(jan[0].entry_date, "2026-01-02");
        let months = diary_months(conn, 2026).unwrap();
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].month, 1);
        assert_eq!(months[0].entry_count, 2);
    }

    #[test]
    fn delete_goes_to_trash_and_restores() {
        let s = store();
        let conn = s.conn();
        let item_id = diary_upsert(conn, "2026-03-01", "<p>x</p>", None).unwrap();
        diary_delete(conn, "2026-03-01").unwrap();
        assert!(diary_get(conn, "2026-03-01").unwrap().is_none());
        repo::restore_item(conn, item_id).unwrap();
        assert!(diary_get(conn, "2026-03-01").unwrap().is_some());
    }

    #[test]
    fn streak_counts_consecutive_days() {
        let s = store();
        let conn = s.conn();
        diary_upsert(conn, "2026-10-17", "<p>a</p>", None).unwrap();
        diary_upsert(conn, "2026-10-18", "<p>b</p>", None).unwrap();
        assert_eq!(diary_streak(conn, "2026-10-19").unwrap(), 2);
        diary_upsert(conn, "2026-10-19", "<p>c</p>", None).unwrap();
        assert_eq!(diary_streak(conn, "2026-10-19").unwrap(), 3);
        assert_eq!(diary_streak(conn, "2026-10-21").unwrap(), 0);
    }

    #[test]
    fn prev_day_handles_month_and_year_rolls() {
        assert_eq!(prev_day((2026, 3, 1)), (2026, 2, 28));
        assert_eq!(prev_day((2024, 3, 1)), (2024, 2, 29)); // leap year
        assert_eq!(prev_day((2026, 1, 1)), (2025, 12, 31));
        assert_eq!(fmt_date(2026, 3, 1), "2026-03-01");
    }
}
