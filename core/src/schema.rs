//! Embedded schema migrations. Forward-only; `PRAGMA user_version` tracks the
//! applied version. Each migration runs inside one transaction.

use rusqlite::Connection;

use crate::error::{Error, Result};

/// v1 — initial schema (`docs/data-model.md`). External-content FTS5 tables for
/// mail, tasks and notes are kept in sync by triggers.
pub const V1: &str = r"
CREATE TABLE item_list (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  parent_id INTEGER REFERENCES item_list(id),
  name TEXT NOT NULL,
  color TEXT,
  icon TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  settings TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX idx_item_list_kind ON item_list(kind, parent_id);

CREATE TABLE item (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('event','task','note','contact','password','message')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT,
  private INTEGER NOT NULL DEFAULT 0,
  list_id INTEGER REFERENCES item_list(id),
  sort_key TEXT
);
CREATE INDEX idx_item_kind_list ON item(kind, list_id) WHERE deleted_at IS NULL;

CREATE TABLE tag (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  color TEXT
);

CREATE TABLE item_tag (
  item_id INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
  tag_id INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
  PRIMARY KEY (item_id, tag_id)
);
CREATE INDEX idx_item_tag_tag ON item_tag(tag_id);

CREATE TABLE item_link (
  from_id INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
  to_id INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
  PRIMARY KEY (from_id, to_id)
);
CREATE INDEX idx_item_link_to ON item_link(to_id);
CREATE TRIGGER item_link_sym AFTER INSERT ON item_link
BEGIN
  INSERT OR IGNORE INTO item_link(from_id, to_id) VALUES (NEW.to_id, NEW.from_id);
END;

CREATE TABLE item_version (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
  saved_at TEXT NOT NULL,
  snapshot TEXT NOT NULL
);
CREATE INDEX idx_item_version_item ON item_version(item_id, saved_at DESC);

CREATE TABLE attachment (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL REFERENCES item(id) ON DELETE CASCADE,
  sha256 TEXT NOT NULL,
  filename TEXT NOT NULL,
  size INTEGER NOT NULL,
  mime TEXT
);
CREATE UNIQUE INDEX idx_attachment_sha ON attachment(sha256);

CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY,
  at TEXT NOT NULL,
  item_id INTEGER,
  action TEXT NOT NULL,
  detail TEXT
);
CREATE INDEX idx_audit_at ON audit_log(at DESC);

CREATE TABLE undo_op (
  id INTEGER PRIMARY KEY,
  at TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL
);

CREATE TABLE reminder_queue (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL,
  due_at TEXT NOT NULL,
  fired_at TEXT,
  method TEXT NOT NULL DEFAULT 'popup',
  snoozed_to TEXT
);
CREATE INDEX idx_reminder_due ON reminder_queue(due_at) WHERE fired_at IS NULL;

CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE category (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  color TEXT,
  holiday INTEGER NOT NULL DEFAULT 0,
  kind TEXT
);

-- mail -----------------------------------------------------------------------
CREATE TABLE mail_account (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('imap','pop3')),
  host TEXT,
  port INTEGER,
  security TEXT,
  user TEXT,
  color TEXT,
  identity_name TEXT,
  reply_to TEXT,
  oauth INTEGER NOT NULL DEFAULT 0,
  settings TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE mail_folder (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES mail_account(id) ON DELETE CASCADE,
  parent_id INTEGER REFERENCES mail_folder(id),
  name TEXT NOT NULL,
  role TEXT,
  uidvalidity INTEGER,
  last_seen_uid INTEGER NOT NULL DEFAULT 0,
  sync_state TEXT
);
CREATE INDEX idx_mail_folder_acct ON mail_folder(account_id);

CREATE TABLE thread (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  subject TEXT,
  last_date TEXT,
  pinned INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_thread_acct ON thread(account_id, last_date DESC);
CREATE INDEX idx_thread_subject ON thread(account_id, subject);

CREATE TABLE message (
  id INTEGER PRIMARY KEY,
  item_id INTEGER REFERENCES item(id),
  account_id INTEGER NOT NULL REFERENCES mail_account(id) ON DELETE CASCADE,
  folder_id INTEGER NOT NULL REFERENCES mail_folder(id) ON DELETE CASCADE,
  uid INTEGER NOT NULL,
  message_id TEXT,
  in_reply_to TEXT,
  thread_id INTEGER REFERENCES thread(id),
  subject TEXT NOT NULL DEFAULT '',
  from_name TEXT NOT NULL DEFAULT '',
  from_addr TEXT NOT NULL DEFAULT '',
  to_addrs TEXT NOT NULL DEFAULT '',
  cc_addrs TEXT NOT NULL DEFAULT '',
  date TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  flags TEXT NOT NULL DEFAULT '{}',
  category TEXT,
  private INTEGER NOT NULL DEFAULT 0,
  body TEXT NOT NULL DEFAULT '',
  snippet TEXT NOT NULL DEFAULT '',
  html_path TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_msg_folder_date ON message(folder_id, date DESC, id DESC);
CREATE INDEX idx_msg_thread ON message(thread_id);
CREATE UNIQUE INDEX idx_msg_folder_uid ON message(folder_id, uid);

CREATE VIRTUAL TABLE message_fts USING fts5(
  subject, from_name, from_addr, to_addrs, body,
  content='message', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER message_fts_ai AFTER INSERT ON message BEGIN
  INSERT INTO message_fts(rowid, subject, from_name, from_addr, to_addrs, body)
  VALUES (new.id, new.subject, new.from_name, new.from_addr, new.to_addrs, new.body);
END;
CREATE TRIGGER message_fts_ad AFTER DELETE ON message BEGIN
  INSERT INTO message_fts(message_fts, rowid, subject, from_name, from_addr, to_addrs, body)
  VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.to_addrs, old.body);
END;
CREATE TRIGGER message_fts_au AFTER UPDATE ON message BEGIN
  INSERT INTO message_fts(message_fts, rowid, subject, from_name, from_addr, to_addrs, body)
  VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.to_addrs, old.body);
  INSERT INTO message_fts(rowid, subject, from_name, from_addr, to_addrs, body)
  VALUES (new.id, new.subject, new.from_name, new.from_addr, new.to_addrs, new.body);
END;

CREATE TABLE mail_rule (
  id INTEGER PRIMARY KEY,
  account_id INTEGER REFERENCES mail_account(id) ON DELETE CASCADE,
  enabled INTEGER NOT NULL DEFAULT 1,
  conditions TEXT NOT NULL,
  actions TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE identity (
  id INTEGER PRIMARY KEY,
  account_id INTEGER REFERENCES mail_account(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  email TEXT NOT NULL,
  signature_html TEXT,
  is_default INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE template (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('mail','event','task','note')),
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE outbox (
  id INTEGER PRIMARY KEY,
  draft_json TEXT NOT NULL,
  send_at TEXT,
  state TEXT NOT NULL DEFAULT 'queued',
  error TEXT
);

-- calendar -------------------------------------------------------------------
CREATE TABLE calendar (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  color TEXT,
  kind TEXT NOT NULL DEFAULT 'local',
  url TEXT,
  sync_token TEXT,
  visible INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE event (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  calendar_id INTEGER REFERENCES calendar(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  start_utc TEXT NOT NULL,
  end_utc TEXT,
  all_day INTEGER NOT NULL DEFAULT 0,
  location TEXT,
  completion INTEGER NOT NULL DEFAULT 0,
  description TEXT,
  rrule TEXT,
  rdate TEXT,
  exdate TEXT,
  category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
  uid TEXT
);
CREATE INDEX idx_event_start ON event(start_utc);

CREATE TABLE event_reminder (
  id INTEGER PRIMARY KEY,
  event_id INTEGER NOT NULL REFERENCES event(id) ON DELETE CASCADE,
  minutes_before INTEGER NOT NULL,
  method TEXT NOT NULL DEFAULT 'popup',
  sound TEXT
);

CREATE TABLE event_participant (
  event_id INTEGER NOT NULL REFERENCES event(id) ON DELETE CASCADE,
  contact_id INTEGER,
  email TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  PRIMARY KEY (event_id, email)
);

CREATE TABLE subscription (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  calendar_id INTEGER REFERENCES calendar(id) ON DELETE CASCADE,
  interval_min INTEGER NOT NULL DEFAULT 60,
  last_fetched TEXT
);

-- tasks ----------------------------------------------------------------------
CREATE TABLE custom_field (
  id INTEGER PRIMARY KEY,
  kind TEXT,
  name TEXT NOT NULL,
  type TEXT NOT NULL CHECK(type IN ('text','date','dropdown','yesno','checkbox')),
  options TEXT
);

CREATE TABLE task (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  notes TEXT,
  start_utc TEXT,
  due_utc TEXT,
  priority INTEGER NOT NULL DEFAULT 0,
  percent INTEGER NOT NULL DEFAULT 0,
  completed_at TEXT,
  rrule TEXT,
  category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
  floating INTEGER NOT NULL DEFAULT 0,
  assigned_contact_id INTEGER,
  custom TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX idx_task_due ON task(due_utc) WHERE completed_at IS NULL;

CREATE VIRTUAL TABLE task_fts USING fts5(
  title, notes,
  content='task', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER task_fts_ai AFTER INSERT ON task BEGIN
  INSERT INTO task_fts(rowid, title, notes) VALUES (new.id, new.title, new.notes);
END;
CREATE TRIGGER task_fts_ad AFTER DELETE ON task BEGIN
  INSERT INTO task_fts(task_fts, rowid, title, notes) VALUES ('delete', old.id, old.title, old.notes);
END;
CREATE TRIGGER task_fts_au AFTER UPDATE ON task BEGIN
  INSERT INTO task_fts(task_fts, rowid, title, notes) VALUES ('delete', old.id, old.title, old.notes);
  INSERT INTO task_fts(rowid, title, notes) VALUES (new.id, new.title, new.notes);
END;

-- notes ----------------------------------------------------------------------
CREATE TABLE note (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  title TEXT NOT NULL DEFAULT '',
  content TEXT NOT NULL DEFAULT '',
  favorite INTEGER NOT NULL DEFAULT 0,
  reminder_at TEXT,
  last_viewed_at TEXT,
  custom TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE note_tab (
  id INTEGER PRIMARY KEY,
  note_id INTEGER NOT NULL REFERENCES note(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  content TEXT NOT NULL DEFAULT '',
  position INTEGER NOT NULL DEFAULT 0
);

CREATE VIRTUAL TABLE note_fts USING fts5(
  title, content,
  content='note', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER note_fts_ai AFTER INSERT ON note BEGIN
  INSERT INTO note_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
END;
CREATE TRIGGER note_fts_ad AFTER DELETE ON note BEGIN
  INSERT INTO note_fts(note_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content);
END;
CREATE TRIGGER note_fts_au AFTER UPDATE ON note BEGIN
  INSERT INTO note_fts(note_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content);
  INSERT INTO note_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
END;

-- contacts -------------------------------------------------------------------
CREATE TABLE contact (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  uid TEXT,
  first_name TEXT NOT NULL DEFAULT '',
  last_name TEXT NOT NULL DEFAULT '',
  display_name TEXT NOT NULL DEFAULT '',
  organization TEXT,
  photo_sha TEXT,
  color TEXT,
  favorite INTEGER NOT NULL DEFAULT 0,
  birthday TEXT,
  anniversary TEXT,
  notes TEXT,
  custom TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE contact_field (
  id INTEGER PRIMARY KEY,
  contact_id INTEGER NOT NULL REFERENCES contact(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN ('phone','email','address','url','date')),
  label TEXT,
  value TEXT NOT NULL,
  is_primary INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_contact_field_value ON contact_field(kind, value);

CREATE TABLE contact_group_m (
  contact_id INTEGER NOT NULL REFERENCES contact(id) ON DELETE CASCADE,
  list_id INTEGER NOT NULL REFERENCES item_list(id) ON DELETE CASCADE,
  PRIMARY KEY (contact_id, list_id)
);

-- passwords ------------------------------------------------------------------
CREATE TABLE pw_entry (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  username TEXT,
  password_enc TEXT,
  url TEXT,
  notes TEXT,
  icon_sha TEXT,
  strength INTEGER,
  custom TEXT NOT NULL DEFAULT '{}'
);

-- sync -----------------------------------------------------------------------
CREATE TABLE sync_profile (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  url TEXT,
  user TEXT,
  direction TEXT NOT NULL DEFAULT 'two_way',
  interval_min INTEGER NOT NULL DEFAULT 60,
  mapping TEXT,
  last_sync TEXT,
  last_token TEXT
);";

/// v2 — diary module. `item.kind` gains 'diary'; SQLite cannot ALTER a CHECK
/// constraint, so `item` is rebuilt (12-step recipe: `foreign_keys` OFF, copy,
/// drop, rename — column order unchanged). Adds `diary_entry` (one row per
/// local calendar day, unique on `entry_date`) with its own FTS5 index.
pub const V2: &str = r"
CREATE TABLE item_new (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('event','task','note','diary','contact','password','message')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT,
  private INTEGER NOT NULL DEFAULT 0,
  list_id INTEGER REFERENCES item_list(id),
  sort_key TEXT
);
INSERT INTO item_new SELECT id, kind, created_at, updated_at, deleted_at, private, list_id, sort_key FROM item;
DROP TABLE item;
ALTER TABLE item_new RENAME TO item;
CREATE INDEX idx_item_kind_list ON item(kind, list_id) WHERE deleted_at IS NULL;

CREATE TABLE diary_entry (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  entry_date TEXT NOT NULL,
  content TEXT NOT NULL DEFAULT '',
  mood TEXT,
  weather TEXT,
  custom TEXT NOT NULL DEFAULT '{}'
);
CREATE UNIQUE INDEX idx_diary_date ON diary_entry(entry_date);

CREATE VIRTUAL TABLE diary_fts USING fts5(
  content,
  content='diary_entry',
  content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER diary_fts_ai AFTER INSERT ON diary_entry BEGIN
  INSERT INTO diary_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER diary_fts_ad AFTER DELETE ON diary_entry BEGIN
  INSERT INTO diary_fts(diary_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;
CREATE TRIGGER diary_fts_au AFTER UPDATE ON diary_entry BEGIN
  INSERT INTO diary_fts(diary_fts, rowid, content) VALUES ('delete', old.id, old.content);
  INSERT INTO diary_fts(rowid, content) VALUES (new.id, new.content);
END;
";

/// All migrations in order. Runs each unapplied one inside a transaction with
/// foreign keys disabled (the V2 `item` rebuild requires it; the pragma is a
/// no-op inside a transaction, so it is toggled around the whole loop).
///
/// # Errors
/// Propagates SQL errors from any migration batch.
pub fn apply_all(conn: &mut Connection) -> Result<()> {
    use rusqlite::OptionalExtension;
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = (|| {
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (target, sql) in [(1, V1), (2, V2), (3, V3), (4, V4), (5, V5), (6, V6), (7, V7)] {
            if v < target {
                let tx = conn.transaction()?;
                tx.execute_batch(sql)?;
                tx.pragma_update(None, "user_version", target)?;
                tx.commit()?;
            }
        }
        Ok(())
    })();
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // Integrity probe: the V2 rebuild must leave no dangling references.
    let broken: i64 = conn
        .query_row("PRAGMA foreign_key_check", [], |r| {
            r.get::<_, usize>(0).map(|_| 1)
        })
        .optional()?
        .unwrap_or(0);
    if broken > 0 {
        return Err(Error::Parse {
            what: "migration",
            message: String::from("foreign_key_check reported dangling references after migration"),
        });
    }
    result
}

/// v3 — task hierarchy. `item.parent_id` for tree views (tasks first);
/// plain ADD COLUMN, NULL default, so no rebuild is needed.
pub const V3: &str = r"
ALTER TABLE item ADD COLUMN parent_id INTEGER REFERENCES item(id);
";

/// v4 — device sync. Every item gets a device-independent `guid` so two
/// installs can exchange data without colliding local row ids, and a
/// trigger-fed `change_log` records every insert/update/tombstone on `item`
/// (all kind-table writers update `item.updated_at`, so `item` is the single
/// capture point). The purge trigger drops an item's history: deletion is
/// communicated by the tombstone while it lives, and after a purge there is
/// nothing left to replicate. The backfill gives pre-v4 rows their guid.
pub const V4: &str = r"
ALTER TABLE item ADD COLUMN guid TEXT;
CREATE UNIQUE INDEX idx_item_guid ON item(guid) WHERE guid IS NOT NULL;
CREATE TABLE device (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  platform TEXT NOT NULL,
  paired_at TEXT NOT NULL,
  last_sync TEXT
);
CREATE TABLE change_log (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL,
  op TEXT NOT NULL CHECK(op IN ('put','del')),
  at TEXT NOT NULL
);
CREATE INDEX idx_change_item ON change_log(item_id);
CREATE TRIGGER change_ins AFTER INSERT ON item BEGIN
  INSERT INTO change_log (item_id, op, at) VALUES (new.id, 'put', new.updated_at);
END;
CREATE TRIGGER change_upd AFTER UPDATE ON item BEGIN
  INSERT INTO change_log (item_id, op, at)
  VALUES (new.id,
          CASE WHEN new.deleted_at IS NOT NULL THEN 'del' ELSE 'put' END,
          new.updated_at);
END;
CREATE TRIGGER change_purged AFTER DELETE ON item BEGIN
  DELETE FROM change_log WHERE item_id = old.id;
END;
UPDATE item SET guid = lower(hex(randomblob(16))) WHERE guid IS NULL;
";

/// v5 — AI plumbing (`docs/ai_integ_plan.md`, "Infrastructure needed"). `ai_job`
/// is the cancellable job queue the background worker drains; `embedding`
/// holds one vector per item as a little-endian f32 BLOB inside this
/// `SQLCipher` database so embeddings keep the encrypted-at-rest guarantee
/// (plan §1: encrypted DB or
/// nowhere). Similarity is computed in Rust with an FTS5 prefilter; a
/// loadable-extension vector index (sqlite-vec) is deferred — see
/// `docs/data-model.md` "AI model".
pub const V5: &str = r"
CREATE TABLE ai_job (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK(state IN ('queued','running','done','failed','cancelled')),
  params TEXT NOT NULL DEFAULT '{}',
  result TEXT,
  error TEXT,
  created_at TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT
);
CREATE INDEX idx_ai_job_state ON ai_job(state, id);

CREATE TABLE embedding (
  item_id INTEGER PRIMARY KEY REFERENCES item(id) ON DELETE CASCADE,
  model TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  vec BLOB NOT NULL
);
";

/// v6 — token accounting (plan §4: cost accounting for tier-2 providers,
/// surfaced in Settings). Counters ride on `ai_job` rows — usage is
/// per-job by construction — and `ai_usage` aggregates them. Plain ADD
/// COLUMN, so no rebuild.
pub const V6: &str = r"
ALTER TABLE ai_job ADD COLUMN tokens_in INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_job ADD COLUMN tokens_out INTEGER NOT NULL DEFAULT 0;
";

/// v7 — Google Calendar/Tasks sync. `event`/`task` gain a remote id, a
/// dirty flag for the push phase (`clean|dirty|deleted`) and the remote
/// `updated` stamp used for last-writer-wins on pull. `google_link` is the
/// per-mail-account OAuth join: which Google identity, which local calendar
/// row and Google task list, plus the incremental-read watermarks
/// (events.list syncToken, tasks.list updatedMin) and connection status.
pub const V7: &str = r"
ALTER TABLE event ADD COLUMN gcal_id TEXT;
ALTER TABLE event ADD COLUMN sync_state TEXT NOT NULL DEFAULT 'clean';
ALTER TABLE event ADD COLUMN sync_updated_at TEXT;
CREATE UNIQUE INDEX idx_event_gcal ON event(gcal_id) WHERE gcal_id IS NOT NULL;
ALTER TABLE task ADD COLUMN gtask_id TEXT;
ALTER TABLE task ADD COLUMN sync_state TEXT NOT NULL DEFAULT 'clean';
ALTER TABLE task ADD COLUMN sync_updated_at TEXT;
CREATE UNIQUE INDEX idx_task_gtask ON task(gtask_id) WHERE gtask_id IS NOT NULL;
CREATE TABLE google_link (
  mail_account_id INTEGER PRIMARY KEY REFERENCES mail_account(id) ON DELETE CASCADE,
  google_email TEXT NOT NULL,
  calendar_id INTEGER REFERENCES calendar(id) ON DELETE SET NULL,
  tasklist_id TEXT,
  ev_sync_token TEXT,
  task_updated_min TEXT,
  status TEXT NOT NULL DEFAULT 'connected',
  last_sync TEXT,
  last_error TEXT
);
";
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a populated v1 database (no V2 objects), as an old install has.
    fn v1_populated() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        // Raw insert: `create_item` stamps a guid, which only exists from V4.
        conn.execute(
            "INSERT INTO item (kind, created_at, updated_at) VALUES ('note', 'x', 'x')",
            [],
        )
        .unwrap();
        let item = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO note (item_id, title, content) VALUES (?1, 'pre-migration', 'hello')",
            [item],
        )
        .unwrap();
        conn
    }

    #[test]
    fn v2_rebuild_preserves_rows_and_adds_diary() {
        let mut conn = v1_populated();
        apply_all(&mut conn).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 7);
        // Pre-existing rows survived the `item` rebuild, kind CHECK intact.
        let (kind, title): (String, String) = conn
            .query_row(
                "SELECT i.kind, n.title FROM item i JOIN note n ON n.item_id = i.id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "note");
        assert_eq!(title, "pre-migration");
        // Diary objects exist and accept an entry.
        conn.execute(
            "INSERT INTO item (kind, created_at, updated_at) VALUES ('diary', 'x', 'x')",
            [],
        )
        .unwrap();
        let item: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO diary_entry (item_id, entry_date, content) VALUES (?1, '2026-10-19', '<p>hi</p>')",
            [item],
        )
        .unwrap();
        // CHECK rejects the removed kinds' neighbours: 'diary' is the addition.
        assert!(conn
            .execute(
                "INSERT INTO item (kind, created_at, updated_at) VALUES ('diaryx', 'x', 'x')",
                []
            )
            .is_err());
        // No dangling references.
        let broken: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0);
        // V3 added item.parent_id.
        conn.execute(
            "INSERT INTO item (kind, created_at, updated_at, parent_id) \
             SELECT kind, created_at, updated_at, id FROM item WHERE id = ?1",
            [item],
        )
        .unwrap();
        let parent: i64 = conn
            .query_row(
                "SELECT parent_id FROM item WHERE id = ?1",
                [conn.last_insert_rowid()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent, item);
        // Re-running is a no-op.
        apply_all(&mut conn).unwrap();
        let v2: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 7);
    }

    #[test]
    fn v7_adds_google_sync_columns_and_link() {
        let mut conn = v1_populated();
        apply_all(&mut conn).unwrap();
        // google_link accepts a row and enforces its status CHECK default.
        conn.execute(
            "INSERT INTO mail_account (kind, name, user, host) \
             VALUES ('imap', 'Test', 'me@gmail.com', 'imap.gmail.com')",
            [],
        )
        .unwrap();
        let account: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO google_link (mail_account_id, google_email) \
             VALUES (?1, 'me@gmail.com')",
            [account],
        )
        .unwrap();
        let (email, status): (String, String) = conn
            .query_row(
                "SELECT google_email, status FROM google_link WHERE mail_account_id = ?1",
                [account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((email.as_str(), status.as_str()), ("me@gmail.com", "connected"));
        // New event/task rows default to `clean` and accept remote ids.
        conn.execute(
            "INSERT INTO item (kind, created_at, updated_at) VALUES ('event', 'x', 'x')",
            [],
        )
        .unwrap();
        let item: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO event (item_id, title, start_utc, gcal_id) \
             VALUES (?1, 'Sync', '2026-10-21T10:00:00Z', 'abc123')",
            [item],
        )
        .unwrap();
        let state: String = conn
            .query_row("SELECT sync_state FROM event WHERE gcal_id = 'abc123'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, "clean");
        // Remote ids are unique.
        conn.execute(
            "INSERT INTO item (kind, created_at, updated_at) VALUES ('event', 'x', 'x')",
            [],
        )
        .unwrap();
        let dup: i64 = conn.last_insert_rowid();
        assert!(conn
            .execute(
                "INSERT INTO event (item_id, title, start_utc, gcal_id) \
                 VALUES (?1, 'Dup', '2026-10-21T11:00:00Z', 'abc123')",
                [dup]
            )
            .is_err());
    }
}
