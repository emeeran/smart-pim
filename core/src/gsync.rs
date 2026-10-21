//! Google Calendar/Tasks two-way sync engine (schema v7 `google_link`).
//!
//! The engine is transport-free: callers hand it a [`GoogleClient`]
//! implementation, so the push/pull/LWW logic is unit-testable against a
//! fake. Per run:
//!
//! 1. push local tombstones, then live local rows that changed since the
//!    last successful sync (`item.updated_at` > `last_sync`, or an explicit
//!    `sync_state != 'clean'` override) — remote ids and `sync_updated_at`
//!    are stamped with direct SQL so pulled rows never re-mark dirty;
//! 2. pull remote changes incrementally (events via `syncToken`, tasks via
//!    `updatedMin`), applying last-writer-wins by remote `updated` against
//!    the locally stamped `sync_updated_at`;
//! 3. stamp the watermarks only after a fully successful pull. An expired
//!    events token (HTTP 410) is retried once without a token; the full
//!    resync is idempotent because upserts key on the remote id / UID.
//!
//! Recurring events keep their RRULE verbatim (matching ICS import);
//! `RECURRENCE-ID` instances are skipped in v1. Google tasks have no
//! priority, so `task.priority` always stays local.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};

/// Transport/API failure modes the engine reacts to.
#[derive(Debug, PartialEq)]
pub enum GError {
    /// HTTP 410 — the incremental token expired; retry without it.
    Gone,
    /// Any other failure; surfaces in `google_link.last_error`.
    Api(String),
}

impl std::fmt::Display for GError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GError::Gone => write!(f, "sync token expired (410)"),
            GError::Api(m) => write!(f, "{m}"),
        }
    }
}

type GResult<T> = std::result::Result<T, GError>;

/// One event on either side of the sync. `start`/`end` are RFC 3339 UTC, or
/// `YYYY-MM-DD` when `all_day`; `rrule` carries the RRULE body without the
/// `RRULE:` prefix, matching `event.rrule`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GEvent {
    /// Google event id; `None` while pushing a never-synced local row.
    pub id: Option<String>,
    pub title: String,
    pub start: String,
    pub end: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    pub rrule: Option<String>,
    /// Remote `updated` stamp (RFC 3339), the LWW referee.
    pub updated: String,
    /// Events listing reports deletions inline (`status: cancelled`).
    pub deleted: bool,
}

/// One task on either side of the sync.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GTask {
    pub id: Option<String>,
    pub title: String,
    pub notes: Option<String>,
    pub due: Option<String>,
    /// Google tasks carry no priority; completion is the only state.
    pub completed: bool,
    pub updated: String,
    pub deleted: bool,
}

/// One full events listing: every remaining page consumed, `next_sync_token`
/// valid for the next incremental call.
#[derive(Debug, Default)]
pub struct EventPage {
    pub events: Vec<GEvent>,
    pub next_sync_token: Option<String>,
}

/// One full tasks listing (`updatedMin` + `showDeleted`, all pages).
#[derive(Debug, Default)]
pub struct TaskPage {
    pub tasks: Vec<GTask>,
}

/// The client seam the engine drives; implemented over REST in the shell
/// (`src-tauri/src/google.rs`) and by a fake in tests.
pub trait GoogleClient {
    fn list_events(&self, sync_token: Option<&str>) -> GResult<EventPage>;
    /// Create or update; returns the event with its Google id and `updated`.
    fn upsert_event(&self, ev: &GEvent) -> GResult<GEvent>;
    fn delete_event(&self, id: &str) -> GResult<()>;
    fn list_tasks(&self, updated_min: Option<&str>) -> GResult<TaskPage>;
    fn upsert_task(&self, task: &GTask) -> GResult<GTask>;
    fn delete_task(&self, id: &str) -> GResult<()>;
}

/// A `google_link` row (v7): the OAuth join between a mail account and one
/// Google identity, plus the incremental-read watermarks.
#[derive(Debug, Clone)]
pub struct GoogleLink {
    pub mail_account_id: i64,
    pub google_email: String,
    pub calendar_id: Option<i64>,
    pub tasklist_id: Option<String>,
    pub ev_sync_token: Option<String>,
    pub task_updated_min: Option<String>,
    pub status: String,
    pub last_sync: Option<String>,
}

impl GoogleLink {
    /// Load the link row for a mail account, if any.
    ///
    /// # Errors
    /// Propagates SQL errors.
    pub fn load(conn: &Connection, mail_account_id: i64) -> Result<Option<Self>> {
        conn.query_row(
            "SELECT mail_account_id, google_email, calendar_id, tasklist_id, \
             ev_sync_token, task_updated_min, status, last_sync \
             FROM google_link WHERE mail_account_id = ?1",
            [mail_account_id],
            |r| {
                Ok(Self {
                    mail_account_id: r.get(0)?,
                    google_email: r.get(1)?,
                    calendar_id: r.get(2)?,
                    tasklist_id: r.get(3)?,
                    ev_sync_token: r.get(4)?,
                    task_updated_min: r.get(5)?,
                    status: r.get(6)?,
                    last_sync: r.get(7)?,
                })
            },
        )
        .optional()
        .map_err(Error::from)
    }

    fn save_watermarks(&self, conn: &Connection, last_sync: &str) -> Result<()> {
        conn.execute(
            "UPDATE google_link SET last_sync = ?2, last_error = NULL \
             WHERE mail_account_id = ?1",
            params![self.mail_account_id, last_sync],
        )?;
        Ok(())
    }

    fn save_error(&self, conn: &Connection, message: &str) {
        let _ = conn.execute(
            "UPDATE google_link SET last_error = ?2 WHERE mail_account_id = ?1",
            params![self.mail_account_id, message],
        );
    }
}

/// What one `sync_account` run did.
#[derive(Debug, Default, PartialEq)]
pub struct SyncReport {
    pub pushed: usize,
    pub pulled: usize,
    pub deleted: usize,
    /// Remote changes older than the local copy, left alone.
    pub conflicts: usize,
    /// True when the events token had expired and a full resync ran.
    pub full_resync: bool,
}

/// RFC 3339 comparison that survives mixed precision (`…:00Z` vs `…:00.000Z`).
fn newer(remote: &str, local: Option<&str>) -> bool {
    let (Some(r), Some(l)) = (
        DateTime::parse_from_rfc3339(remote).ok(),
        local.and_then(|s| DateTime::parse_from_rfc3339(s).ok()),
    ) else {
        return true;
    };
    r.with_timezone(&Utc) > l.with_timezone(&Utc)
}

/// Run a full two-way sync for one linked account. On error the watermarks
/// stay untouched (the next run retries) and `last_error` records why.
///
/// # Errors
/// Propagates SQL errors; transport failures are recorded on the link and
/// returned as [`Error::Parse`].
pub fn sync_account(
    conn: &Connection,
    link: &GoogleLink,
    client: &dyn GoogleClient,
) -> Result<SyncReport> {
    let Some(calendar_id) = link.calendar_id else {
        return Err(Error::Parse {
            what: "google_link",
            message: String::from("no local calendar linked"),
        });
    };
    let watermark = crate::model::now_iso();
    let since = link.last_sync.clone().unwrap_or_default();
    let mut report = SyncReport::default();

    report.deleted += push_deletions(conn, calendar_id, link, client, &since)?;
    report.pushed = push_changes(conn, calendar_id, link, client, &since)?;
    let (pulled, conflicts) =
        pull_events(conn, calendar_id, link, client, &mut report.full_resync)?;
    report.pulled += pulled;
    report.conflicts += conflicts;
    report.pushed += push_task_changes(conn, link, client, &since)?;
    let (pulled, conflicts) = pull_tasks(conn, link, client)?;
    report.pulled += pulled;
    report.conflicts += conflicts;

    link.save_watermarks(conn, &watermark)?;
    Ok(report)
}

fn transport_err(link: &GoogleLink, conn: &Connection, e: &GError) -> Error {
    link.save_error(conn, &e.to_string());
    Error::Parse { what: "gsync", message: e.to_string() }
}

// ------------------------------------------------------------------ events

fn local_event_row(conn: &Connection, event_id: i64) -> Result<GEvent> {
    conn.query_row(
        "SELECT e.gcal_id, e.title, e.start_utc, e.end_utc, e.all_day, e.location, \
         e.description, e.rrule, COALESCE(e.sync_updated_at, i.updated_at) \
         FROM event e JOIN item i ON i.id = e.item_id WHERE e.id = ?1",
        [event_id],
        |r| {
            Ok(GEvent {
                id: r.get(0)?,
                title: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
                all_day: r.get::<_, i64>(4)? != 0,
                location: r.get(5)?,
                description: r.get(6)?,
                rrule: r.get(7)?,
                updated: r.get(8)?,
                deleted: false,
            })
        },
    )
    .map_err(Error::from)
}

fn stamp_event(conn: &Connection, event_id: i64, remote: &GEvent) -> Result<()> {
    conn.execute(
        "UPDATE event SET gcal_id = ?2, sync_state = 'clean', sync_updated_at = ?3 \
         WHERE id = ?1",
        params![event_id, remote.id, remote.updated],
    )?;
    Ok(())
}

/// Push live rows of the linked calendar that changed since `since`.
fn push_changes(
    conn: &Connection,
    calendar_id: i64,
    link: &GoogleLink,
    client: &dyn GoogleClient,
    since: &str,
) -> Result<usize> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT e.id FROM event e JOIN item i ON i.id = e.item_id \
             WHERE e.calendar_id = ?1 AND i.deleted_at IS NULL \
             AND (e.sync_state != 'clean' OR i.updated_at >= ?2)",
        )?;
        let rows = stmt
            .query_map(params![calendar_id, since], |r| r.get(0))?
            .flatten()
            .collect();
        rows
    };
    let mut pushed = 0usize;
    for id in ids {
        let ev = local_event_row(conn, id)?;
        // PUT semantics: a stale gcal_id from a failed earlier push is fine;
        // a fresh row (id None) is a create.
        let remote = client.upsert_event(&ev).map_err(|e| transport_err(link, conn, &e))?;
        let stamped = GEvent { id: remote.id.clone(), updated: remote.updated, ..ev };
        stamp_event(conn, id, &stamped)?;
        pushed += 1;
    }
    Ok(pushed)
}

/// Push local tombstones of the linked calendar (trashed after `since` and
/// already carrying a remote id) to Google, then purge locally. Rows trashed
/// without a remote id never synced — nothing to propagate.
fn push_deletions(
    conn: &Connection,
    calendar_id: i64,
    link: &GoogleLink,
    client: &dyn GoogleClient,
    since: &str,
) -> Result<usize> {
    let ids: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT e.item_id, e.gcal_id FROM event e JOIN item i ON i.id = e.item_id \
             WHERE e.calendar_id = ?1 AND i.deleted_at IS NOT NULL AND i.updated_at >= ?2 \
             AND e.gcal_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![calendar_id, since], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .flatten()
            .collect();
        rows
    };
    let mut deleted = 0usize;
    for (item_id, remote_id) in ids {
        client.delete_event(&remote_id).map_err(|e| transport_err(link, conn, &e))?;
        crate::repo::purge_item(conn, item_id)?;
        deleted += 1;
    }
    Ok(deleted)
}

/// Pull remote events incrementally; on 410 retry once without a token.
fn pull_events(
    conn: &Connection,
    calendar_id: i64,
    link: &GoogleLink,
    client: &dyn GoogleClient,
    full_resync: &mut bool,
) -> Result<(usize, usize)> {
    let page = match client.list_events(link.ev_sync_token.as_deref()) {
        Ok(p) => p,
        Err(GError::Gone) => {
            *full_resync = true;
            client.list_events(None).map_err(|e| transport_err(link, conn, &e))?
        }
        Err(e) => return Err(transport_err(link, conn, &e)),
    };
    let mut pulled = 0usize;
    let mut conflicts = 0usize;
    for remote in &page.events {
        let Some(gid) = remote.id.as_deref() else { continue };
        if remote.deleted {
            pulled += delete_local_event(conn, calendar_id, gid);
            continue;
        }
        let existing: Option<(i64, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT e.id, e.uid, COALESCE(e.sync_updated_at, i.updated_at) \
                 FROM event e JOIN item i ON i.id = e.item_id \
                 WHERE e.gcal_id = ?1 AND i.deleted_at IS NULL",
                [gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((id, uid, local_updated)) = existing else {
            let uid = Some(format!("gcal-{gid}"));
            let event_id =
                crate::events::upsert_event(conn, calendar_id, &record_from(remote, uid))?;
            stamp_event(conn, event_id, remote)?;
            pulled += 1;
            continue;
        };
        if !newer(&remote.updated, local_updated.as_deref()) {
            conflicts += 1; // local copy is at least as new — LWW keeps it
            continue;
        }
        crate::events::upsert_event(conn, calendar_id, &record_from(remote, uid))?;
        stamp_event(conn, id, remote)?;
        pulled += 1;
    }
    if let Some(token) = &page.next_sync_token {
        conn.execute(
            "UPDATE google_link SET ev_sync_token = ?2 WHERE mail_account_id = ?1",
            params![link.mail_account_id, token],
        )?;
    }
    Ok((pulled, conflicts))
}

fn delete_local_event(conn: &Connection, calendar_id: i64, gid: &str) -> usize {
    let item_id: Option<i64> = conn
        .query_row(
            "SELECT e.item_id FROM event e JOIN item i ON i.id = e.item_id \
             WHERE e.gcal_id = ?1 AND e.calendar_id = ?2 AND i.deleted_at IS NULL",
            params![gid, calendar_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    match item_id {
        Some(id) if crate::repo::purge_item(conn, id).is_ok() => 1,
        _ => 0,
    }
}

fn record_from(remote: &GEvent, uid: Option<String>) -> crate::ical::EventRecord {
    crate::ical::EventRecord {
        uid,
        title: remote.title.clone(),
        start: remote.start.clone(),
        end: remote.end.clone(),
        all_day: remote.all_day,
        location: remote.location.clone(),
        description: remote.description.clone(),
        rrule: remote.rrule.clone(),
        categories: Vec::new(),
        private: false,
    }
}

// ------------------------------------------------------------------- tasks

fn local_task_row(conn: &Connection, task_id: i64) -> Result<GTask> {
    conn.query_row(
        "SELECT t.gtask_id, t.title, t.notes, t.due_utc, t.completed_at IS NOT NULL, \
         COALESCE(t.sync_updated_at, i.updated_at) \
         FROM task t JOIN item i ON i.id = t.item_id WHERE t.id = ?1",
        [task_id],
        |r| {
            Ok(GTask {
                id: r.get(0)?,
                title: r.get(1)?,
                notes: r.get(2)?,
                due: r.get(3)?,
                completed: r.get::<_, i64>(4)? != 0,
                updated: r.get(5)?,
                deleted: false,
            })
        },
    )
    .map_err(Error::from)
}

fn stamp_task(conn: &Connection, task_id: i64, remote: &GTask) -> Result<()> {
    conn.execute(
        "UPDATE task SET gtask_id = ?2, sync_state = 'clean', sync_updated_at = ?3 \
         WHERE id = ?1",
        params![task_id, remote.id, remote.updated],
    )?;
    Ok(())
}

/// Push local task tombstones, then live rows. The first-ever run (`since`
/// empty) only pushes already-linked rows — never a mass upload of the local
/// task DB; propagation of new local tasks starts with the second sync.
fn push_task_changes(
    conn: &Connection,
    link: &GoogleLink,
    client: &dyn GoogleClient,
    since: &str,
) -> Result<usize> {
    let tombstones: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT t.item_id, t.gtask_id FROM task t JOIN item i ON i.id = t.item_id \
             WHERE t.gtask_id IS NOT NULL AND i.deleted_at IS NOT NULL AND i.updated_at >= ?1",
        )?;
        let rows = stmt
            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?
            .flatten()
            .collect();
        rows
    };
    let mut count = 0usize;
    for (item_id, remote_id) in tombstones {
        client.delete_task(&remote_id).map_err(|e| transport_err(link, conn, &e))?;
        crate::repo::purge_item(conn, item_id)?;
        count += 1;
    }

    let live: Vec<i64> = if since.is_empty() {
        Vec::new()
    } else {
        let mut stmt = conn.prepare(
            "SELECT t.id FROM task t JOIN item i ON i.id = t.item_id \
             WHERE i.deleted_at IS NULL \
             AND (t.sync_state != 'clean' OR i.updated_at >= ?1)",
        )?;
        let rows = stmt.query_map([since], |r| r.get(0))?.flatten().collect();
        rows
    };
    for id in live {
        let gt = local_task_row(conn, id)?;
        let remote = client.upsert_task(&gt).map_err(|e| transport_err(link, conn, &e))?;
        let stamped = GTask { id: remote.id.clone(), updated: remote.updated, ..gt };
        stamp_task(conn, id, &stamped)?;
        count += 1;
    }
    Ok(count)
}

fn pull_tasks(
    conn: &Connection,
    link: &GoogleLink,
    client: &dyn GoogleClient,
) -> Result<(usize, usize)> {
    let page = client
        .list_tasks(link.task_updated_min.as_deref())
        .map_err(|e| transport_err(link, conn, &e))?;
    let mut pulled = 0usize;
    let mut conflicts = 0usize;
    for remote in &page.tasks {
        let Some(gid) = remote.id.as_deref() else { continue };
        if remote.deleted {
            pulled += delete_local_task(conn, gid);
            continue;
        }
        let existing: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT t.id, COALESCE(t.sync_updated_at, i.updated_at) \
                 FROM task t JOIN item i ON i.id = t.item_id \
                 WHERE t.gtask_id = ?1 AND i.deleted_at IS NULL",
                [gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, local_updated)) = existing else {
            let task_id = crate::tasks::save_task(
                conn,
                &crate::tasks::TaskInput {
                    id: None,
                    title: remote.title.clone(),
                    notes: remote.notes.clone(),
                    due_utc: remote.due.clone(),
                    priority: 0,
                    percent: i64::from(remote.completed) * 100,
                },
            )?;
            stamp_task(conn, task_id, remote)?;
            pulled += 1;
            continue;
        };
        if !newer(&remote.updated, local_updated.as_deref()) {
            conflicts += 1;
            continue;
        }
        apply_task_fields(conn, id, remote)?;
        stamp_task(conn, id, remote)?;
        pulled += 1;
    }
    conn.execute(
        "UPDATE google_link SET task_updated_min = ?2 WHERE mail_account_id = ?1",
        params![link.mail_account_id, crate::model::now_iso()],
    )?;
    Ok((pulled, conflicts))
}

fn delete_local_task(conn: &Connection, gid: &str) -> usize {
    let item_id: Option<i64> = conn
        .query_row(
            "SELECT t.item_id FROM task t JOIN item i ON i.id = t.item_id \
             WHERE t.gtask_id = ?1 AND i.deleted_at IS NULL",
            [gid],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    match item_id {
        Some(id) if crate::repo::purge_item(conn, id).is_ok() => 1,
        _ => 0,
    }
}

/// Overwrite the Google-owned fields of a linked task; priority stays local,
/// and an in-progress local percent survives a Google un-completion.
fn apply_task_fields(conn: &Connection, task_id: i64, remote: &GTask) -> Result<()> {
    let percent = if remote.completed {
        100
    } else {
        conn.query_row("SELECT percent FROM task WHERE id = ?1", [task_id], |r| {
            r.get::<_, i64>(0).map(|p| p.min(99))
        })?
    };
    conn.execute(
        "UPDATE task SET title = ?2, notes = ?3, due_utc = ?4, percent = ?5, \
         completed_at = CASE WHEN ?5 >= 100 THEN COALESCE(completed_at, ?6) ELSE NULL END \
         WHERE id = ?1",
        params![task_id, remote.title, remote.notes, remote.due, percent, remote.updated],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::cell::RefCell;

    /// In-memory fake: the "remote" is a pair of vectors the tests seed and
    /// inspect. `fail_gone_once` makes the next token-bearing events listing
    /// return 410; push results are stamped `push_updated` (the fake's "now").
    /// Shared id mint so pushed and seeded rows can never collide.
    static ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

    struct FakeGoogle {
        /// The remote store: what a full listing (`list_events(None)`) and
        /// `list_tasks` return.
        events: RefCell<Vec<GEvent>>,
        tasks: RefCell<Vec<GTask>>,
        /// One-shot change feed drained by the next *incremental* listing,
        /// like Google's syncToken deltas.
        pending: RefCell<Vec<GEvent>>,
        fail_gone_once: RefCell<bool>,
        push_updated: RefCell<String>,
    }

    impl FakeGoogle {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                tasks: RefCell::new(Vec::new()),
                pending: RefCell::new(Vec::new()),
                fail_gone_once: RefCell::new(false),
                push_updated: RefCell::new(String::from("2026-10-21T11:00:00Z")),
            }
        }

        /// Add an event remotely and report it as an incremental change.
        fn seed_event(&self, mut ev: GEvent) {
            let n = ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let id = ev.id.clone().unwrap_or_else(|| format!("r{n}"));
            if ev.updated.is_empty() {
                ev.updated = String::from("2026-10-21T10:00:00Z");
            }
            ev.id = Some(id.clone());
            self.events.borrow_mut().push(ev.clone());
            self.pending.borrow_mut().push(ev);
            let _ = n;
        }
    }

    impl GoogleClient for FakeGoogle {
        fn list_events(&self, sync_token: Option<&str>) -> GResult<EventPage> {
            if *self.fail_gone_once.borrow() && sync_token.is_some() {
                *self.fail_gone_once.borrow_mut() = false;
                return Err(GError::Gone);
            }
            let events = match sync_token {
                // Incremental: only the queued deltas, once.
                Some(_) => std::mem::take(&mut *self.pending.borrow_mut()),
                // Full listing: everything the store holds.
                None => self.events.borrow().clone(),
            };
            Ok(EventPage { events, next_sync_token: Some(String::from("tok")) })
        }

        fn upsert_event(&self, ev: &GEvent) -> GResult<GEvent> {
            let mut out = ev.clone();
            let n = ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let id = out.id.clone().unwrap_or_else(|| format!("r{n}"));
            out.id = Some(id);
            out.updated = self.push_updated.borrow().clone();
            let events = &mut *self.events.borrow_mut();
            match events.iter_mut().find(|e| e.id == out.id) {
                Some(e) => *e = out.clone(),
                None => events.push(out.clone()),
            }
            Ok(out)
        }

        fn delete_event(&self, id: &str) -> GResult<()> {
            self.events.borrow_mut().retain(|e| e.id.as_deref() != Some(id));
            Ok(())
        }

        fn list_tasks(&self, _updated_min: Option<&str>) -> GResult<TaskPage> {
            Ok(TaskPage { tasks: self.tasks.borrow().clone() })
        }

        fn upsert_task(&self, t: &GTask) -> GResult<GTask> {
            let mut out = t.clone();
            let id =
                out.id.clone().unwrap_or_else(|| format!("t{}", self.tasks.borrow().len() + 1));
            out.id = Some(id);
            out.updated = self.push_updated.borrow().clone();
            let tasks = &mut *self.tasks.borrow_mut();
            match tasks.iter_mut().find(|t| t.id == out.id) {
                Some(t) => *t = out.clone(),
                None => tasks.push(out.clone()),
            }
            Ok(out)
        }

        fn delete_task(&self, id: &str) -> GResult<()> {
            self.tasks.borrow_mut().retain(|t| t.id.as_deref() != Some(id));
            Ok(())
        }
    }

    struct Fixture {
        store: Store,
        calendar_id: i64,
        link: GoogleLink,
    }

    fn fixture() -> Fixture {
        let store = Store::open_memory("gsync").unwrap();
        let conn = store.conn();
        // google_link references mail_account.
        conn.execute(
            "INSERT INTO mail_account (kind, name, user, host) \
             VALUES ('imap', 'Gmail', 'me@gmail.com', 'imap.gmail.com')",
            [],
        )
        .unwrap();
        let account: i64 = conn.last_insert_rowid();
        let calendar_id = crate::events::ensure_calendar(conn, "Google", Some("#1a73e8")).unwrap();
        conn.execute(
            "INSERT INTO google_link (mail_account_id, google_email, calendar_id, tasklist_id) \
             VALUES (?1, 'me@gmail.com', ?2, 'MDF-tasklist')",
            rusqlite::params![account, calendar_id],
        )
        .unwrap();
        let link = GoogleLink::load(conn, account).unwrap().unwrap();
        Fixture { store, calendar_id, link }
    }

    fn local_event(f: &Fixture, title: &str) -> i64 {
        crate::calrepo::save_event(
            f.store.conn(),
            &crate::calrepo::EventInput {
                id: None,
                calendar_id: f.calendar_id,
                title: title.into(),
                start: "2026-10-21T10:00:00+00:00".into(),
                end: Some("2026-10-21T11:00:00+00:00".into()),
                all_day: false,
                location: None,
                description: None,
                rrule: None,
                reminder_minutes: None,
                participants: Vec::new(),
            },
        )
        .unwrap()
    }

    fn local_task(f: &Fixture, title: &str) -> i64 {
        crate::tasks::save_task(
            f.store.conn(),
            &crate::tasks::TaskInput {
                id: None,
                title: title.into(),
                notes: None,
                due_utc: None,
                priority: 0,
                percent: 0,
            },
        )
        .unwrap()
    }

    fn event_col(f: &Fixture, id: i64, col: &str) -> String {
        f.store
            .conn()
            .query_row(
                &format!("SELECT {col} FROM event WHERE id = ?1"),
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Run a sync with a freshly loaded link — like the shell does, one load
    /// per run. (The in-memory `Fixture::link` never sees saved watermarks.)
    fn sync(
        f: &Fixture,
        client: &dyn GoogleClient,
    ) -> std::result::Result<SyncReport, crate::error::Error> {
        let link = GoogleLink::load(f.store.conn(), f.link.mail_account_id)?.unwrap();
        sync_account(f.store.conn(), &link, client)
    }

    #[test]
    fn push_stamps_remote_ids_and_next_run_is_quiet() {
        let f = fixture();
        let id = local_event(&f, "From local");
        // Strictly older than the first watermark, so run 2's `>=` scan
        // (at-least-once under millisecond resolution) has nothing to redo.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let fake = FakeGoogle::new();
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.pushed, 1);
        assert_eq!(event_col(&f, id, "sync_state"), "clean");
        assert!(!event_col(&f, id, "gcal_id").is_empty());
        // Nothing changed since: the next run pushes nothing (pull is quiet).
        let again = sync(&f, &fake).unwrap();
        assert_eq!(again.pushed, 0);
        assert_eq!(again.pulled, 0);
    }

    #[test]
    fn pull_creates_and_lww_keeps_newer_local() {
        let f = fixture();
        let id = local_event(&f, "Local edit");
        let fake = FakeGoogle::new();
        fake.seed_event(GEvent {
            title: "New from Google".into(),
            start: "2026-10-22T09:00:00Z".into(),
            updated: "2026-10-22T08:00:00Z".into(),
            ..GEvent::default()
        });
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.pulled, 1);
        let conn = f.store.conn();
        let title: String = conn
            .query_row("SELECT title FROM event WHERE uid LIKE 'gcal-%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "New from Google");

        // Remote echoes the local row with an older stamp ⇒ skipped, counted.
        // Drain the queue first: the seeded delta was consumed by the full
        // listing above (no token yet), not the incremental one.
        fake.list_events(Some("tok")).unwrap();
        let gid = event_col(&f, id, "gcal_id");
        fake.seed_event(GEvent {
            id: Some(gid),
            title: "Stale remote".into(),
            start: "2026-10-21T10:00:00Z".into(),
            updated: "2026-01-01T00:00:00Z".into(),
            ..GEvent::default()
        });
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(event_col(&f, id, "title"), "Local edit");
    }

    #[test]
    fn remote_deletion_purges_local_row() {
        let f = fixture();
        let fake = FakeGoogle::new();
        let id = local_event(&f, "Gone remote");
        sync(&f, &fake).unwrap();
        let gid = event_col(&f, id, "gcal_id");
        // Google reports cancellations inline; the local row must go.
        fake.seed_event(GEvent {
            id: Some(gid),
            title: "Gone remote".into(),
            start: "2026-10-21T10:00:00Z".into(),
            updated: "2026-12-31T00:00:00Z".into(),
            deleted: true,
            ..GEvent::default()
        });
        sync(&f, &fake).unwrap();
        let left: i64 = f
            .store
            .conn()
            .query_row("SELECT count(*) FROM event WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn local_deletion_pushes_and_purges() {
        let f = fixture();
        let fake = FakeGoogle::new();
        let id = local_event(&f, "Gone local");
        sync(&f, &fake).unwrap();
        let gid = event_col(&f, id, "gcal_id");
        // trash_item stamps `updated_at` from the real clock, i.e. after the
        // first run's watermark — the tombstone qualifies for the push.
        crate::calrepo::delete_event(f.store.conn(), id).unwrap();
        let dbg: (Option<String>, Option<String>, Option<String>) = f.store.conn().query_row(
            "SELECT i.deleted_at, i.updated_at, (SELECT last_sync FROM google_link) FROM item i JOIN event e ON e.item_id = i.id WHERE e.id = ?1", [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        eprintln!("[dbg] tomb={dbg:?} gid={gid:?}");
        let report = sync(&f, &fake).unwrap();
        eprintln!("[dbg] report={report:?}");
        assert_eq!(report.deleted, 1);
        assert!(fake
            .events
            .borrow()
            .iter()
            .all(|e| e.id.as_deref() != Some(gid.as_str())));
    }

    #[test]
    fn expired_token_triggers_full_resync() {
        let f = fixture();
        let fake = FakeGoogle::new();
        // First run establishes the incremental token.
        sync(&f, &fake).unwrap();
        assert_eq!(
            f.store
                .conn()
                .query_row::<String, _, _>("SELECT ev_sync_token FROM google_link", [], |r| r
                    .get(0))
                .unwrap(),
            "tok"
        );
        // Token expired server-side; the queued change only arrives via a
        // full listing.
        *fake.fail_gone_once.borrow_mut() = true;
        fake.seed_event(GEvent {
            title: "Resynced".into(),
            start: "2026-10-23T09:00:00Z".into(),
            updated: "2026-10-23T08:00:00Z".into(),
            ..GEvent::default()
        });
        let report = sync(&f, &fake).unwrap();
        assert!(report.full_resync);
        assert_eq!(report.pulled, 1);
    }

    #[test]
    fn tasks_pull_then_push_then_pull_lww() {
        let f = fixture();
        let fake = FakeGoogle::new();

        // First run on a fresh link: pull-only — no mass upload of the local
        // task DB.
        fake.tasks.borrow_mut().push(GTask {
            id: Some(String::from("t1")),
            title: "From Google".into(),
            due: Some("2026-10-25T17:00:00Z".into()),
            updated: "2026-10-21T09:00:00Z".into(),
            ..GTask::default()
        });
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.pulled, 1);
        assert_eq!(report.pushed, 0);
        let (task_id, gid): (i64, Option<String>) = f
            .store
            .conn()
            .query_row(
                "SELECT id, gtask_id FROM task WHERE title = 'From Google'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(gid.is_some());

        // Second run: completing locally pushes as completed.
        crate::tasks::save_task(
            f.store.conn(),
            &crate::tasks::TaskInput {
                id: Some(task_id),
                title: "From Google".into(),
                notes: None,
                due_utc: None,
                priority: 7,
                percent: 100,
            },
        )
        .unwrap();
        // The edit must be strictly older than the next watermark, else the
        // at-least-once `>=` scan re-pushes it and overwrites the queued
        // remote delta (millisecond-resolution watermarks).
        std::thread::sleep(std::time::Duration::from_millis(5));
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.pushed, 1);
        let remote = fake
            .tasks
            .borrow()
            .iter()
            .find(|t| t.id == gid)
            .cloned()
            .expect("remote task exists");
        assert!(remote.completed);

        // Third run: Google un-completes with a newer stamp. Pull-side LWW
        // applies it: percent leaves the completed bucket (capped below 100),
        // completion clears, priority stays local.
        let mut undone = remote;
        undone.completed = false;
        undone.updated = String::from("2026-10-23T00:00:00Z");
        fake.tasks.borrow_mut().retain(|t| t.id != undone.id);
        fake.tasks.borrow_mut().push(undone);
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.pulled, 1);
        let (percent, completed, priority): (i64, Option<String>, i64) = f
            .store
            .conn()
            .query_row(
                "SELECT percent, completed_at, priority FROM task WHERE id = ?1",
                [task_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(percent, 99);
        assert!(completed.is_none());
        assert_eq!(priority, 7);
    }

    #[test]
    fn fresh_link_never_mass_uploads_tasks() {
        let f = fixture();
        local_task(&f, "Pre-existing 1");
        local_task(&f, "Pre-existing 2");
        let fake = FakeGoogle::new();
        let report = sync(&f, &fake).unwrap();
        assert_eq!(report.pushed, 0);
        assert!(fake.tasks.borrow().is_empty());
    }

    #[test]
    fn transport_error_records_last_error_and_keeps_watermark() {
        let f = fixture();
        let id = local_event(&f, "Doomed");
        struct Boom;
        impl GoogleClient for Boom {
            fn list_events(&self, _: Option<&str>) -> GResult<EventPage> {
                Err(GError::Api(String::from("503 down")))
            }
            fn upsert_event(&self, _: &GEvent) -> GResult<GEvent> {
                Err(GError::Api(String::from("503 down")))
            }
            fn delete_event(&self, _: &str) -> GResult<()> {
                Ok(())
            }
            fn list_tasks(&self, _: Option<&str>) -> GResult<TaskPage> {
                Ok(TaskPage::default())
            }
            fn upsert_task(&self, _: &GTask) -> GResult<GTask> {
                Ok(GTask::default())
            }
            fn delete_task(&self, _: &str) -> GResult<()> {
                Ok(())
            }
        }
        assert!(sync(&f, &Boom).is_err());
        let conn = f.store.conn();
        let (err, last_sync): (Option<String>, Option<String>) = conn
            .query_row("SELECT last_error, last_sync FROM google_link", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(err.unwrap().contains("503"));
        assert!(last_sync.is_none(), "watermark must not move on failure");
        // Local row untouched and still dirty for the next run.
        assert_eq!(event_col(&f, id, "sync_state"), "clean"); // default, timestamp-driven
    }
}
