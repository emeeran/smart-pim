# Smart PIM — Feature Matrix (Gate 0)

**Reference product:** EssentialPIM Pro (Astonsoft, https://www.epim.com) — clean-room
reimplementation of observable features from public docs only. No Astonsoft assets, icons,
schemas, strings, or branding.
**Target:** original desktop app, `.deb`, Debian/Ubuntu amd64, local-first single-user.

> **Scope correction (supersedes prompt.md's feature list).** prompt.md described a
> *Product* Information Management system (product master data, ETIM/ECLASS, BMEcat,
> variants, channel catalogs). The confirmed reference product is a *Personal* information
> manager. All product-data/PIM-specific items from prompt.md are marked **N/A** below and
> excluded from the MVP unless you say otherwise. See docs/ASSUMPTIONS.md A-1.

Priorities: **P0** = MVP (Gate 2–3), **P1** = second pass (Gate 4), **P2** = out of scope
unless you override. Per prompt.md, multi-user/server features are P2.

## Sources

- https://www.epim.com (home: editions, pricing, 26.5 release highlights)
- https://www.epim.com/features (module descriptions, security, portability, utilities)
- https://www.epim.com/features/explore-epim/epim-mail-vs-pro (full Mail-vs-Pro comparison tables)
- Module pages: /features/workspace/{mail,calendar,tasks,notes,contacts,passwords}
- /features/explore-epim/integrations (service × module sync matrix), /features/explore-epim/epim-cloud
- All fetched 2026-10-18; raw observations in docs/rea/findings.md (F-003+)

## General / platform

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Connected modules: Mail, Calendar, Tasks, Notes, Contacts, Passwords, Today, Trash | P0 | High | One app, shared sidebar; SQLite schema with per-item-type tables + unified item linking |
| Local-first storage, works offline | P0 | High | SQLite WAL in app-data dir |
| AES-256 database encryption | P0 | Medium | SQLCipher via rusqlite `bundled-sqlcipher`, or app-level SQLCipher + key from passphrase (KDF: argon2) |
| Auto-lock after idle/selected actions + manual lock | P0 | High | Idle detect via Tauri/DE APIs |
| Password protection for selected (private) items | P1 | Medium | Per-item private flag + hidden until unlock; "private excluded from sync/export" |
| Global search across all modules | P0 | High | SQLite FTS5 with per-module indexes |
| Global Sidebar for cross-module access | P0 | High | UI shell feature |
| Tags + centralized tag management (Pro) | P0 | High | tag table + junction tables |
| Cross-links / related items between any items (Pro) | P1 | High | Generic item_ref junction; UI picker |
| Item version history and restore (Pro) | P1 | Medium | append-only history table per item |
| Attachments manager incl. external file links (Pro) | P1 | High | hash-addressed media folder (matches prompt.md media design) |
| Automatic backup (Pro) | P0 | High | Scheduled VACUUM INTO / file copy with retention |
| Trash with filter-by-type/date and restore | P0 | High | Soft delete |
| Import/export CSV, ICS, VCF, EML, HTML, PDF, RTF, XML, MD | P0 (CSV/ICS/VCF), P1 (rest) | High | Non-destructive import: validate → report → single transaction (per prompt.md rules) |
| Portable mode (run from USB, no host traces) | P1 | High | `--portable` flag storing data dir beside binary |
| Light/dark theme, customizable views/fields/shortcuts | P1 | High | Prompt.md requirement too |
| EPIM Cloud sync + web access | P2 | Low | Proprietary hosted service; out of scope |
| Multi-user LAN / EPIM Server / EPIM Business (shared DB, access rights, real-time collab) | P2 | Low | Server feature — prompt.md excludes |
| Android/iOS apps, Wi-Fi device sync | P2 | Low | Out of scope for desktop .deb |

## Mail

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| IMAP + POP3 accounts, unlimited; Gmail/Outlook.com/iCloud OAuth | P0 (IMAP/POP3), P1 (OAuth flows) | High | Rust: `imap`, `pop3`, `lettre`, `rustls`; OAuth app registration needed per provider |
| Microsoft Exchange (EWS/ActiveSync) | P2 | Medium | Heavy protocol; defer |
| Unified Inbox, conversation/thread view | P0 | High | FTS + normalized message store |
| Compose: HTML + plain text, quick reply, attachments | P0 | High | lettre + editor component |
| Send later (scheduled send) | P1 | High | Outbox queue + scheduler |
| Message rules + adaptive spam filtering | P1 (rules), P2 (adaptive spam) | Medium | Rule engine straightforward; Bayesian spam is P2 |
| Advanced search/filter, flags/follow-ups, categories, pinned emails | P0 | High | FTS5 + flag columns |
| Convert email → task/event/note with backlink | P1 | High | Cross-link reuse |
| Templates, multiple signatures per identity, multiple identities/aliases | P1 | High | |
| PGP / PGP-MIME / S/MIME | P1 (PGP), P2 (S/MIME) | Medium | `rPGP`/sequoia; S/MIME needs X.509 stack |
| Attachment reminder | P1 | High | Heuristic |
| In-app message translation | P2 | Medium | External translation API dependency — out of scope unless you provide an API |
| Spell/grammar check, multilingual dictionaries | P1 | Medium | hunspell bindings; grammar check P2 |
| Auto-archiving/deletion policies | P1 | High | |
| Print messages | P1 | High | WebView print |

## Calendar

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Day, Week, Week Agenda, Month, Year, Agenda views + adjustable range | P0 | High | Custom-rendered grid in UI |
| Recurring events (RRULE) | P0 | High | `chrono` + ICS RRULE handling (`ical`/`rrule` crate) |
| Multiple reminders (popup/sound/email) | P0 (popup), P1 (email) | High | Desktop notifications via Tauri |
| Categories (color-coded), holiday categories | P0 | High | |
| Tasks sidebar inside Calendar; tasks shown in calendar | P0 | High | |
| Custom view bar: save filtered configurations | P1 | High | |
| Event templates | P1 | High | |
| Participant invitations (iMIP / iTIP send) | P1 | Medium | ICS ATTENDEE + email send |
| iCal (.ics) import/export + internet calendar subscriptions | P0 (import/export), P1 (subscriptions) | High | Subscriptions = scheduled HTTP fetch of ICS |
| Zoom/Teams/Google Meet links | P1 (Meet link gen), P2 (Zoom/Teams APIs) | Medium | Meet: no API needed for basic link; Zoom/Teams need OAuth apps |
| Weather forecast in Calendar | P2 | Medium | External API dependency |
| Working hours, week numbering, additional-events indicator | P1 | High | |
| Customizable printing | P1 | High | print-to-PDF via WebView |
| Google/M365/iCloud/CalDAV/Nextcloud calendar sync | P1 (CalDAV/Nextcloud), P2 (Google/M365/iCloud) | Medium | `http`+XML CalDAV client feasible; vendor sync is a large P2 |

## Tasks

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Hierarchical tasks/subtasks | P0 | High | |
| Tree, Plain, Kanban views (drag between columns) | P0 | High | Virtualized lists |
| Smart Lists (Important, Flagged, Completed, All) | P0 | High | Saved filters |
| Floating tasks (roll forward until done) | P1 | High | |
| Recurring task patterns | P0 | High | Same RRULE engine |
| Multiple reminders | P0 | High | |
| Custom fields (text/date/dropdown/yes-no/checkbox) | P1 | High | EAV pattern (matches prompt.md attribute design) |
| Task templates | P1 | High | |
| Assignments/participant invitations | P2 | Low | Assumes server/collab context |
| Search/filter, date-based grouping | P0 | High | |
| Google Tasks / Microsoft To Do / Toodledo sync | P2 | Medium | Vendor API integrations; defer |

## Notes

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Multilevel note tree | P0 | High | |
| Markdown + rich-text editing, tables, images, drawings, links | P0 (MD + basic rich text), P1 (drawings) | Medium | Tiptap/ProseMirror or Milkdown in webview; drawing canvas P1 |
| Multi-tab notes | P1 | High | |
| Anchors / in-note navigation | P1 | High | |
| Find & replace across notes | P0 | High | FTS5 |
| Tags, note reminders | P0 | High | |
| Convert selected text → task/event | P1 | High | |
| Sticky Notes on desktop + groups (Pro) | P1 | Medium | Frameless always-on-top Tauri windows |
| Custom text styles, rulers, line numbers, formatted titles | P1 | High | |
| Google Drive / OneDrive / Toodledo sync | P2 | Medium | Vendor APIs; defer |

## Contacts

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Contact CRUD, multi-level groups, multi-group assignment, favorites | P0 | High | |
| vCard (.vcf) import/export; CSV import with column mapping + validation report | P0 | High | Matches prompt.md import-wizard design |
| Custom fields (text/date/checkbox/list) | P1 | High | EAV |
| Advanced search/sort/filter, letter bar, custom layouts | P0 | High | |
| Per-contact email history | P1 | High | Join on addresses |
| Schedule meeting from contact card | P1 | High | |
| Click-to-dial | P2 | Low | DE-dependent (xdg handlers); niche on Linux |
| Newsletters / personalized bulk email (merge fields) | P1 | Medium | SMTP + template merge |
| Label & envelope design/printing | P1 | Medium | HTML/CSS print templates |
| Google/Outlook/iCloud/CardDAV/Nextcloud sync | P1 (CardDAV/Nextcloud), P2 (vendors) | Medium | |

## Passwords

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Encrypted entry store, tree groups/subgroups | P0 | High | Same SQLCipher DB |
| Password generator (length, character sets) | P0 | High | `rand` |
| Custom fields + masked values | P1 | High | |
| Website icon fetching | P1 | High | Favicon fetch + hash storage |
| Auto-type global hotkey | P1 | Medium | Linux: `enigo`/wtype under Wayland caveats — flag at Gate 1 |
| Search/filter, related-item links | P0 | High | |
| Browser autofill | P2 | Low | Requires browser extension; out of scope |

## Today view

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Cross-module daily overview (events, tasks, birthdays, recent mail) | P0 | High | Aggregated queries |
| Filters and custom date ranges | P1 | High | |

## Engine / quality (from prompt.md engineering rules)

| Feature | Pri | Feasibility | Notes |
|---|---|---|---|
| Migrations, repository layer, audit/undo log | P0 | High | Undo via inverse-op log |
| Non-destructive import: dry-run validation report → single transaction | P0 | High | |
| Large-dataset tests (100k items) + FTS perf | P0 | High | Applies to mail corpus instead of products |
| .deb packaging, desktop entry, icon, MIME associations (.ics/.vcf/.csv/.eml) | P0 | High | |

## N/A — in prompt.md but absent from the reference product

| prompt.md item | Status | Reason |
|---|---|---|
| Product master data / product grid | N/A | Not a PIM (see A-1) |
| ETIM / ECLASS classification | N/A | Not a PIM |
| BMEcat import/export; .bmecat MIME | N/A | Not a PIM; replaced by ICS/VCF/EML MIME handling |
| Variants / bundles | N/A | No equivalent |
| Completeness scoring | N/A | No equivalent (closest: task/notes metadata, not scored) |
| Channel/catalog output, web-shop feed | N/A | No equivalent |
| Multi-language per-field values | N/A | App UI language only (translation of content is P2 Mail feature) |
| printpdf catalog output | N/A | Replaced by HTML/print-to-PDF for calendar/contacts/messages |

## Shipped in the full-clone slice (2026-10-19, docs/epim_clone_plan.md)

| Feature | Status | Notes |
|---|---|---|
| Wi-Fi device sync (desktop-to-desktop) | Done | schema v4 guid + change_log; mDNS + TLS + pairing; `pfcore::sync` LWW engine; `examples/sync_peer.rs` CLI peer |
| Android companion app | Protocol ready | wire contract in `src-tauri/src/wifisync.rs`; app itself is a separate project |
| Cloud sync: CalDAV/CardDAV | Done | `src-tauri/src/dav.rs` + `dav_sync` command |
| Cloud sync: whole-DB file (WebDAV; Dropbox/Drive/Box via DAV gateways) | Done | encrypted `VACUUM INTO` snapshot + If-Match ETag; remote-changed saves copy to backups instead of clobbering |
| EPIM interchange: import presets | Done | EPIM CSV header detection in `src/lib/epim_csv.ts` |
| EPIM interchange: export HTML/RTF/XML/MD | Done | `core::export` + `export_module` command (with existing CSV/ICS/VCF/EML) |
| Automatic backup + restore | Done | daily `VACUUM INTO` snapshot (retention 14) in the worker tick; list/restore in Settings |
| Shell parity: smart lists, mini month, letter rails, mail cards, kanban counts, notes breadcrumb | Done | see F-004 observations; per-module tree sections + list-panel patterns |
| Appointment dialog (recurrence/reminders/completion/participants UI) | Done | FormDialog + EventDialog; participants/type not persisted (no columns) |
| Settings UI | Done | appearance, security, backups, Wi-Fi devices, DAV profiles |
| OAuth mail (Gmail/Outlook/iCloud), PGP/S-MIME, adaptive spam | Not started | needs per-provider app registration; next phase |
| Sticky notes, auto-type, newsletters, labels/envelopes, click-to-dial | Not started | P2 per plan §5 |
| AI integration | Done | docs/ai_integ_plan.md implemented: providers, job queue, all §3 features preview-then-apply; embeddings encrypted in-DB; passwords excluded |

## Shipped in the Gate 5 slice (rebrand + shell + Diary)

| Feature | Status | Notes |
|---|---|---|
| Diary module (one entry per local day, streak, mood) | Done | schema v2; FTS5; trash/versions/links |
| Grey theme + live system-theme following | Done | token-coverage test prevents partial themes |
| Labelled navigation tree (EfficientPIM-style) | Done | count badges, collapse toggle |
| App toolbar under menu bar | Done | modules register via useToolbar slot |
| Related Items tabs with link/unlink | Done | Notes/Contacts/Passwords; commands item_links/item_link/item_unlink |
| Calendar Year + Agenda views | Done | single events_window call per range; view persisted |
| Task tree view + parent assignment | Done | schema v3 item.parent_id; cycle-safe reparenting |
| AI integration plan | Planned | docs/ai_integ_plan.md (planning only, no code) |
