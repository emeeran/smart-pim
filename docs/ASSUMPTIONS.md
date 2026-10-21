# Smart PIM — Assumptions (Gate 0)

Each assumption is flagged for your correction. Nothing below is fixed until you approve.

## A-1 — Reference product identity (RESOLVED, with scope correction)

- **You confirmed the target is the product at https://www.epim.com**: EssentialPIM /
  EPIM Pro by Astonsoft — a *personal* information manager (Mail, Calendar, Tasks, Notes,
  Contacts, Passwords).
- **Consequence:** prompt.md's feature list was written for a *product* PIM (Viamedici EPIM:
  product master data, ETIM/ECLASS, BMEcat, variants, completeness scoring, channel feeds).
  **None of those features exist in EssentialPIM.** They are marked N/A in
  docs/feature-matrix.md and excluded from the MVP. The project name `EPIM-Lnx`
  (EPIM for Linux — EssentialPIM has no Linux version) is consistent with this reading.
- If you actually wanted the Viamedici-style PIM, say so now and the matrix is redone.

## A-2 — MVP definition (replaces prompt.md's PIM MVP)

MVP = P0 rows of docs/feature-matrix.md: encrypted local SQLite store; Mail (IMAP/POP3,
unified inbox, threads, compose, search); Calendar (Day/Week/Month/Year/Agenda, recurrence,
reminders, categories, ICS import/export); Tasks (tree/kanban, recurrence, smart lists);
Notes (tree, Markdown + rich text, FTS); Contacts (groups, VCF/CSV import-export);
Passwords (generator, groups); Today view; global search (FTS5); tags; trash; automatic
backup; undo/audit log; import wizard with validation report.

## A-3 — Stack (prompt.md's assumptions, adjusted)

- **Kept:** Tauri 2 + TypeScript/Vite UI; Rust `core` crate; rusqlite (WAL + migrations +
  FTS5); serde; csv; sha2 (attachment dedupe); `cargo tauri build --bundles deb`.
- **Changed/added because of A-1:**
  - AES-256 encryption → **SQLCipher** (rusqlite `bundled-sqlcipher` feature), key derived
    from a master passphrase (argon2). Replaces prompt.md's plain-SQLite assumption.
  - Email → `imap`, `pop3` (or `async-imap`), `lettre` (SMTP), `mail-parser`, `rustls`.
  - PGP → `rPGP` (pure Rust; P1).
  - ICS/vCard → `ical` / `rrule` / `vcard` crates. **Replaces** prompt.md's quick-xml
    BMEcat/ETIM pipeline, calamine/rust_xlsxwriter, printpdf (XLSX/BMEcat/PDF-catalog are
    N/A per A-1; printing uses WebView print-to-PDF).
- **Flagged risk:** SQLCipher + imap/lettre crate versions must be pinned and
  Debian-buildable; verified at Gate 2.
- **Not adopted:** FastAPI sidecar (prompt.md alternative) — Rust-only unless you object.

## A-4 — Platform/runtime assumptions

- Linux amd64; **X11 first, Wayland best-effort**. Auto-Type (global hotkey + synthetic
  input) and Sticky Notes depend on DE specifics (wtype/ydotool under Wayland). Flagged for
  Gate 1 — may be P1-with-caveats.
- Desktop notifications via `notify-rust` (org.freedesktop.Notifications).
- No WebView-injected proprietary code: system WebKitGTK per Tauri defaults.
- OAuth-dependent features (Gmail/Outlook.com/iCloud account setup, Zoom/Teams, Google
  Drive/Toodledo sync) need API client IDs you must register — all such vendor sync is P2
  by default. CalDAV/CardDAV (Nextcloud/ownCloud/Radicale) is P1 because it needs no
  proprietary API keys.

## A-5 — Legal / clean-room (AMENDED by user directive, 2026-10-18)

- Public docs/datasheets/demos only; no Astonsoft binaries, installers, ASARs, decompilation,
  or DB-format extraction. Original name "Smart PIM".
- **Amendment (user):** UI/UX and feature set are replicated **exactly**, except brand name
  and icon. Implementation rule that keeps this clean-room: layout, workflows, interaction
  patterns, and visual structure are re-implemented as **original code** (our own CSS,
  icons drawn/rendered by us, original strings where the text is creative copy; functional
  labels like "New Task", "Filter", "Layout" are not protectable and are replicated).
  No Astonsoft image/font/asset files are copied, embedded, or redistributed.
- Reference: docs/rea/ui-ref/ screenshots (observation only) + F-004 layout observations.

## A-5b — Priority amendments (user directive, 2026-10-18)

- "Replicate its features" ⇒ **Mail is P0** (open question 3 answered).
- CalDAV/CardDAV stays **P1**; Google/M365/iCloud/Toodledo vendor sync stays **P2** — not a
  scope decision but a dependency one: each needs registered OAuth client IDs and vendor
  review. Say the word and any of them moves up.

## A-6 — Out of scope unless you override

EPIM Cloud (hosted service + web access), EPIM Server/Business (multi-user LAN, access
rights, real-time collaboration), Android/iOS apps, Wi-Fi device sync, adaptive spam ML,
in-app translation (needs paid API), browser autofill extension, Exchange EWS, weather
integration (P2; needs an API key — will scaffold a provider interface), click-to-dial.

## Open questions for you (answer with your Gate 0 approval)

1. A-1 scope correction OK? (Personal organizer, not product PIM.)
2. A-3 stack changes OK (SQLCipher, mail crates, ICS/vCard instead of BMEcat/XLSX)?
3. Is **Mail** truly P0? An email client is the largest single work item in the MVP. If
   dropped to P1, MVP becomes calendar/tasks/notes/contacts/passwords — considerably
   faster to Gate 3. Your call.
4. CalDAV/CardDAV sync priority (currently P1)?
