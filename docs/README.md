# Smart PIM

A local-first, single-user personal information manager for Linux — Mail,
Calendar, Tasks, Notes, Contacts, Passwords — with a SQLCipher-encrypted
database (AES-256, argon2id key derivation).

Smart PIM is a clean-room, original implementation inspired by the public
behavior of established PIM applications. No third-party code, assets, or
data formats were copied or extracted (see `docs/rea/findings.md` and
`docs/ASSUMPTIONS.md` for the full clean-room methodology).

## Features

- **Encrypted everything**: one SQLCipher database; lock screen + idle
  auto-lock; closing the lock genuinely closes the connection.
- **Mail** (P0): IMAP/POP3 accounts (TLS/STARTTLS), incremental UID sync,
  Unified Inbox, threading, date-grouped list, reading pane with allowlist-
  sanitized HTML + per-sender remote-content blocking, compose with
  Send Later (outbox with missed-fire recovery), rules engine applied on
  ingest, full-text search with filters, convert message → task/note/event
  with backlinks.
- **Calendar**: Day/Week/Month grids, RRULE recurrence (computed, never
  stored, capped at 730 occurrences/query), reminders via the shared queue,
  ICS import/export (idempotent by UID).
- **Tasks**: due-ordered list, Kanban (To Do / In Progress / Done),
  percent-complete, version snapshots on every edit.
- **Notes**: Tiptap rich-text editor with autosave, favorites, full-text
  search.
- **Contacts**: letter bar, vCard 3.0/4.0 import/export, CSV import with
  column mapping + dry-run validation + atomic single-transaction commit.
- **Passwords**: masked entries, generator, strength heuristic.
- **Universal item model**: every item supports tags, symmetric Related
  Items links, private flag, version history with restore, soft-delete
  Trash with restore/purge, audit log, undo.

## Sync & migration

- **From EssentialPIM Pro**: export per module from EPIM (CSV / iCal /
  vCard / EML), then Smart PIM → File → Import. The wizard detects EPIM's
  CSV headers automatically. To move back, File → Export writes CSV, HTML,
  RTF, XML, Markdown (plus ICS/VCF/EML). The proprietary `.epim` file format
  is never parsed — EPIM's own export route is the supported path.
- **Wi-Fi device sync**: Settings → Sync → Devices. Start advertising, note
  the 6-digit code, then pair from the peer (the `sync_peer` CLI example
  demonstrates the client role; the Android companion app uses the same
  protocol: mDNS `_smartpim._tcp` → TLS → pairing token → batched
  guid-addressed changes, last-writer-wins, losers kept in item history).
  Sync refuses to run while the database is locked.
- **Cloud sync** (Settings → Sync → Cloud): CalDAV/CardDAV accounts pull
  and push per-item; WebDAV profiles sync the whole encrypted database file
  as a snapshot with ETag concurrency — if the remote changed since your
  last push, its copy is saved to Backups instead of being applied.
- **Backups**: automatic daily snapshot (14 kept; `backup.interval_min`
  setting, 0 disables) plus manual Backup Now / restore from Settings.
  Restoring re-locks the app and keeps a pre-restore snapshot.

## Build

Prerequisites: Rust 1.85+, Node 20+, `libwebkit2gtk-4.1-dev`,
`libgtk-3-dev`, `build-essential`, `pkg-config`.

```sh
# frontend
npm install
# run in dev
cargo tauri dev
# release build + .deb
cargo tauri build
```

Artifacts land in `target/release/bundle/deb/`.

## Install

```sh
sudo dpkg -i smart-pim_0.0.1_amd64.deb
```

The desktop entry appears as *Smart PIM* (Office). First launch asks for a
new database passphrase — there is no recovery, by design.

`--portable` stores the database beside the binary (USB mode).

## Tests

```sh
cargo test --workspace            # 85+ Rust tests (unit + integration)
npm test                          # frontend windowing/pagination math
# optional: live mail-server fixture tests (docker needed)
scripts/mailfixture/up.sh
cargo test -p smart-pim --test mailserver
```

## Layout

```
core/        data engine crate (store, repo, search, import/export, rules, ops)
src-tauri/   Tauri 2 shell (IPC commands, mail protocol clients, workers)
src/         Preact UI (modules/, shell/, design/, lib/)
docs/        spec, architecture, data model, feature matrix, REA findings
scripts/     mail-server test fixtures
```

## License

Apache-2.0. See `LICENSE`.
