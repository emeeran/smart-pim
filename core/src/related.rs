//! Cross-module related items + the Today dashboard aggregation.
//!
//! Related items (EPIM's "Related Items" tabs): for a contact this is
//! query-time association — their mail addresses match message senders/
//! recipients, `task.assigned_contact_id`, `event_participant` rows — plus
//! explicit `item_link` edges. For any other item it is the explicit link
//! graph. Nothing is copied: links stay live as the underlying rows change.
//!
//! Today dashboard: one snapshot for the landing widgets (agenda, due
//! tasks, per-account unread, upcoming anniversaries). Pure reads; no
//! network, per the local-first rule (`docs/ai_integ_plan.md` §1).

use chrono::Datelike;
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::Result;
use crate::repo;

/// One related row, shaped for the shared UI panel. `id` is the module-row
/// id the UI navigates by (message id, task id, …), `item_id` the universal
/// anchor, `when` a display-ready date.
#[derive(Debug, Clone, Serialize)]
pub struct RelatedItem {
    pub kind: String, // message | task | event | note | diary | contact | other
    pub id: i64,
    pub item_id: i64,
    pub title: String,
    pub detail: Option<String>,
    pub when: Option<String>,
}

fn message_rows(
    conn: &Connection,
    where_clause: &str,
    param: &str,
    limit: i64,
) -> Vec<RelatedItem> {
    let sql = format!(
        "SELECT m.id, m.item_id, m.subject, m.from_name, m.from_addr, m.date \
         FROM message m WHERE {where_clause} ORDER BY m.date DESC LIMIT ?2"
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let mapped = stmt.query_map(params![param, limit], |r| {
        let name: String = r.get(3)?;
        let from = if name.is_empty() {
            r.get::<_, String>(4)?
        } else {
            name
        };
        Ok(RelatedItem {
            kind: "message".into(),
            id: r.get(0)?,
            // Messages synced from IMAP may not carry an item anchor.
            item_id: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            title: r.get::<_, String>(2)?,
            detail: Some(from),
            when: Some(r.get(5)?),
        })
    });
    match mapped.map(|it| it.collect::<std::result::Result<Vec<RelatedItem>, _>>()) {
        Ok(Ok(rows)) => rows,
        _ => Vec::new(),
    }
}

fn tasks_for_contact(conn: &Connection, contact_id: i64) -> Result<Vec<RelatedItem>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.item_id, t.title, t.due_utc FROM task t \
         JOIN item i ON i.id = t.item_id AND i.deleted_at IS NULL \
         WHERE t.assigned_contact_id = ?1 \
         ORDER BY (t.due_utc IS NULL), t.due_utc LIMIT 20",
    )?;
    let rows = stmt.query_map([contact_id], |r| {
        Ok(RelatedItem {
            kind: "task".into(),
            id: r.get(0)?,
            item_id: r.get(1)?,
            title: r.get(2)?,
            detail: None,
            when: r.get(3)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

fn events_for_contact(conn: &Connection, contact_id: i64, email: &str) -> Result<Vec<RelatedItem>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT e.id, e.item_id, e.title, e.start_utc FROM event e \
         JOIN item i ON i.id = e.item_id AND i.deleted_at IS NULL \
         LEFT JOIN event_participant p ON p.event_id = e.id \
         WHERE p.contact_id = ?1 OR lower(p.email) = lower(?2) \
         ORDER BY e.start_utc LIMIT 20",
    )?;
    let rows = stmt.query_map(params![contact_id, email], |r| {
        Ok(RelatedItem {
            kind: "event".into(),
            id: r.get(0)?,
            item_id: r.get(1)?,
            title: r.get(2)?,
            detail: None,
            when: r.get(3)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

/// Everything associated with a contact: mail to/from any of their
/// addresses, assigned tasks, events they participate in, and explicitly
/// linked items. Deduplicated by (kind, id), mail first (EPIM ordering).
pub fn for_contact(conn: &Connection, contact_id: i64) -> Vec<RelatedItem> {
    let emails: Vec<String> = match conn
        .prepare(
            "SELECT value FROM contact_field WHERE contact_id = ?1 AND kind = 'email' LIMIT 10",
        )
        .and_then(|mut s| {
            s.query_map([contact_id], |r| r.get::<_, String>(0))
                .map(|it| it.flatten().collect())
        }) {
        Ok(list) => list,
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<RelatedItem> = Vec::new();
    for email in &emails {
        // Sender match is exact; recipients live in a CSV column.
        out.extend(message_rows(conn, "m.from_addr = ?1", email, 20));
        let like = format!("%{email}%");
        out.extend(message_rows(
            conn,
            "m.to_addrs LIKE ?1 AND m.from_addr <> ?1",
            &like,
            20,
        ));
    }
    if let Ok(tasks) = tasks_for_contact(conn, contact_id) {
        out.extend(tasks);
    }
    if let Some(primary) = emails.first() {
        if let Ok(events) = events_for_contact(conn, contact_id, primary) {
            out.extend(events);
        }
    }

    // Explicit links, labelled through the item graph.
    if let Ok(item_id) = conn.query_row(
        "SELECT item_id FROM contact WHERE id = ?1",
        [contact_id],
        |r| r.get::<_, i64>(0),
    ) {
        for other in repo::related_items(conn, item_id) {
            if let Some((kind, title)) = item_label(conn, other) {
                out.push(RelatedItem {
                    kind,
                    id: 0,
                    item_id: other,
                    title,
                    detail: None,
                    when: None,
                });
            }
        }
    }

    dedupe(out)
}

/// Explicit links for any non-contact item.
#[must_use]
pub fn for_item(conn: &Connection, item_id: i64) -> Vec<RelatedItem> {
    repo::related_items(conn, item_id)
        .into_iter()
        .filter_map(|other| {
            item_label(conn, other).map(|(kind, title)| RelatedItem {
                kind,
                id: 0,
                item_id: other,
                title,
                detail: None,
                when: None,
            })
        })
        .collect()
}

fn item_label(conn: &Connection, item_id: i64) -> Option<(String, String)> {
    let kind: String = conn
        .query_row("SELECT kind FROM item WHERE id = ?1", [item_id], |r| {
            r.get(0)
        })
        .ok()?;
    let title: String = match kind.as_str() {
        "note" => conn.query_row(
            "SELECT title FROM note WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        ),
        "task" => conn.query_row(
            "SELECT title FROM task WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        ),
        "diary" => conn.query_row(
            "SELECT entry_date FROM diary_entry WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        ),
        "event" => conn.query_row(
            "SELECT title FROM event WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        ),
        "contact" => conn.query_row(
            "SELECT display_name FROM contact WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        ),
        _ => Ok(kind.clone()),
    }
    .unwrap_or_else(|_| format!("Item #{item_id}"));
    Some((kind, title))
}

fn dedupe(items: Vec<RelatedItem>) -> Vec<RelatedItem> {
    // Module rows (message/task ids) dedupe on their own id; a link-derived
    // row (id 0) is dropped when a module row for the same item exists.
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_items = std::collections::HashSet::new();
    let mut out = Vec::new();
    for it in items {
        if it.id != 0 {
            if seen_ids.insert((it.kind.clone(), it.id)) {
                seen_items.insert((it.kind.clone(), it.item_id));
                out.push(it);
            }
        } else if seen_items.insert((it.kind.clone(), it.item_id)) {
            out.push(it);
        }
    }
    out
}

// ------------------------------------------------------------------ today

#[derive(Debug, Clone, Serialize)]
pub struct AccountUnread {
    pub id: i64,
    pub name: String,
    pub unread: i64,
}

/// An upcoming birthday/anniversary with the age or years-being-marked at
/// the next occurrence.
#[derive(Debug, Clone, Serialize)]
pub struct Anniversary {
    pub contact_id: i64,
    pub name: String,
    pub kind: String, // birthday | anniversary
    /// Next occurrence, `MM-DD`-shaped `YYYY-MM-DD`.
    pub date: String,
    pub days_ahead: i64,
    pub years: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TodayDashboard {
    pub events: Vec<crate::calrepo::Occurrence>,
    /// Open tasks due today or overdue.
    pub tasks: Vec<crate::tasks::TaskRow>,
    pub accounts: Vec<AccountUnread>,
    pub anniversaries: Vec<Anniversary>,
    pub unread_messages: i64,
}

fn next_occurrence(month_day: &str, today: chrono::NaiveDate) -> Option<(chrono::NaiveDate, i64)> {
    // month_day is the stored `YYYY-MM-DD`; take its MM-DD.
    let md = month_day.get(5..)?; // "MM-DD"
    let this_year = today.year();
    let candidate =
        |y: i32| chrono::NaiveDate::parse_from_str(&format!("{y}-{md}"), "%Y-%m-%d").ok();
    let next = candidate(this_year)
        .filter(|d| *d >= today)
        .or_else(|| candidate(this_year + 1))?;
    let years = next.year()
        - chrono::NaiveDate::parse_from_str(month_day, "%Y-%m-%d")
            .ok()?
            .year();
    Some((next, i64::from(years)))
}

/// Today's landing snapshot. `today` is `YYYY-MM-DD` in the user's local
/// zone (the shell passes its own clock; core stays zone-agnostic).
///
/// # Errors
/// Propagates SQL errors.
pub fn today_dashboard(conn: &Connection, today: &str) -> Result<TodayDashboard> {
    let date = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").map_err(|_| {
        crate::error::Error::Parse {
            what: "today",
            message: "expected YYYY-MM-DD".into(),
        }
    })?;
    let day_end = format!("{today}T23:59:59+00:00");

    let events = crate::calrepo::events_window(conn, None, today, &day_end)?;
    // Open tasks due today or overdue (due date < end of today).
    let all = crate::tasks::tasks_open(conn, 500)?;
    let today_prefix = format!("{today}T");
    let mut tasks: Vec<crate::tasks::TaskRow> = all
        .into_iter()
        .filter(|t| {
            t.due_utc
                .as_deref()
                .is_some_and(|d| d < day_end.as_str() || d.starts_with(&today_prefix))
        })
        .collect();
    tasks.sort_by(|a, b| a.due_utc.cmp(&b.due_utc));
    tasks.truncate(20);

    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, (SELECT count(*) FROM message m \
         JOIN mail_folder f ON f.id = m.folder_id WHERE f.account_id = a.id \
         AND (json_extract(m.flags, '$.seen') IS NULL OR json_extract(m.flags, '$.seen') = 0)) \
         FROM mail_account a ORDER BY a.id",
    )?;
    let accounts = stmt
        .query_map([], |r| {
            Ok(AccountUnread {
                id: r.get(0)?,
                name: r.get(1)?,
                unread: r.get(2)?,
            })
        })?
        .flatten()
        .collect::<Vec<_>>();
    let unread_messages = accounts.iter().map(|a| a.unread).sum();

    // Upcoming birthdays/anniversaries across the next 30 days.
    let mut stmt = conn.prepare(
        "SELECT c.id, c.display_name, c.first_name, c.birthday, c.anniversary \
         FROM contact c JOIN item i ON i.id = c.item_id AND i.deleted_at IS NULL \
         WHERE c.birthday IS NOT NULL OR c.anniversary IS NOT NULL",
    )?;
    let mut anniversaries: Vec<Anniversary> = Vec::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    for (contact_id, display, first, birthday, anniversary) in rows.flatten() {
        let name = if display.trim().is_empty() {
            first
        } else {
            display
        };
        for (kind, stored) in [("birthday", birthday), ("anniversary", anniversary)] {
            let Some(stored) = stored else { continue };
            if let Some((next, years)) = next_occurrence(&stored, date) {
                let days = (next - date).num_days();
                if (0..=30).contains(&days) {
                    anniversaries.push(Anniversary {
                        contact_id,
                        name: name.clone(),
                        kind: kind.into(),
                        date: next.format("%Y-%m-%d").to_string(),
                        days_ahead: days,
                        years,
                    });
                }
            }
        }
    }
    anniversaries.sort_by_key(|a| a.days_ahead);
    anniversaries.truncate(10);

    Ok(TodayDashboard {
        events,
        tasks,
        accounts,
        anniversaries,
        unread_messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts;
    use crate::model::Kind;
    use crate::store::Store;
    use crate::tasks;
    use crate::vcard::ContactRecord;

    fn contact(conn: &Connection, name: &str, email: &str, birthday: Option<&str>) -> i64 {
        let (first, last) = name.split_once(' ').unwrap_or((name, ""));
        contacts::upsert_contact(
            conn,
            &ContactRecord {
                uid: None,
                id: 0,
                item_id: 0,
                first_name: first.into(),
                last_name: last.into(),
                display_name: name.into(),
                organization: None,
                emails: vec![email.into()],
                phones: vec![],
                addresses: vec![],
                urls: vec![],
                birthday: birthday.map(str::to_string),
                notes: None,
                categories: vec![],
            },
        )
        .unwrap()
    }

    fn message(conn: &Connection, from: &str, to: &str, subject: &str) -> i64 {
        conn.execute(
            "INSERT OR IGNORE INTO mail_account (id, name, kind) VALUES (1, 'T', 'imap')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO mail_folder (id, account_id, name) VALUES (1, 1, 'INBOX')",
            [],
        )
        .unwrap();
        let uid: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(uid), 0) + 1 FROM message WHERE folder_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO message (account_id, folder_id, uid, subject, from_name, from_addr, \
             to_addrs, date, body, created_at, flags) \
             VALUES (1, 1, ?4, ?3, 'Ana Vera', ?1, ?2, '2026-10-18', '', 'x', '{\"seen\":0}')",
            params![from, to, subject, uid],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn contact_related_finds_mail_tasks_and_events() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let cid = contact(conn, "Ana Vera", "ana@example.com", Some("1990-03-08"));
        let _m = message(conn, "ana@example.com", "me@x.com", "Quarterly numbers");

        // Task assigned to Ana.
        let task_id = tasks::save_task(
            conn,
            &tasks::TaskInput {
                id: None,
                title: "Call Ana".into(),
                notes: None,
                due_utc: None,
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE task SET assigned_contact_id = ?1 WHERE id = ?2",
            params![cid, task_id],
        )
        .unwrap();

        // Explicit link: the task linked to Ana's item.
        let contact_item: i64 = conn
            .query_row("SELECT item_id FROM contact WHERE id = ?1", [cid], |r| {
                r.get(0)
            })
            .unwrap();
        let task_item: i64 = conn
            .query_row("SELECT item_id FROM task WHERE id = ?1", [task_id], |r| {
                r.get(0)
            })
            .unwrap();
        repo::link_items(conn, contact_item, task_item).unwrap();

        let rel = for_contact(conn, cid);
        let kinds: Vec<_> = rel.iter().map(|r| r.kind.as_str()).collect();
        assert!(kinds.contains(&"message"), "mail found: {rel:?}");
        assert!(kinds.contains(&"task"));
        // The task appears once — link deduped against the assignment.
        assert_eq!(rel.iter().filter(|r| r.kind == "task").count(), 1);
        assert_eq!(rel[0].title, "Quarterly numbers", "mail first, newest");

        // Recipient-side match.
        let _m2 = message(conn, "other@x.com", "ana@example.com", "Re: numbers");
        let rel = for_contact(conn, cid);
        assert_eq!(rel.iter().filter(|r| r.kind == "message").count(), 2);
    }

    #[test]
    fn for_item_lists_explicit_links() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let a = repo::create_item(conn, Kind::Note, None, false).unwrap();
        let b = repo::create_item(conn, Kind::Task, None, false).unwrap();
        repo::link_items(conn, a, b).unwrap();
        let rel = for_item(conn, a);
        assert_eq!(rel.len(), 1);
        assert_eq!(rel[0].kind, "task");
    }

    #[test]
    fn today_dashboard_aggregates() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let _m = message(conn, "a@x.com", "me@x.com", "one");
        conn.execute("UPDATE message SET flags = '{\"seen\":1}'", [])
            .unwrap();
        message(conn, "b@x.com", "me@x.com", "two");

        // Task due today (2026-10-20) and one overdue.
        tasks::save_task(
            conn,
            &tasks::TaskInput {
                id: None,
                title: "today".into(),
                notes: None,
                due_utc: Some("2026-10-20T10:00:00+00:00".into()),
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();
        tasks::save_task(
            conn,
            &tasks::TaskInput {
                id: None,
                title: "overdue".into(),
                notes: None,
                due_utc: Some("2026-10-01T10:00:00+00:00".into()),
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();
        // Future task must not appear.
        tasks::save_task(
            conn,
            &tasks::TaskInput {
                id: None,
                title: "later".into(),
                notes: None,
                due_utc: Some("2026-12-01T10:00:00+00:00".into()),
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();

        contact(conn, "Nasser SMA", "n@x.com", Some("1957-10-25"));
        let dash = today_dashboard(conn, "2026-10-20").unwrap();
        assert_eq!(dash.unread_messages, 1);
        assert_eq!(dash.accounts.len(), 1);
        let titles: Vec<_> = dash.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["overdue", "today"],
            "overdue first, future excluded"
        );
        assert_eq!(dash.anniversaries.len(), 1);
        assert_eq!(dash.anniversaries[0].days_ahead, 5);
        assert_eq!(dash.anniversaries[0].years, 69, "turning 69 on 2026-10-25");
        assert!(today_dashboard(conn, "not-a-date").is_err());
    }
}
