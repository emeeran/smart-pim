# Smart PIM — Tasks (Gate 1)

Small, verifiable tasks. Every P0 spec item maps to ≥1 task and ≥1 acceptance test.
Status: ☐ / ~ / ✔ / UNRESOLVED. Tests are numbered AT-n and live next to the code.

## T0 Bootstrap (Gate 2 start)
- ~ T0.1 Cargo workspace: `core` crate done (rusqlite 0.37 bundled-sqlcipher-vendored-
  openssl, mail-parser 0.11, rrule 0.14, chrono-tz, csv, argon2, sha2, zeroize — pinned in
  Cargo.lock). `src-tauri` shell, tokio, imap/lettre/rpgp/ts-rs deferred to T2/T3 wires.
- ~ T0.2 Vite + Preact shell skeleton DONE (src/: Preact 10 + signals, tsc strict,
  vitest, vite build = 41 kB JS). ts-rs binding generation NOT started — types are
  hand-mirrored in src/api.ts for now (drift risk noted; ts-rs lands with Gate 4 IPC
  expansion).
- ~ T0.3 `core::store`: SQLCipher key via argon2id, WAL + FK + synchronous pragmas,
  forward-only migration runner (PRAGMA user_version). AT: open/reopen/probe/migration-once
  (store::tests). Pool = single handle for now; tokio pool comes with Tauri wiring.
- ~ T0.4 Design tokens (light + dark, src/design/tokens.css) + original code-drawn SVG
  icon set (src/design/icons.ts) + original logo mark (logo.ts, PNGs under
  src-tauri/icons/). Light theme UI-verified; dark palette written but not
  screenshot-verified yet.

## T1 Core engine (Gate 2, headless)
- ~ T1.1 `item` + lists + tags + links + versions + audit/undo (DT-1, DT-4, DT-6).
  AT: trash lifecycle, symmetric links + undo, undo-create, version-on-update, tag
  roundtrip (repo::tests); purge cascade (gate2::trash_lifecycle). Attachments table
  exists; attachment file store (media/ + sha2 write path) still open.
- ~ T1.2 FTS5 external-content triggers (mail/task/note) + unified global search with
  hostile-input escaping. AT: DT-2 100k consistency + AT-5 search latency (release:
  112 ms for 100k, budget 200 ms) — gate2::at5_at13.
- ~ T1.3 Crypto: argon2id → SQLCipher key derivation, probe/open path, zeroized key.
  AT: wrong-passphrase rejected, reopen OK (store::tests). UI lock screen is T2.2.
  Auto-lock timer not yet implemented (T2.2).
- ~ T1.4 Backup create (VACUUM INTO, encrypted) + retention pruning. AT: snapshot opens
  standalone with data intact, prune keeps N (backup::tests). Scheduler tick = Tauri
  wiring (T3-shared).
- ~ T1.5 Import framework: validate_only dry-run report + commit_all single-transaction
  with rollback-on-error. AT: abort leaves DB empty (import::tests, gate2::csv atomicy).
  Progress events to UI come with Tauri wiring.
- ~ T1.6 CSV engine: RFC 4180 parse, delimiter sniff, alias-based mapping suggestions,
  contact field spec. AT: quoted/semicolon parse, mapping suggestions (csvio::tests).
  Per-module row appliers (tasks/notes) still open (T3 import wizard).
- ~ T1.7 ICS engine: unfold/parse (UTC, TZID via chrono-tz, VALUE=DATE, CLASS:PRIVATE,
  CATEGORIES), serialize, RRULE expansion via rrule 0.14 with cap. AT: 3-event sample
  incl. TZID, roundtrip, daily-rule expansion, bad-rule error (ical::tests);
  DB roundtrip (gate2::ics_pipeline). Missing: RDATE/EXDATE parsing, VTIMEZONE defs,
  recurring-task RRULE reuse for tasks (T3.26).
- ~ T1.8 VCF engine: 3.0/4.0 parse (N/FN/ORG/TEL/EMAIL/ADR/URL/BDAY/CATEGORIES/UID),
  write 4.0, PHOTO→warning. AT: vcard::tests + store roundtrip (contacts::tests,
  gate2::vcf_pipeline). Custom contact fields via `custom` JSON still open.
- ~ T1.9 Mail store: accounts, folders, message upsert + FTS, threading (Message-ID →
  References/In-Reply-To, normalized-subject fallback), folder unread counts, EML ingest
  via mail-parser, HTML-strip fallback. AT: threading paths, EML extraction, counts
  (mailstore::tests); AT-13 100k corpus release: insert 6.7 s, peak RSS 390 MiB
  (budget 1.5 GiB) — gate2::at5_at13. O(n²) thread-lookup fixed with
  idx_thread_subject. Progress events + per-message commit streaming = Tauri wiring.
- ~ T1.10 DONE (minus attachments): `src-tauri/src/mailsync.rs` — IMAP via
  `imap`+native-tls (none/starttls/tls), incremental UID fetch guarded by
  uidvalidity, folder discovery via LIST; minimal hand-rolled POP3 (UIDL
  fingerprinting, non-destructive); SMTP via lettre; background poll worker.
  AT-14 PASS against a real Dovecot container (scripts/mailfixture/): sync
  incremental + FTS-indexed, rules fire on ingest, POP3 dedupe +
  non-destructive, Send Later recovery + Mailpit sink. NOT DONE: attachment
  download to media/, IDLE push (poll only), IMAP flag-change mirroring.
- ~ T1.11 DONE: `core::rules` — AND conditions (from/to/subject contains|equals)
  → move/mark-read/tag, per-account or global, evaluated pure + applied on
  ingest; outbox queue with immediate + Send Later, missed fires flagged,
  retry with surfaced errors. AT-15 covered by 5 unit tests + fixture tests.
- ~ T1.12 DONE: `core::ops` — fires exactly once, missed-horizon recovery
  flagged, snooze re-arms; wired to the event editor and a 30 s UI poll that
  pops due reminders. AT-16: 4 unit tests.
- ~ T1.13 EML import + shared parse_message() (EML/IMAP/POP3 single source).
  EML/HTML/MD/RTF export still NOT STARTED (compose contract done; export
  dialogs remain).
- ~ T1.14 Keyset pagination: message_page (date,id cursor) + task_page. AT: full
  pagination walk, no dupes, inserts don't shift pages (pagination::tests).

## T2 UI shell (Gate 3)
- ~ T2.1 DONE for shell scope: window title `<db path> — Smart PIM` (verified),
  menu bar File/Edit/View/Go/Actions/Tools/Help with working Lock/Backup/Search/Go
  items, rail in S2 order with unread + trash badges, Ctrl+1..8 + Ctrl+Q +
  Ctrl+Shift+L wired (Ctrl+2 and Ctrl+Q verified under Xvfb via XTEST chords),
  status bar with counts + bell (pending reminders). Remaining: per-module menu
  items (Insert/Actions) activate with their modules in Gate 4.
- ~ T2.2 Lock screen UI + unlock/lock commands UI-verified end-to-end (create on
  first unlock, wrong-passphrase retry). Auto-lock poll thread implemented
  (settings `auto_lock_minutes`, default 15, 0=off; idle math unit-tested) but the
  timeout fire is not yet exercised end-to-end in the UI.
- ~ T2.3 Three-pane layout with drag splitters, widths persisted per module
  (localStorage). Resizing verified manually; persistence round-trip not
  covered by an automated test yet.
- ~ T2.4 Global search overlay (Ctrl+Q / Actions menu) UI-verified end-to-end:
  debounced FTS query → Rust → results grouped by kind, keyboard navigation.
  AT-5 re-run in UI (measuring round-trip incl. IPC, not just SQL) still open.
- ~ T2.5 Trash UI: unified list, type filter, restore + purge with confirm,
  counts refresh. Logic covered by shell tests (do_trash/restore/purge over
  pfcore); interactive walkthrough pending.
- ~ T2.6 Settings dialog: theme select (light/dark/system), auto-lock minutes
  (persisted to `setting`), manual backup with path feedback. Dark palette in
  tokens; not yet screenshot-verified. Shortcuts remapping (S57) NOT started.
- ~ T2.7 Smart Lists panel, pill tags, "Notes | Related Items" tabs built and
  styled. Version-history/restore UI and attachment manager NOT started (need
  the module views they attach to; Gate 4).
- ~ T2.8 VirtualList (windowed mounting, transform positioning, over-index
  rendering, keyset page-append + dedupe). Windowing/dedupe/grouping math
  unit-tested (8 vitest cases). AT-19 60 fps measurement UNRESOLVED: needs an
  interactive display session (Playwright + tauri-driver); headless Xvfb cannot
  produce honest frame timings. Flagged for Gate 4.

## T3 Modules (Gate 4, one feature per commit, tests each)
Mail: ~ T3.1 account wizard (create + test-sync, rollback on failure) ·
~ T3.2 folder tree with unread counts (unified inbox pending) ·
~ T3.3 date-grouped message cards (layouts pending) ·
~ T3.4 reading pane: server-side allowlist-sanitized HTML (core::htmlsan),
  remote-content blocking per sender, plain/HTML toggle, quoted reply
  (quick-reply box pending) · ~ T3.5 compose (plain text; signatures/
  templates/attachment reminder pending; rich editor shared with Notes) ·
☐ T3.6 threading UI · ~ T3.7 send later + outbox pump · ☐ T3.8 rules UI ·
☐ T3.9 flags/categories/pinned/archive · ☐ T3.10 convert ·
☐ T3.11 mail search/filters UI · ☐ T3.12 PGP (P1).
Calendar: ~ T3.13 Day/Week/Month grids (today highlight, click-to-create/edit) ·
☐ T3.14 Year/Agenda views · ~ T3.15 event editor dialog (functional; visual
  replica pending) · ~ T3.16 RRULE field + expansion (recurrence *editor*
  UI pending) · ~ T3.17 event reminders via shared queue + pop ·
~ T3.18 calendars sidebar (mini-month pending) · ☐ T3.19 drag interactions ·
~ T3.20 ICS import/export (subscriptions + printing pending) · ☐ T3.21 tasks sidebar.
Tasks: ~ T3.22 due-ordered list (tree view pending) · ~ T3.23 Kanban ·
~ T3.24 editor (priority/due/percent; custom fields pending) ·
☐ T3.25 smart lists + saved views · ☐ T3.26 floating + recurring ·
☐ T3.27 templates.
Notes: ~ T3.28 smart lists (All/Favorites) + cards (tree pending) ·
~ T3.29 Tiptap 3 editor (markdown mode pending) · ☐ T3.30 tabs/anchors/find ·
~ T3.31 favorites (reminder field pending) · ☐ T3.32 sticky windows (P1) ·
☐ T3.33 selection→task/event.
Contacts: ~ T3.34 letter bar + list + detail card · ~ T3.35 editor
(custom fields pending) · ☐ T3.36 email history · ☐ T3.37 groups ·
☐ T3.38 newsletters/print (P1).
Passwords: ~ T3.39 entries + masked reveal/copy (groups pending) ·
~ T3.40 generator (93-char, class-guaranteed) · ~ T3.41 strength heuristic
(zxcvbn-class P1; favicons pending) · ☐ T3.42 auto-type (P1).
Shared: ~ T3.43 Today dashboard (counts + module cards; widgets pending) ·
~ T3.44 import: ICS + VCF via file picker with count/warning report
(full wizard with column mapping + dry-run table pending) ·
~ T3.45 exports write to app-data/exports (dialogs + print pending) ·
☐ T3.46 private items · ☐ T3.47 shortcuts remapping.
Sync: ☐ T3.48 CalDAV/CardDAV profiles + engine (S55, P1).

## T4 Packaging (Gate 5)
- ☐ T4.1 .deb: desktop entry, original icon, MIME types (S58), Depends pinned.
- ☐ T4.2 Clean-container verification: install/launch/import/edit/export/remove.
- ☐ T4.3 README, CHANGELOG, LICENSE (Apache-2.0), final docs sync.

## UNRESOLVED / flagged
- Auto-Type reliability under Wayland (T3.42) — X11 first; alternatives: ydotool
  (uinput perms) or portal-free global shortcut; decide at Gate 3 with a spike.
- OAuth vendor sync (Google/M365/iCloud/Toodledo) — P2 until client IDs provided.
- Weather widget — interface scaffolded (T3.13), provider P2.
- Dark-theme/dialog parity beyond New Appointment — approximate from design language
  (F-004 unknowns); refine in Gate 3 with side-by-side review.
- AT-19 (60 fps with 100k rows) not yet measured — needs interactive session; see T2.8.
- Keyboard chords verified via XTEST under Xvfb (WM-less); real-desktop verification
  pending a run on the user's session.
- IMAP/POP3/SMTP (T1.10), rules (T1.11), reminder scheduler (T1.12) still open —
  reminder *count* already surfaces in the status bar from `reminder_queue`.
- Translation service (Mail) — P2, needs API choice.

## Traceability (P0 spec → task → test)
S1–S8→T2.1/2/3/4/5/6+T0.4 · S9→T0.3/T1.3 · S10→T1.1/T2.7 · S11→T1.1 · S12→T1.4 ·
S13→T1.5/T3.44 · S14→T1.13/T3.45 · S15–S27→T1.9/1.10/1.11/T3.1–3.12 ·
S28–S35→T1.7/T3.13–3.21 · S36–S41→T3.22–3.27 · S42–S45→T3.28–3.33 ·
S46–S49→T1.8/T3.34–3.38 · S50–S53→T3.39–3.42 · S54→T3.43 · S58→T4.1 ·
N1→AT-5/AT-13/AT-18/AT-19.
