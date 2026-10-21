# Smart PIM — Full EPIM Pro Clone Plan

**Goal:** reach functional + UI/UX parity with EssentialPIM Pro (Astonsoft, https://www.epim.com) — every module, interaction pattern and workflow — **except brand name and logo**, plus two expansions EPIM only partially covers: **cloud sync** (Google Drive, Dropbox, Box, NAS, Nextcloud/WebDAV) and **Wi-Fi sync with an Android companion app**. Full **import/export interchange with EPIM Pro** and an **AI integration scope** are included.

**Clean-room guardrails (unchanged, from prompt.md):** layout, workflows, and interaction patterns are re-implemented from observation of public docs/screenshots only. No Astonsoft binaries, assets, icons, fonts, schemas, or strings. The proprietary `.epim` database format is parsed or reverse-engineered; interchange with EPIM Pro goes through EPIM's own export formats (CSV/ICS/VCF/EML/HTML/RTF/TXT). Sources and raw observations:
docs/rea/findings.md F-003/F-004/F-005.

**Scope override (2026-10-19):** docs/feature-matrix.md marked "Android/iOS apps, Wi-Fi device sync" and "EPIM Cloud" as P2-out-of-scope. The user has explicitly pulled them into scope. This document supersedes those rows.

---

## 1. Current state (already shipped, feature/gate-4)

Modules: Today, Mail (IMAP/POP3, threading, unified inbox, rules, send-later, search), Calendar (day/week/month/year/agenda, recurrence, subscriptions), Tasks (list/tree/kanban, hierarchy, reminders), Notes (Tiptap rich text, tree, tabs), Diary, Contacts (vCard/CSV), Passwords, Trash. Shell: labelled nav tree + badges, per-module toolbar, three-pane layouts, "Notes | Related Items" detail tabs, status bar, 4 themes (light/grey/dark/system). 

Core: SQLCipher + argon2id, universal `item` model with `updated_at`/`deleted_at`/ `private`/`list_id`, tags, item links, version history, audit/undo, FTS5 per module, `custom_field`, `template`, `identity`, `outbox`, `subscription`, and a **`sync_profile` table** (`kind`, `url`, `direction`, `interval_min`, `last_sync`, `last_token`) — already the right anchor for sync work. Details: docs/feature-matrix.md, docs/data-model.md.

## 2. Gap analysis — what a "clone exactly" still needs

| Area        | Gap vs EPIM Pro                                                                                                                                                                                                                                         | Size |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- |
| Shell/UI    | Smart Lists section in tree panel; account/list section with colored calendars; "Add List/Calendar/Group" links; mini month in Calendar tree; letter bar (Contacts/Passwords); date-grouped mail cards w/ avatars; status-bar parity; dark-theme polish | M    |
| Dialogs     | New Appointment dialog (recurrence builder, reminders, participants, completion slider); New/Edit dialogs for all kinds; Settings UI                                                                                                                    | L    |
| Mail        | OAuth (Gmail/Outlook/iCloud), PGP/S-MIME, adaptive spam, templates w/ merge, multi-identity/signatures, attachment reminder, archiving policies, pinned emails, translation (AI), print                                                                 | L    |
| Calendar    | Saved custom views (view bar), event templates, participant invitations (iMIP), Google Meet link gen, weather (AI/API), working hours, printing                                                                                                         | M    |
| Tasks       | Floating tasks, recurring tasks, custom fields UI, task templates, Smart Lists                                                                                                                                                                          | M    |
| Notes       | Sticky notes (desktop widgets), anchors, find & replace UI, convert selection→task/event, rulers/line-number options                                                                                                                                    | M    |
| Contacts    | Per-contact email history, newsletters w/ merge fields, labels/envelopes print, meeting scheduling, click-to-dial                                                                                                                                       | M    |
| Passwords   | Website icon fetch, auto-type global hotkey, password health/report                                                                                                                                                                                     | S    |
| General     | Attachments manager UI, private-item lock UI, automatic backup scheduler, portable mode (`--portable`), custom fields EAV UI, duplicate finder                                                                                                          | L    |
| Interchange | EPIM Pro import (CSV layouts per module, ICS, VCF, EML); export: HTML/PDF/RTF/XML/MD in addition to existing CSV/ICS/VCF/EML                                                                                                                            | M    |
| Sync        | Whole workstream — see §4                                                                                                                                                                                                                               | XL   |
| AI          | Planned, not built — docs/ai_integ_plan.md                                                                                                                                                                                                              | L    |

Sizes: S ≤ 1 day, M ≤ 1 week, L ≤ 2–3 weeks, XL > 3 weeks (single-dev estimates).

## 3. Workstream A — UI/UX parity (from F-004 layout observations)

Re-style every module to the observed EPIM patterns. All original code against the existing
token system (`src/design/tokens.css`); extend the token-coverage test when new tokens land.

1. **Tree panel upgrade (all modules):** extend `src/shell/NavTree.tsx`'s per-module
   secondary tree with: Smart Lists section (All/Important/Flagged/Completed with counts —
   Tasks; All/Favorites/Recently Viewed/Recently Modified — Notes); account section
   ("Local <Kind>" hierarchies with colored dots + counts); trailing "+ Add List/Calendar/
   Group"; mini month picker in Calendar. Backed by the existing `list_id`/`category`
   columns — no schema change.
2. **List panel patterns:** mail date-grouped cards (TODAY/YESTERDAY/LAST WEEK headers,
   avatar, 2-line preview, colored accent bar, unread bold — headers already exist in
   `src/lib/list-model.ts`); contacts/passwords A–Z letter bar; tasks table with inline
   "Add a task" row; kanban column counts + per-column "+" + card progress.
3. **Detail panes:** mail reading pane with pinned quick-reply box; notes breadcrumb +
   history arrows + word count/zoom status; contacts header card (avatar, favorite star,
   action row) with Contact/Phone/Internet/Additional Fields/Membership/Notes sections;
   passwords Entry tab with masked field + eye toggle + Generate + copy buttons (partially
   shipped — bring to full pattern).
4. **Dialogs:** one shared `FormDialog` scaffold, then per-kind dialogs matching the
   observed New Appointment pattern: type dropdown, Home | Related Items tabs, recurrence
   banner + "Repeat every N <unit>" + custom RRULE gear + Ends dropdown, "+ Add Reminder"
   presets, participants autocomplete, completion slider, footer Delete/Save & New/OK/Cancel.
5. **Status bar + toolbar parity:** "Items: N", duration (calendar), unread badge, reminder
   bell, word count + zoom slider (notes), globe (far right); toolbar filter + layout
   buttons.
6. **Settings UI:** full preferences dialog (themes, shortcuts, backup, auto-lock, sync,
   mail identities, rules) — currently scattered.
7. **Today view parity:** columns CALENDAR / TASKS / MESSAGES / ANNIVERSARIES with
   weather + colored chips, dismissible banner.

**Verify:** side-by-side walkthrough against `docs/rea/ui-ref/` screenshots per module;
theme-coverage + a11y tests stay green.

## 4. Workstream D — Sync (the big one)

### D1 — Wi-Fi sync with an Android companion app (EPIM's "PC Sync" pattern)

Observed EPIM behavior: the Android app scans the LAN for PCs running desktop EPIM;
sync is initiated from the phone ("PC Sync"), with a scan-timeout setting and a manual
"Connect manually" fallback when discovery fails (forum-confirmed). Smart PIM's
equivalent, built on the existing `sync_profile` table:

- **Desktop side (src-tauri):** a `sync` feature in the shell process.
  - Discovery: mDNS/DNS-SD advertise `_smartpim._tcp` on the LAN (crate: `mdns-sd`),
    plus manual "pair by IP:port" fallback (matches EPIM's Connect-manually escape hatch).
  - Transport: local TCP listener, TLS with a self-signed cert generated per install;
    first-connect **pairing** = 6-digit code shown on desktop, entered on phone; token
    stored in `sync_profile.last_token` (never plaintext on the wire after pairing).
  - Protocol: length-prefixed JSON (serde) — `HELLO` → `AUTH` → `PULL since cursor` /
    `PUSH batch` → `ACK` → `BYE`. Batched, resumable, max-batch-size limited.
- **Change tracking (core, schema V4):** the piece everything else reuses.
  - `change_log(id INTEGER PK, item_id, op TEXT, at TEXT, device_id)` appended in the same
    transaction as every repo write (hooks in `core/src/repo.rs` where audit_log is
    already written — same call sites).
  - `device(id TEXT PK, name, platform, paired_at, last_sync)` registry.
  - Tombstones already exist (`item.deleted_at`) so deletes replicate; `item.updated_at`
    + `change_log` cursor per device give incremental pull. No vector clocks for v1 —
      **last-writer-wins per field group** with a documented conflict policy (EPIM's own
      Wi-Fi sync offers no merge UI); conflicts where both sides changed the same item are
      surfaced in a "Sync report" dialog, loser version saved to `item_version` (already
      exists) — nothing is ever silently destroyed.
  - Mail is desktop-authoritative: phones pull read/unread + folder moves, never push.
- **Android app (separate repo/project, Kotlin + Compose, Rust core via JNI or a thin
  Kotlin store):** modules Calendar, Tasks, Notes, Contacts, Passwords, Diary (EPIM's
  Android app covers cal/tasks/notes/contacts/passwords + widgets). Scope the app itself
  as its own mini-project: read/write local store, widgets later. **Desktop-led plan: build
  and test the protocol with a CLI simulator first (`src-tauri/examples/sync_peer.rs`),
  ship the app as phase D2.**

### D2 — Cloud protocol sync (per-module, EPIM-style integrations)

Reuse `sync_profile` rows with `kind` per protocol. Priority order (feasibility from
feature-matrix.md):

1. **CalDAV (calendar+tasks VTODO) and CardDAV (contacts)** — plain HTTP+XML; covers
   Nextcloud/ownCloud/Radicale and most self-hosted setups. Crates: `http`, `quick-xml`;
   test against a local Radicale container.
2. **ICS subscriptions (read)** — already shipped (`subscription` table); add scheduling
   via the existing mail-worker tick.
3. **Google** — Calendar API, Tasks API, People API (contacts); OAuth desktop flow with
   a registered client; notes/passwords-via-Drive only as encrypted blobs (see D3).
4. **File-DB cloud sync (Dropbox, Google Drive, Box, OneDrive, WebDAV)** — EPIM's own
   model ("move the database to Dropbox"). Smart PIM equivalent: ** encrypted-DB-as-object
   sync**: the SQLCipher file is already encrypted at rest; sync it as an opaque object to
   any provider with a generic WebDAV/OAuth-storage client. Requirements:
   - WAL mode must be checkpointed + a consistent copy taken (`VACUUM INTO` to a temp
     file, upload, keep `.prev` for rollback) — never live-sync the hot WAL file.
   - Single-writer lease file + change detection (ETag/revision) + "DB in use elsewhere"
     dialog; automatic local backup before every pull (retention in settings).
   - This deliberately reuses the automatic-backup machinery (P0 general gap) — build
     backup first, file-cloud sync is backup + upload.

**Explicitly out of scope (unchanged):** EPIM Cloud (proprietary hosted service), EPIM
Server / multi-user LAN (prompt.md excludes multi-user), Exchange/ActiveSync (P2).

### D3 — Interchange with EPIM Pro (import/export)

- **From EPIM Pro → Smart PIM:** user runs EPIM's File → Export per module (CSV with
  EPIM's per-module column layouts, ICS for calendar, VCF for contacts, EML for mail,
  HTML/RTF/TXT for notes) → Smart PIM's existing ImportWizard (non-destructive
  validate→report→single transaction) gains **EPIM-flavored CSV mapping presets** per
  module. Document the recipe in docs/README.md ("Migrating from EssentialPIM").
- **To EPIM Pro:** Smart PIM already exports CSV/ICS/VCF/EML; add HTML, PDF (print-to-PDF),
  RTF, XML, MD writers in `core/src/export/` so every module round-trips both ways.
- **Never** parse `.epim` directly (encrypted, proprietary, clean-room guardrail). The
  export route is also what EPIM officially supports for moving data.

## 5. Workstream B — Module feature parity (prioritized)

P1 (next after UI parity): OAuth mail (Gmail/Outlook/iCloud), PGP via `rPGP`, mail
templates + merge, multi-identity/signatures (tables exist), attachment reminder,
archiving policies; recurring tasks + floating tasks; calendar saved views + event
templates + iMIP invitations; notes anchors + convert-selection→task/event; attachments
manager UI; private-item lock UI; automatic backup scheduler; duplicate finder.
P2: S/MIME, adaptive spam filter, weather, click-to-dial, labels/envelopes, newsletters,
sticky notes, auto-type, browser-independent extras.

## 6. Workstream E — AI integration

Planning doc exists and is approved scope: **docs/ai_integ_plan.md** (local-first,
Ollama-default, BYO-key cloud opt-in, per-module features, "AI never writes without a diff
preview" via item_version + undo, passwords module fully excluded). Build order when
funded: ai_status/ai_run/ai_cancel plumbing → mail triage digests + semantic hybrid search
(sqlite-vec) → NL capture → notes/diary summarize.

## 7. Schema work required (V4+)

- **V4 (sync):** `change_log`, `device` (§D1). `sync_profile` already exists.
- **V5 (parity):** floating/recurring-task columns on `task`; saved-view table
  (`saved_view`); sticky-note table; custom-field instances for tasks (EAV exists for
  contacts — generalize).
- Every migration follows the established recipe (docs/data-model.md, forward-only,
  `apply_all`, populated-DB test + FK probe).

## 8. Roadmap

| Phase                          | Contents                                                                                      | Exit criteria                                        |
| ------------------------------ | --------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| **0. Foundations**             | change_log + device schema (V4) + repo hooks; automatic backup scheduler; FormDialog scaffold | unit tests green; backup/restore round-trip test     |
| **1. UI parity I**             | tree panels, list-panel patterns, detail panes, status bar (§A1–A5)                           | side-by-side vs ui-ref per module                    |
| **2. Interchange**             | EPIM CSV presets, HTML/PDF/RTF/XML/MD export, migration doc                                   | round-trip tests; sample EPIM exports import cleanly |
| **3. Sync core**               | Wi-Fi protocol + pairing + CLI peer simulator; conflict policy + sync report                  | two desktop instances sync; kill/resume tests        |
| **4. UI parity II + features** | dialogs (§A4), settings, Today parity, P1 features (§5)                                       | feature-matrix rows flipped to Done                  |
| **5. Cloud sync**              | CalDAV/CardDAV; encrypted file-DB sync to WebDAV/Dropbox/Drive/Box                            | Radicale container test; provider sandbox tests      |
| **6. Android app**             | companion app (separate repo) on the D1 protocol; widgets after                               | end-to-end phone↔desktop sync                        |
| **7. AI**                      | per docs/ai_integ_plan.md                                                                     | per that plan                                        |

Order rationale: change tracking (0) unblocks both sync phases; UI parity is chunked so
interchange and sync — the user's explicit new requirements — land early (2–3) rather than
after a long UI grind.

## 9. Risks & unknowns

1. **Google/Microsoft OAuth** requires registering apps per provider — user action
   needed; without it, OAuth mail/calendar ships "bring your own client id".
2. **Whole-DB file sync** on consumer cloud drives is corruption-prone (no leases on
   Drive/Box); mitigated by VACUUM INTO copies + lease file + mandatory pre-pull backup.
   Documented residual risk.
3. **Android app** is a second product (Kotlin, store signing, widgets) — biggest single
   estimate; the protocol-first approach keeps desktop shippable without it.
4. **Wi-Fi sync security:** LAN discovery + TLS + pairing token; never advertise on
   non-private networks (detect via NetworkManager interface type); auto-lock must engage
   during sync for locked DBs (sync runs in the shell with the DB open — reject sync when
   locked).
5. **Auto-type on Linux** is Wayland-fragile (feature-matrix note stands).
6. Dark-theme/settings UI layouts have no public reference (F-004 unknowns) — approximate
   in the same design language, flagged for refinement.

## 10. Verification

- Per-phase: `cargo test --workspace`, `clippy -D warnings`, `cargo fmt --check`,
  `npm test`, `npm run build`, deb build (established Gate 5 loop).
- Sync: property tests on change_log apply/idempotency (replay = no-op); two-instance
  integration test via the CLI peer; kill-during-sync resume test; wrong-token pairing
  rejected.
- Interchange: golden-file round-trips (export → import → export equals), sample EPIM
  exports (user-provided fixtures) as integration fixtures.
- UI: token-coverage test extension; keyboard-shortcut inventory test.

---

### Sources (public docs only; observations logged in docs/rea/findings.md)

- https://www.epim.com , https://www.epim.com/features
- https://www.epim.com/features/explore-epim/epim-mail-vs-pro
- https://www.epim.com/features/explore-epim/integrations (service × module sync matrix)
- https://www.epim.com/features/explore-epim/epim-cloud
- Module pages: epim.com/features/workspace/{mail,calendar,tasks,notes,contacts,passwords}
- Wi-Fi sync behavior: epim.com/forum/viewtopic.php?t=14206 (scan timeout / Connect
  manually), m.essentialpim.com/forum/viewtopic.php?t=36206
- Whole-DB Dropbox model: epim.com/blog-and-news/blog/an-overview-of-backing-up-and-syncing-epim-data
- UI layout observations: docs/rea/ui-ref/ (12 public screenshots, observation only)
