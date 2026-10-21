# Smart PIM — Architecture (Gate 1)

## Overview

```
┌──────────────────────────── Tauri 2 (system WebKitGTK) ───────────────────────────┐
│  UI: TypeScript + Vite, no framework lock-in kept minimal (Preact + signals)      │
│  Shell: rail / toolbar / tree / list / detail components (F-004 layout)           │
│  Rich text editor: Tiptap (ProseMirror) shared by Mail compose, Notes, item notes │
└──────────────▲───────────────────────────────────────────────────▲────────────────┘
               │ Tauri IPC (typed commands, events)                │ direct read for
               ▼                                                   ▼ virtualized lists
┌──────────────────────────── Rust "core" crate ────────────────────────────────────┐
│ store/    SQLCipher pool, migrations, repositories (per module)                   │
│ model/    serde types shared with UI via ts-rs generated bindings                 │
│ mail/     imap · async-imap IDLE · pop3 · lettre SMTP · mail-parser · mail-builder│
│ ical/     ical + rrule (import/export/subscription, recurrence expansion)         │
│ contacts/ vcard I/O, CSV mapping                                                  │
│ crypto/   argon2 KDF → SQLCipher key; rPGP (P1)                                   │
│ search/   FTS5 indexes per module + unified global search                         │
│ ops/      audit log, undo log, backup scheduler, reminder scheduler, rules engine │
│ sync/     CalDAV/CardDAV client (P1), vendor adapters (P2)                        │
└──────────────▲───────────────────────────────────────────────────────────────────┘
               ▼
  SQLCipher DB (AES-256, WAL)        media/ attachments store (SHA-256 addressed)
```

## Decisions

| Decision | Choice | Why |
|---|---|---|
| UI framework | Preact + signals, custom CSS design tokens | EPIM's UI is bespoke three-pane; a heavy framework adds nothing; virtualized lists via `@tanstack/virtual` (works with Preact) |
| Editor | Tiptap 2 | One editor across Notes/Mail/item-notes replicates the shared "Notes | Related Items" pattern |
| DB | rusqlite + `bundled-sqlcipher`, WAL | AES-256 per spec S9; pin versions in Cargo.lock |
| IPC | Typed commands + `ts-rs` generated TS types; lists paged by keyset | Keeps 100k-row lists off the JS heap except visible windows |
| Recurrence | `rrule` crate, expansion capped at 730 occurrences/query | ICS-compatible RRULE (S29) |
| Notifications | `notify-rust` + Tauri tray | S27/S30 |
| Printing | HTML print templates → WebKit print-to-PDF | S14 |
| Markdown | `pulldown-cmark` (render) + Tiptap markdown extension (edit) | S43 |

## Process model

- Single Tauri process; Rust work on tokio runtime (mail sync, schedulers); UI never
  blocks on IPC > 50 ms — long ops (import, initial mail fetch) stream progress events.
- Reminder scheduler: in-process tokio task, re-armed on boot from DB (missed reminders
  fire once, flagged).
- Backup: `VACUUM INTO` to `backups/YYYY-MM-DD-HHMM.db` + SHA-256 sidecar, retention N.

## Data flow example (incoming mail)

```
tokio IDLE/poll → fetch UIDs → mail-parser → rules engine → INSERT messages + FTS
→ emit 'mail:new' event → UI updates list/unread badge/Today → notification
```

## Keyboard map (S59, replicating EPIM-equivalents)

Ctrl+1..8 modules · Ctrl+Q global search · Ctrl+N new item · Delete → trash ·
Ctrl+Z/Ctrl+Shift+Z undo/redo · Ctrl+Enter quick reply · F5 / Ctrl+R send-receive ·
Ctrl+E reply · Ctrl+Shift+E reply all · Ctrl+F in-view find · Ctrl+K link item ·
Ctrl+Shift+L lock now · Space triage (mail) · J/K navigate lists · Ctrl+P print.

## Error & audit

- Every mutation: audit row (item, action, before/after hash) + undo inverse op.
- Import: dry-run report object returned before commit (S13); commit is one transaction.
- All command errors are typed enums serialized to the UI (no panics across IPC).

## Testing strategy

- `core`: unit tests per repository + integration tests with fixtures (ICS/VCF/CSV/EML
  corpora incl. malformed; 100k message and 50k task generators for perf gates).
- UI: Playwright over `tauri-driver` for shell flows (module switching, list virtualization,
  quick reply); visual snapshots for theme tokens.
- Packaging: containerized `dpkg -i` smoke test (Gate 5).

## Directory layout

```
src-tauri/          Tauri shell, commands, tray, single-instance
  core/             Rust crate (store, model, mail, ical, contacts, crypto, search, ops, sync)
src/                UI (shell/, modules/{today,mail,calendar,tasks,notes,contacts,passwords},
                    components/, design/{tokens.css,icons.ts} (original, code-drawn))
docs/               this + feature-matrix, ASSUMPTIONS, rea/, data-model, tasks, spec
```