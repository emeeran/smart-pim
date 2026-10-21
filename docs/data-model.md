# Smart PIM — Data Model (Gate 1)

SQLCipher SQLite, WAL. `item` is the universal anchor giving every module tags, links,
history, attachments, privacy, and trash for free (spec S10).

## Core

```sql
-- migrations table
CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);

CREATE TABLE item (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('event','task','note','contact','password','message')),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  deleted_at TEXT,                      -- trash
  private INTEGER NOT NULL DEFAULT 0,
  list_id INTEGER,                      -- owning list/calendar/group (FK below)
  sort_key TEXT
);
CREATE INDEX idx_item_kind_list ON item(kind, list_id) WHERE deleted_at IS NULL;

CREATE TABLE item_list (              -- "Local Tasks/Notes/Calendars/Contacts/Passwords" trees
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL, parent_id INTEGER REFERENCES item_list(id),
  name TEXT NOT NULL, color TEXT, icon TEXT, sort_order INTEGER NOT NULL DEFAULT 0,
  settings TEXT                        -- JSON: floating-tasks opt-in, calendar show/hide...
);

CREATE TABLE tag (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, color TEXT);
CREATE TABLE item_tag (item_id INTEGER REFERENCES item(id), tag_id INTEGER REFERENCES tag(id),
  PRIMARY KEY(item_id, tag_id));

CREATE TABLE item_link (              -- "Related Items" cross-links
  from_id INTEGER REFERENCES item(id), to_id INTEGER REFERENCES item(id),
  PRIMARY KEY(from_id, to_id));
CREATE TRIGGER item_link_sym AFTER INSERT ON item_link
  BEGIN INSERT OR IGNORE INTO item_link VALUES(NEW.to_id, NEW.from_id); END;

CREATE TABLE item_version (           -- automatic version history (S10)
  id INTEGER PRIMARY KEY, item_id INTEGER REFERENCES item(id),
  saved_at TEXT NOT NULL, snapshot TEXT NOT NULL);   -- JSON of full item payload

CREATE TABLE attachment (
  id INTEGER PRIMARY KEY, item_id INTEGER REFERENCES item(id),
  sha256 TEXT NOT NULL, filename TEXT NOT NULL, size INTEGER NOT NULL, mime TEXT);
CREATE UNIQUE INDEX idx_att_sha ON attachment(sha256);  -- dedup; media/ file per hash

CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY, at TEXT NOT NULL, item_id INTEGER,
  action TEXT NOT NULL, detail TEXT);                -- JSON before/after refs
CREATE TABLE undo_op (
  id INTEGER PRIMARY KEY, at TEXT NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL);
```

## Mail

```sql
CREATE TABLE mail_account (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, kind TEXT CHECK(kind IN('imap','pop3')),
  host TEXT, port INTEGER, security TEXT, user TEXT,  -- secrets in OS keyring; ref here
  color TEXT, identity_name TEXT, reply_to TEXT, oauth INTEGER DEFAULT 0, settings TEXT);

CREATE TABLE mail_folder (
  id INTEGER PRIMARY KEY, account_id REFERENCES mail_account(id),
  parent_id REFERENCES mail_folder(id), name TEXT NOT NULL,
  role TEXT,            -- inbox|drafts|sent|spam|trash|archive|null
  uidvalidity INTEGER, last_seen_uid INTEGER, sync_state TEXT);

CREATE VIRTUAL TABLE message_fts USING fts5(
  subject, from_name, from_addr, to_addrs, body, content='message', content_rowid='id');

CREATE TABLE message (
  id INTEGER PRIMARY KEY, account_id REFERENCES mail_account(id),
  folder_id REFERENCES mail_folder(id), uid INTEGER,
  message_id TEXT UNIQUE, in_reply_to TEXT, thread_id INTEGER REFERENCES thread(id),
  subject TEXT, from_name TEXT, from_addr TEXT, to_addrs TEXT, cc_addrs TEXT,
  date TEXT, size INTEGER, flags TEXT,          -- JSON: seen, flagged, answered, pinned...
  category TEXT, private INTEGER DEFAULT 0, html_path TEXT,  -- sanitized HTML blob on disk
  snippet TEXT, created_at TEXT, deleted_at TEXT);
CREATE INDEX idx_msg_folder_date ON message(folder_id, date DESC);
CREATE INDEX idx_msg_thread ON message(thread_id);

CREATE TABLE thread (id INTEGER PRIMARY KEY, subject TEXT, account_id INTEGER,
  last_date TEXT, pinned INTEGER DEFAULT 0);

CREATE TABLE mail_rule (id INTEGER PRIMARY KEY, account_id INTEGER, enabled INTEGER,
  conditions TEXT NOT NULL, actions TEXT NOT NULL, position INTEGER);
CREATE TABLE identity (id INTEGER PRIMARY KEY, account_id INTEGER, name TEXT,
  email TEXT, signature_html TEXT, is_default INTEGER);
CREATE TABLE template (id INTEGER PRIMARY KEY, kind TEXT CHECK(kind IN
  ('mail','event','task','note')), name TEXT, content TEXT NOT NULL);
CREATE TABLE outbox (id INTEGER PRIMARY KEY, draft_json TEXT NOT NULL,
  send_at TEXT, state TEXT, error TEXT);
```

## Calendar

```sql
CREATE TABLE calendar (id INTEGER PRIMARY KEY, name TEXT, color TEXT,
  kind TEXT DEFAULT 'local', url TEXT, sync_token TEXT, visible INTEGER DEFAULT 1);

CREATE TABLE event (
  id INTEGER PRIMARY KEY, item_id UNIQUE REFERENCES item(id),
  calendar_id REFERENCES calendar(id), title TEXT NOT NULL,
  start_utc TEXT NOT NULL, end_utc TEXT, all_day INTEGER DEFAULT 0,
  location TEXT, completion INTEGER DEFAULT 0, description TEXT,
  rrule TEXT, rdate TEXT, exdate TEXT,      -- ICS semantics
  category_id INTEGER, uid TEXT);           -- ICS UID for sync
CREATE TABLE event_reminder (id INTEGER PRIMARY KEY, event_id INTEGER,
  minutes_before INTEGER, method TEXT DEFAULT 'popup', sound TEXT);
CREATE TABLE event_participant (event_id INTEGER, contact_id INTEGER,
  email TEXT, status TEXT, PRIMARY KEY(event_id, email));
CREATE TABLE category (id INTEGER PRIMARY KEY, name TEXT, color TEXT,
  holiday INTEGER DEFAULT 0, kind TEXT);    -- shared by events/tasks
CREATE TABLE subscription (id INTEGER PRIMARY KEY, name TEXT, url TEXT,
  calendar_id INTEGER, interval_min INTEGER, last_fetched TEXT);
```

## Tasks / Notes / Contacts / Passwords

```sql
CREATE TABLE task (
  id INTEGER PRIMARY KEY, item_id UNIQUE REFERENCES item(id),
  title TEXT NOT NULL, notes TEXT,           -- rich text JSON
  start_utc TEXT, due_utc TEXT, priority INTEGER DEFAULT 0,   -- 0 none; 1..! levels
  percent INTEGER DEFAULT 0, completed_at TEXT,
  rrule TEXT, category_id INTEGER, floating INTEGER DEFAULT 0,
  assigned_contact_id INTEGER, custom TEXT);  -- JSON EAV {field_id: value}
CREATE TABLE custom_field (id INTEGER PRIMARY KEY, kind TEXT, name TEXT,
  type TEXT CHECK(type IN('text','date','dropdown','yesno','checkbox')), options TEXT);

CREATE TABLE note (
  id INTEGER PRIMARY KEY, item_id UNIQUE REFERENCES item(id),
  title TEXT, content TEXT NOT NULL,         -- rich text JSON (+ md_source)
  favorite INTEGER DEFAULT 0, reminder_at TEXT, last_viewed_at TEXT,
  custom TEXT);
CREATE TABLE note_tab (id INTEGER PRIMARY KEY, note_id INTEGER,
  title TEXT, content TEXT, position INTEGER);   -- in-note multi-tabs (S43)

CREATE TABLE contact (
  id INTEGER PRIMARY KEY, item_id UNIQUE REFERENCES item(id),
  first_name TEXT, last_name TEXT, display_name TEXT, organization TEXT,
  photo_sha TEXT,     -- avatar in media/
  color TEXT,         -- deterministic initial-avatar color
  favorite INTEGER DEFAULT 0, birthday TEXT, anniversary TEXT,
  notes TEXT, custom TEXT);
CREATE TABLE contact_field (id INTEGER PRIMARY KEY, contact_id INTEGER,
  kind TEXT CHECK(kind IN('phone','email','address','url','date')),
  label TEXT, value TEXT, is_primary INTEGER DEFAULT 0);
CREATE TABLE contact_group_m (contact_id INTEGER, list_id INTEGER,
  PRIMARY KEY(contact_id, list_id));        -- multi-group membership

CREATE TABLE pw_entry (
  id INTEGER PRIMARY KEY, item_id UNIQUE REFERENCES item(id),
  title TEXT, username TEXT, password_enc TEXT,   -- app-level AES-GCM on top of DB (P0),
  url TEXT, notes TEXT, icon_sha TEXT, strength INTEGER, custom TEXT);
```

## Shared ops

```sql
CREATE TABLE reminder_queue (id INTEGER PRIMARY KEY, item_id INTEGER, due_at TEXT,
  fired_at TEXT, method TEXT, snoozed_to TEXT);
CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT);   -- theme, autolock, shortcuts...
CREATE TABLE sync_profile (id INTEGER PRIMARY KEY, kind TEXT, url TEXT, user TEXT,
  direction TEXT, interval_min INTEGER, mapping TEXT, last_sync TEXT, last_token TEXT);
```

## Invariants & indexes

- FTS external-content tables kept in sync by triggers on insert/update/delete of
  `message` (and per-module text tables); `(id, kind)` projections into a unified
  `search_index` virtual table for Ctrl+Q global search.
- Deletion = `item.deleted_at` set (except messages: moved to Trash folder + soft flag);
  permanent purge from Trash only.
- Recurrence expansion never stored; computed on read with cap (S/arch N1).
- Migrations: forward-only, transactional; embedded in binary.

## Acceptance tests keyed to this model

DT-1 trigger-symmetric cross-links; DT-2 FTS trigger consistency after 100k inserts;
DT-3 recurrence read-expansion cap; DT-4 version snapshot on every update path;
DT-5 keyset pagination stable under inserts; DT-6 trash restore restores links+tags.

## Later migrations (Gate 5)

Applied by `schema::apply_all` (forward-only, `PRAGMA user_version`):

```sql
-- v2: diary module. `item.kind` gains 'diary' — CHECK rebuild (SQLite cannot
-- ALTER a CHECK); column order unchanged. One entry per LOCAL calendar day.
CREATE TABLE diary_entry (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL UNIQUE REFERENCES item(id) ON DELETE CASCADE,
  entry_date TEXT NOT NULL,             -- 'YYYY-MM-DD', local date
  content TEXT NOT NULL DEFAULT '',     -- HTML (Tiptap)
  mood TEXT, weather TEXT,
  custom TEXT NOT NULL DEFAULT '{}'
);
CREATE UNIQUE INDEX idx_diary_date ON diary_entry(entry_date);
CREATE VIRTUAL TABLE diary_fts USING fts5(content, content='diary_entry',
  content_rowid='id', tokenize='unicode61 remove_diacritics 2');  -- + ai/ad/au triggers

-- v3: task hierarchy
ALTER TABLE item ADD COLUMN parent_id INTEGER REFERENCES item(id);

-- v4: device sync. Every item gets a stable random guid (payloads reference
-- items across devices without colliding local ids); a trigger-fed change_log
-- records insert/update/tombstone on `item` (the single capture point — all
-- kind writers bump item.updated_at); `device` registers pairings.
ALTER TABLE item ADD COLUMN guid TEXT;
CREATE UNIQUE INDEX idx_item_guid ON item(guid) WHERE guid IS NOT NULL;
CREATE TABLE device (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, platform TEXT NOT NULL,
  paired_at TEXT NOT NULL, last_sync TEXT
);
CREATE TABLE change_log (
  id INTEGER PRIMARY KEY, item_id INTEGER NOT NULL,
  op TEXT NOT NULL CHECK(op IN ('put','del')), at TEXT NOT NULL
);  -- + change_ins/change_upd/change_purged triggers

-- v5: AI plumbing. ai_job is the cancellable queue the shell worker drains;
-- embedding holds one vector per item as a little-endian f32 BLOB inside
-- this (SQLCipher-encrypted) database; staleness = item.updated_at > embedding.updated_at.
CREATE TABLE ai_job (
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK(state IN ('queued','running','done','failed','cancelled')),
  params TEXT NOT NULL DEFAULT '{}', result TEXT, error TEXT,
  created_at TEXT NOT NULL, started_at TEXT, finished_at TEXT
);  -- v6 adds tokens_in/tokens_out counters (usage accounting).
CREATE TABLE embedding (
  item_id INTEGER PRIMARY KEY REFERENCES item(id) ON DELETE CASCADE,
  model TEXT NOT NULL, updated_at TEXT NOT NULL, vec BLOB NOT NULL
);
```

Diary entries participate in the universal model (tags, links, versions,
trash, audit) via their `item` anchor; `global_search` covers `diary_fts`.

## Sync model (v4)

`core::sync` exchanges guid-addressed `ChangePayload`s (item scalars, tags,
links, the kind-table row, child sets like `contact_field`). Pull =
`changes_since(cursor)` / `snapshot_all` for fresh or stale peers; apply =
`apply_batch` with last-writer-wins on `updated_at`; every losing version is
preserved into `item_version`. Apply-time change-log rows are pruned so
replicated state is not re-broadcast; `prune_change_log` records a floor in
`setting` (`sync.log_floor`) so stale peer cursors trigger a re-snapshot.
Messages are desktop-authoritative and never replicate.

## AI model (v5)

Embeddings for semantic search live in the `embedding` table **inside this
SQLCipher database** — never in a plaintext sidecar (`docs/ai_integ_plan.md`
§1). `core::aisearch` encodes/decodes f32 vectors, scores cosine, and fuses
with FTS5 ranks (0.7 semantic / 0.3 lexical); a sqlite-vec index is deferred —
the Rust scan sits behind an FTS prefilter and stays fast at ten-thousands of
items. Diary embedding is gated behind its own `ai.feature.diary` flag.
The shell's `aiworker` drains `ai_job` with per-job cancellation flags and
never holds the store lock across a provider call. Each job records the
provider-reported token usage (`tokens_in`/`tokens_out`, v6); `ai_usage`
aggregates the counters for the Settings panel, which estimates spend from
user-entered per-1k rates. Counters only — no prompt content is retained
beyond the job's own result.
