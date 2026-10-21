# Changelog

All notable changes to Smart PIM are documented here.
Format: Keep a Changelog; versioning: SemVer.

## [Unreleased] — AI integration (docs/ai_integ_plan.md)

### Added
- **Local-first AI**, off unless a provider answers: OpenAI-compatible local
  server (Ollama/llama.cpp) by default; cloud providers opt-in per feature with
  a BYO key stored in the encrypted database and never displayed again.
- Features (all preview-then-apply; nothing the model produces is ever written
  without confirmation): note summarize; email thread digests; reply drafting
  into the existing compose; natural-language task capture (prefills the
  editor); action-item extraction creating linked tasks; auto-tag suggestions;
  inbox-rule suggestions applied through the rules engine; calendar
  find-a-slot (pure computation); contact duplicate detection with
  merge-diff; and semantic search via embeddings (toggle in global search).
  Diary reflection is a separate opt-in. **Passwords is excluded entirely** —
  no password material ever reaches a model.
- Infrastructure: schema v5 (`ai_job` cancellable queue + encrypted
  `embedding` table), background worker with cooperative cancellation that
  never blocks the UI on a slow model, prompt-injection defenses
  (sanitized `<data>` payloads, strict-JSON replies), and an AI settings
  section with model picker and per-feature toggles.

### Changed
- Databases migrate to schema v5 (forward-only, in place).

### Added — AI plan completion (token accounting, subtask split, signature contacts)
- **Token/cost accounting** (plan §4): every chat job records the provider's
  token usage on its `ai_job` row (schema v6 adds `tokens_in`/`tokens_out`);
  Settings → AI shows totals, a per-feature-kind breakdown, an estimated
  spend once per-1k-token rates are entered (aimed at tier-2 cloud models),
  and a counter reset. Counters only — no prompt content is retained.
- **Task subtask splitting** (plan §3): "Suggest subtasks…" in the task
  editor proposes 3–5 ordered subtasks as an editable list; applying creates
  real child tasks through `task_save` + the `item.parent_id` tree.
- **Signature → contact enrichment** (plan §3): "AI Contact…" in the mail
  reading pane proposes contact fields — deterministic local scan for
  emails/phones/URLs, model only fills names/organization — with an
  editable diff and an automatic merge suggestion when the sender's address
  already matches a contact. Merging fills empty fields and appends new
  addresses; it never overwrites existing values. Both features ship behind
  their own `ai.feature.subtasks` / `ai.feature.contacts` flags.

### Changed
- Databases migrate to schema v6 (v5 + usage counters; forward-only, in place).

### Added — EPIM parity slice: Today dashboard, Related Items, UX polish
- **Today widget board** (`core::related::today_dashboard`): Agenda
  (today's events), TO DO (due/overdue tasks, checkable in place through
  the same task_save path), per-account Messages unread with click-through,
  and Anniversaries — next birthdays/anniversaries with computed ages,
  straight from existing `contact` columns. No weather widget: local-first
  means no background network calls.
- **Related Items** (`related_items` command + shared `RelatedPanel`):
  EPIM's cross-module tab. A contact's panel shows their mail (exact
  sender match plus recipient scan), assigned tasks, events they
  participate in (`event_participant`), and explicit `item_link` edges —
  deduplicated, query-time, nothing copied. Rows navigate across modules
  (jump target consumed by Mail/Tasks/Contacts).
- **Calendar**: per-calendar visibility checkboxes (EPIM's checkbox
  column) filtering every view client-side, persisted locally.
- **Tasks**: tree-view inline "Add a task" row and hover indent/outdent
  buttons (⇤/⇥) driving the existing cycle-safe `task_set_parent`.
- **Contacts**: "Send E-mail" (pre-fills the Mail compose via a
  cross-module hand-off) and "Schedule Meeting" actions on the detail card;
  Related Items tab now shows the rich panel.
- **Passwords**: "Copy user name" alongside the existing reveal/copy.
- **Status bar**: reminder bell navigates to Today.

## [Unreleased] — full EPIM Pro clone slice (sync, interchange, UI parity)

### Added
- **Wi-Fi device sync**: schema v4 gives every item a stable guid and a
  trigger-fed change log; mDNS discovery, TLS transport, 6-digit pairing;
  last-writer-wins engine that preserves every losing version into item
  history. Pair devices from Settings → Sync → Devices; CLI peer simulator
  at `src-tauri/examples/sync_peer.rs`. The Android companion app speaks the
  documented protocol (docs/epim_clone_plan.md §D1).
- **Cloud sync**: CalDAV/CardDAV accounts (Settings → Sync → Cloud) and
  whole-database file sync to any WebDAV endpoint (Dropbox/Drive/Box via
  their DAV gateways) using encrypted VACUUM INTO snapshots with ETag
  optimistic concurrency; a changed remote is saved to Backups, never
  applied blindly.
- **Automatic backups**: daily snapshot with 14-copy retention (setting
  `backup.interval_min`, 0 disables) and Backup list/restore in Settings.
- **EPIM interchange**: EssentialPIM CSV import presets with automatic
  header detection; export to CSV, HTML, RTF, XML, Markdown (plus the
  existing ICS/VCF/EML) via File → Export.
- **UI parity with EssentialPIM Pro**: per-module Smart Lists tree with
  counts, mini month in the Calendar tree, date-grouped mail cards with
  avatars and a pinned quick-reply box, A–Z letter rails in Contacts and
  Passwords, kanban column counts and card progress, notes breadcrumb with
  history, full New Appointment dialog (recurrence builder, reminder
  presets, completion slider), and a sectioned Settings pane.

### Changed
- Databases migrate to schema v4 (forward-only; existing databases upgrade
  in place, no action needed).

## [Unreleased] — rebrand + EfficientPIM-style shell + Diary

### Changed (BREAKING)
- **Product renamed Prodforge → Smart PIM everywhere**: Cargo packages
  (`smart-pim` / `smart_pim_lib`), binary `/usr/bin/smart-pim`, `.deb`
  package name, app identifier `io.smartpim.app` (app-data dir moves), DB
  filename `smartpim.db`, and the argon2 key-derivation salt.
  **Databases created before the rebrand cannot be opened.** Migrate by
  exporting CSV/vCard/ICS from the old app and re-importing.
- Module shortcuts remapped: Ctrl+6 = Diary; Contacts/Passwords/Trash are
  now Ctrl+7/8/9.
- `.deb` ships `smart-pim_0.0.1_amd64.deb`; MIME type is now
  `application/x-smartpim-import`; WM_CLASS is `smart-pim`.

### Added
- Diary module: one entry per local day, month navigator + streak,
  debounced autosave, mood, FTS5 search, trash/versions/links via the
  universal item model (schema v2).
- Grey theme; system theme now follows the OS live (previously only
  remapped the window background).
- EfficientPIM-style shell: 208px labelled navigation tree with count
  badges and collapse toggle; single app toolbar under the menu bar;
  universal "Notes | Related Items" tabs with link/unlink in Notes,
  Contacts, Passwords.
- Calendar Year and Agenda views; task tree view with parent assignment
  (schema v3 `item.parent_id`).

## [0.1.0] — 2026-10-19

First packaged release. Clean-room implementation; original code and assets.

### Added
- SQLCipher (AES-256) database with argon2id key derivation, lock screen,
  idle auto-lock, `--portable` mode.
- Universal item model: tags, symmetric Related Items, private flag,
  version history with restore, soft-delete Trash, audit log, undo.
- FTS5 search across mail/tasks/notes (100k-message corpus: search < 200 ms,
  import 6.7 s, 390 MiB peak RSS).
- Backup (`VACUUM INTO`) with retention pruning.
- Mail: IMAP/POP3 clients (TLS/STARTTLS), incremental UID sync, Unified
  Inbox, threading, reading pane with allowlist HTML sanitizer + per-sender
  remote-content blocking, compose + Send Later outbox with missed-fire
  recovery, rules engine, filtered search, convert to task/note/event.
- Calendar: Day/Week/Month grids, RRULE expansion (cap 730), reminders,
  ICS import/export.
- Tasks: list + Kanban + percent completion.
- Notes: Tiptap rich-text editor with autosave and snapshots.
- Contacts: letter bar, vCard import/export, CSV import with mapping +
  dry-run validation + atomic commit.
- Passwords: masked entries, generator, strength heuristic.
- Reminders: shared queue, missed-fire recovery, 30 s UI poll.
- Tauri 2 desktop shell for Linux (`.deb`), original icon and branding.

### Known limitations (documented, not hidden)
- No OpenPGP/S-MIME, CalDAV/CardDAV, sticky-note windows, Auto-Type,
  adaptive spam learning, OAuth vendor sync (needs client IDs),
  translation service, weather provider.
- IMAP: attachment download and server flag mirroring not yet implemented;
  polling sync (no IDLE push).
- AT-19 (60 fps virtualized scroll with 100k rows) implemented and unit-
  tested at the windowing level; frame-rate measurement requires an
  interactive display session.
