//! Mail rules engine (T1.11 / S22): ordered condition/action sets applied to
//! incoming messages, per account or all; manual run on a folder. Conditions
//! and actions are stored as JSON in `mail_rule`.
//!
//! Condition JSON: `{"field":"from"|"to"|"subject","op":"contains"|"equals",
//! "value":"..."}`. Actions JSON: `{"type":"move","folder":"Archive"}` |
//! `{"type":"mark_read"}` | `{"type":"tag","value":"news"}`. All conditions
//! of a rule must match (AND); all actions apply in order.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub field: String, // from | to | subject
    pub op: String,    // contains | equals
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Move to a named folder (created on demand in the target account).
    Move {
        folder: String,
    },
    MarkRead,
    Tag {
        value: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    pub id: i64,
    pub account_id: Option<i64>,
    pub enabled: bool,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
    pub position: i64,
}

pub fn create_rule(
    conn: &Connection,
    account_id: Option<i64>,
    conditions: &[Condition],
    actions: &[Action],
    position: i64,
) -> Result<i64> {
    if conditions.is_empty() || actions.is_empty() {
        return Err(Error::Row {
            row: 0,
            message: "rule needs at least one condition and one action".into(),
        });
    }
    conn.execute(
        "INSERT INTO mail_rule (account_id, enabled, conditions, actions, position) \
         VALUES (?1, 1, ?2, ?3, ?4)",
        params![
            account_id,
            serde_json::to_string(conditions)?,
            serde_json::to_string(actions)?,
            position
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_rule_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<()> {
    conn.execute(
        "UPDATE mail_rule SET enabled = ?2 WHERE id = ?1",
        params![id, i64::from(enabled)],
    )?;
    Ok(())
}

pub fn delete_rule(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM mail_rule WHERE id = ?1", [id])?;
    Ok(())
}

pub fn rules_of_account(conn: &Connection, account_id: Option<i64>) -> Result<Vec<Rule>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, enabled, conditions, actions, position FROM mail_rule \
         WHERE ?1 IS NULL OR account_id IS NULL OR account_id = ?1 ORDER BY position, id",
    )?;
    let rows = stmt.query_map(params![account_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, account_id, enabled, conditions, actions, position) = row?;
        out.push(Rule {
            id,
            account_id,
            enabled: enabled != 0,
            conditions: serde_json::from_str(&conditions)?,
            actions: serde_json::from_str(&actions)?,
            position,
        });
    }
    Ok(out)
}

/// Outcome of applying rules to one message.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RuleOutcome {
    pub moved_to_folder: Option<i64>,
    pub marked_read: bool,
    pub tagged: Vec<String>,
}

fn condition_matches(cond: &Condition, subject: &str, from: &str, to: &str) -> bool {
    let hay = match cond.field.as_str() {
        "subject" => subject,
        "from" => from,
        "to" => to,
        _ => return false,
    };
    match cond.op.as_str() {
        "contains" => hay.to_lowercase().contains(&cond.value.to_lowercase()),
        "equals" => hay.eq_ignore_ascii_case(&cond.value),
        _ => false,
    }
}

fn rule_applies(rule: &Rule, subject: &str, from: &str, to: &str) -> bool {
    rule.enabled
        && rule
            .conditions
            .iter()
            .all(|c| condition_matches(c, subject, from, to))
}

/// Apply the matching rules for `account_id` to one message's fields and
/// return the combined outcome. Pure: the caller (sync loop or manual run)
/// executes the actions against the store. This is testable without network.
pub fn evaluate(
    conn: &Connection,
    account_id: i64,
    subject: &str,
    from: &str,
    to: &str,
) -> Result<RuleOutcome> {
    let mut outcome = RuleOutcome::default();
    for rule in rules_of_account(conn, Some(account_id))? {
        if !rule_applies(&rule, subject, from, to) {
            continue;
        }
        for action in &rule.actions {
            match action {
                Action::Move { folder } => {
                    outcome.moved_to_folder = Some(mkdir_or_resolve(conn, account_id, folder)?);
                }
                Action::MarkRead => outcome.marked_read = true,
                Action::Tag { value } => outcome.tagged.push(value.clone()),
            }
        }
    }
    Ok(outcome)
}

fn mkdir_or_resolve(conn: &Connection, account_id: i64, name: &str) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM mail_folder WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO mail_folder (account_id, name) VALUES (?1, ?2)",
        params![account_id, name],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Apply `evaluate`'s outcome to a stored message (shared by the sync loop
/// and the manual "run rules on folder" command).
pub fn apply_outcome(
    conn: &Connection,
    message_id: i64,
    item_id: Option<i64>,
    outcome: &RuleOutcome,
) -> Result<()> {
    if outcome.marked_read {
        conn.execute(
            "UPDATE message SET flags = json_set(flags, '$.seen', 1) WHERE id = ?1",
            [message_id],
        )?;
    }
    if let Some(folder_id) = outcome.moved_to_folder {
        conn.execute(
            "UPDATE message SET folder_id = ?2 WHERE id = ?1",
            params![message_id, folder_id],
        )?;
    }
    if let Some(item) = item_id {
        for tag in &outcome.tagged {
            crate::repo::tag_item(conn, item, tag)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ outgoing queue (S21)

/// Queue a draft for sending: immediate (`send_at = NULL`) or scheduled
/// (Send Later). Missed scheduled sends (state still queued, `send_at` in the
/// past) are picked up on next start and flagged to the sender.
pub fn queue_outgoing(conn: &Connection, draft_json: &str, send_at: Option<&str>) -> Result<i64> {
    conn.execute(
        "INSERT INTO outbox (draft_json, send_at, state) VALUES (?1, ?2, 'queued')",
        params![draft_json, send_at],
    )?;
    Ok(conn.last_insert_rowid())
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingEntry {
    pub id: i64,
    pub draft_json: String,
    pub send_at: Option<String>,
    /// True when `send_at` passed while the app was closed (S21: fires on
    /// next start, flagged).
    pub missed: bool,
}

/// Pop due outgoing messages (oldest first), marking them `sending`.
pub fn take_due_outgoing(conn: &Connection, now: &str, limit: i64) -> Result<Vec<OutgoingEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, draft_json, send_at FROM outbox \
         WHERE state = 'queued' AND (send_at IS NULL OR send_at <= ?1) \
         ORDER BY send_at IS NOT NULL, send_at, id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![now, limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, draft_json, send_at) = row?;
        let missed = send_at.as_deref().is_some_and(|at| at < now);
        conn.execute("UPDATE outbox SET state = 'sending' WHERE id = ?1", [id])?;
        out.push(OutgoingEntry {
            id,
            draft_json,
            send_at,
            missed,
        });
    }
    Ok(out)
}

pub fn mark_sent(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("UPDATE outbox SET state = 'sent' WHERE id = ?1", [id])?;
    Ok(())
}

pub fn mark_failed(conn: &Connection, id: i64, error: &str) -> Result<()> {
    // Back to queued for the next attempt; the error is surfaced in the UI.
    conn.execute(
        "UPDATE outbox SET state = 'queued', error = ?2 WHERE id = ?1",
        params![id, error],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailstore::{self, MessageInput};
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    fn acct(conn: &Connection) -> (i64, i64) {
        let a = mailstore::create_account(conn, "Work", "imap", None, None, None).unwrap();
        let inbox = mailstore::ensure_folder(conn, a, None, "INBOX", Some("inbox")).unwrap();
        (a, inbox)
    }

    #[test]
    fn rule_with_move_markread_and_tag() {
        let s = store();
        let conn = s.conn();
        let (a, inbox) = acct(conn);
        create_rule(
            conn,
            Some(a),
            &[Condition {
                field: "from".into(),
                op: "contains".into(),
                value: "news@x.io".into(),
            }],
            &[
                Action::Move {
                    folder: "Newsletters".into(),
                },
                Action::MarkRead,
                Action::Tag {
                    value: "news".into(),
                },
            ],
            0,
        )
        .unwrap();

        let msg_id = mailstore::upsert_message(
            conn,
            a,
            inbox,
            &MessageInput {
                uid: 1,
                subject: "Weekly digest".into(),
                from_name: "News".into(),
                from_addr: "news@x.io".into(),
                to_addrs: "me@x.io".into(),
                date: "2026-01-01T00:00:00+00:00".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let item: i64 = conn
            .query_row("SELECT item_id FROM message WHERE id = ?1", [msg_id], |r| {
                r.get(0)
            })
            .unwrap();

        let outcome = evaluate(conn, a, "Weekly digest", "news@x.io", "me@x.io").unwrap();
        apply_outcome(conn, msg_id, Some(item), &outcome).unwrap();

        let (folder, flags): (String, String) = conn
            .query_row(
                "SELECT f.name, m.flags FROM message m JOIN mail_folder f ON f.id = m.folder_id \
                 WHERE m.id = ?1",
                [msg_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(folder, "Newsletters");
        assert!(flags.contains("\"seen\":1"), "{flags}");
        assert_eq!(crate::repo::tags_of_item(conn, item).len(), 1);
    }

    #[test]
    fn no_match_noop_and_case_insensitive_contains() {
        let s = store();
        let conn = s.conn();
        let (a, _inbox) = acct(conn);
        create_rule(
            conn,
            None, // applies to all accounts
            &[Condition {
                field: "subject".into(),
                op: "contains".into(),
                value: "INVOICE".into(),
            }],
            &[Action::MarkRead],
            0,
        )
        .unwrap();
        let hit = evaluate(conn, a, "Your invoice #42", "billing@x.io", "me@x.io").unwrap();
        assert!(hit.marked_read, "contains is case-insensitive");
        let miss = evaluate(conn, a, "Hello", "billing@x.io", "me@x.io").unwrap();
        assert!(!miss.marked_read);
    }

    #[test]
    fn empty_rule_is_rejected() {
        let s = store();
        let conn = s.conn();
        assert!(create_rule(conn, None, &[], &[Action::MarkRead], 0).is_err());
    }

    #[test]
    fn outbox_lifecycle_and_missed_flag() {
        let s = store();
        let conn = s.conn();
        let immediate = queue_outgoing(conn, "{\"subject\":\"hi\"}", None).unwrap();
        let scheduled = queue_outgoing(
            conn,
            "{\"subject\":\"later\"}",
            Some("2026-01-01T09:00:00+00:00"),
        )
        .unwrap();

        // "Now" is after the scheduled send: both due, scheduled one flagged missed.
        let due = take_due_outgoing(conn, "2026-01-01T10:00:00+00:00", 10).unwrap();
        assert_eq!(due.len(), 2);
        assert!(!due[0].missed, "immediate first");
        assert!(due[1].missed);

        mark_failed(conn, immediate, "smtp down").unwrap();
        let retry = take_due_outgoing(conn, "2026-01-01T10:00:00+00:00", 10).unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].id, immediate);

        mark_sent(conn, immediate).unwrap();
        assert!(take_due_outgoing(conn, "2026-01-01T10:00:00+00:00", 10)
            .unwrap()
            .is_empty());
        let _ = scheduled;
    }

    #[test]
    fn future_send_stays_queued() {
        let s = store();
        let conn = s.conn();
        queue_outgoing(conn, "{}", Some("2026-01-01T23:00:00+00:00")).unwrap();
        assert!(take_due_outgoing(conn, "2026-01-01T10:00:00+00:00", 10)
            .unwrap()
            .is_empty());
    }
}
