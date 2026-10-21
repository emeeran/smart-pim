# Smart PIM — Specification (Gate 1)

Faithful functional + UI/UX replication of EssentialPIM Pro (public behavior only), as an
original Linux desktop app. Brand name and icon are original; everything else follows
docs/feature-matrix.md and the layout evidence in docs/rea/findings.md F-004.

Normative language: MUST/SHOULD per RFC 2119. "EPIM" below means the reference product's
observed behavior, not its code.

## 1. Application shell

- S1. Single main window; title = `<database path> — Smart PIM`. Menu bar: File, Edit, View,
  Go, Actions, Tools, Help (Notes adds Insert), mirroring EPIM's menus with equivalent
  commands.
- S2. Left icon rail (~44px, full height), top-to-bottom: Today, Mail (unread badge),
  Calendar, Tasks, Notes, Contacts, Passwords, Trash; bottom: account placeholder, Settings.
  MUST match EPIM ordering and behave as module switchers (Ctrl+1..8 shortcuts).
- S3. Each module = toolbar + tree panel + list panel + detail pane, per F-004. Panes
  resizable; layouts saved per module.
- S4. Global search box in every toolbar ("Search in: All fields, Ctrl+Q") → global
  FTS results view grouped by module; Enter opens the item.
- S5. Status bar: item count for current view, unread/mail info, bell with pending reminder
  count (click → reminders list), module-specific extras (Duration in Calendar; word count
  + zoom in Notes), theme toggle access via Settings.
- S6. Light + dark themes; light theme replicates the reference palette (white cards,
  light-gray panels, blue accent, colored left-accent bars, pill tags). Dark theme uses the
  same structure with an original dark palette.
- S7. Database lock screen: on launch (and after auto-lock timeout) a passphrase prompt
  unlocks the SQLCipher database. Auto-lock after configurable idle period; manual
  File→Lock. Wrong passphrase ≠ corrupt DB (retry, not error).
- S8. Trash module: unified soft-deleted items from all modules, filter by type and date,
  restore, delete permanently, auto-purge after configurable days.

## 2. Data & security

- S9. Single SQLCipher (AES-256) SQLite database in the app data dir; `--portable` flag
  stores everything beside the binary (USB mode). Schema via embedded migrations.
- S10. Every item type supports: tags, cross-links ("Related Items"), private flag
  (hidden until unlock; excluded from export), version history (Pro behavior: automatic on
  save; restore from history), attachments (stored by SHA-256 in a media folder), notes
  body (rich text), creation/modification timestamps.
- S11. Undo/redo of item mutations (inverse-op log) + audit log (timestamped, queryable in
  Tools→History).
- S12. Automatic backup: scheduled (default daily) `VACUUM INTO` snapshots with retention
  N; manual backup/restore in Tools. Non-destructive: never deletes originals.
- S13. Import (all types): file → column/field mapping preview → dry-run validation report
  (row-level errors/warnings) → commit in ONE transaction. Source file never modified.
  Formats: CSV (all modules), ICS (calendar/tasks), VCF (contacts), EML (mail), MD (notes).
- S14. Export: CSV, ICS, VCF, EML, HTML, MD per module; PDF via print-to-PDF for
  calendar/contacts/message printing with layout options.

## 3. Mail (P0)

- S15. Accounts: unlimited IMAP + POP3 (TLS); OAuth (Gmail/Outlook.com/iCloud) behind the
  same account dialog, enabled when client IDs are provided (P2 default). Per-account
  color + name; folders synced per IMAP hierarchy.
- S16. Unified Inbox aggregating all accounts' inboxes; per-account and per-folder views;
  folder tree with unread counts.
- S17. Message list: date-grouped cards (Today/Yesterday/Last Week/…), avatar/initial,
  sender, subject, 2-line preview, time, unread bold + accent bar, flags, attachment
  indicator, pinned section. Layouts: card (wide), classic (vertical split), wide.
- S18. Conversation threading with expand/collapse; opening a conversation shows the
  selected message.
- S19. Reading pane with HTML (sanitized) + plain-text toggle, remote-content blocking
  with per-sender allow, reply/forward/all, quick-reply box at pane bottom (Ctrl+Enter).
- S20. Compose: HTML editor + plain text, To/Cc/Bcc with contact autocomplete + contact
  group expansion, attachments, multiple signatures per identity, templates (dynamic
  placeholders), attachment-missing reminder heuristic, Save as draft (drafts synced to
  IMAP Drafts when supported).
- S21. Send Later: queued scheduled send (works while app runs; fires on next start if
  missed, flagged).
- S22. Rules: condition/action sets applied on incoming (per account or all), manual run on
  folder; adaptive spam filtering is P2.
- S23. Flags/follow-ups with reminder creation; categories; tags; archive policy
  (age-based move to Archive).
- S24. Convert message → Task / Event / Note with backlink (Related Items both ways).
- S25. PGP: import keys, sign/verify, encrypt/decrypt inline + PGP/MIME (P1). S/MIME P2.
- S26. Search: FTS over subject/from/to/body with filters (account, folder, unread,
  flagged, attachment, category, tag, date range). Perf target: results < 200 ms on 100k
  messages (Gate 3 checklist, mirrors prompt.md).
- S27. Polling/push: IDLE for IMAP when available, else interval poll; new-mail desktop
  notification + tray badge.

## 4. Calendar (P0)

- S28. Views: Day, Week, Week Agenda, Month, Year, Agenda; adjustable days-per-week;
  week numbers; working-hours definition with optional view clipping. View parity with the
  reference toolbar (view switcher group + Filter + Layout).
- S29. Event model: title, start/end (or all-day), calendar (colored), category (colored,
  holiday categories render red), location, completion %, description (rich text),
  participants, private flag, RRULE recurrence (full ICS semantics) with "Ends" presets and
  custom rule editor.
- S30. Multiple reminders per event (popup/sound; email P1); notification via
  desktop notifications; snooze/dismiss; reminder bell in status bar.
- S31. Multiple calendars with show/hide checkboxes; "Add Calendar"; per-calendar colors.
- S32. Interactions: click/drag to create; drag to move/resize; double-click edit; modal
  editor replicating the New Appointment dialog (type dropdown, start/end + duration link,
  all-day, calendar + category pickers, location, completion slider, repeat + custom rule,
  ends, reminders, participants, More Options, Save & New/OK/Cancel/Delete, Related Items
  tab).
- S33. Multi-day events render as banners spanning the Month/Week grid; "+N more"
  indicator per day; current-time line in Day/Week.
- S34. ICS: import (single + recurring expansion), export, publish-by-subscription
  (scheduled HTTP fetch of remote ICS, P1). Printing with layout presets.
- S35. Tasks sidebar within Calendar (P0); notes sidebar (P1); weather widget slot
  (interface only until a provider key exists, P2).

## 6. Tasks (P0)

- S36. Hierarchical tasks (unlimited depth), lists ("Local Tasks" style colored lists),
  Smart Lists: All, Important, Flagged, Completed (+ Next 7 Days, Overdue).
- S37. Fields: subject, notes (rich), start/due (date+time), priority (!), reminder(s),
  recurrence, categories, tags, custom fields (text/date/dropdown/yes-no/checkbox),
  completion %, assigned-to (free-text contact link), private.
- S38. Views: Tree (table with SUBJECT/START TIME/DUE TIME/!/CATEGORY/TAGS/ASSIGNEE
  columns, inline add row, drag to re-parent), Plain (flat), Kanban (columns by list or
  completion stage; per-column counts and "+"; drag between columns; card = colored
  background, checkbox, !, tags, dates, %).
- S39. Floating tasks: unfinished dated tasks roll to today until done (opt-in per list).
- S40. Completed style: checkbox + strikethrough everywhere; show-in-calendar (due-dated
  tasks appear in Calendar day grids and Tasks sidebar).
- S41. Templates; date-based grouping toggle; saved custom views (filter bar: category,
  tag, completion, date range → named view, like the reference "Custom View Bar").

## 7. Notes (P0)

- S42. Multilevel note tree with colored lists; Smart Lists: All, Favorites, Recently
  Viewed, Recently Modified.
- S43. Editor: rich text (headings, bold/italic/underline/strike, colors, highlight,
  lists, alignment, indent, links, tables, images, horizontal rules) + Markdown mode with
  live preview toggle; import/export .md; in-note multi-tabs (sub-pages) with "+" tab;
  anchors; find & replace in note; global FTS.
- S44. Note-level features: tags, reminders (26.5 behavior), favorites, pinned/sticky
  desktop windows (always-on-top small editors, P1), formatted titles, word count in
  status bar, zoom.
- S45. Convert selection → Task / Event.

## 8. Contacts (P0)

- S46. Contact fields: name parts, photo/avatar (initial avatar fallback with deterministic
  color), phones, emails, addresses, URLs, dates (birthday/anniversary — feed Today's
  Anniversaries), organization, custom fields, notes, tags, groups (multi-level,
  multi-membership), favorites.
- S47. List: A–Z letter bar, card rows (avatar, name, email, tag pills), inline add;
  detail: header card (avatar, name, star, tags, actions Send E-mail / Schedule Meeting),
  tabbed sections (Contact / Related Items), grouped field sections (Phone Numbers,
  Internet, Additional Fields, Membership, Notes) — replicating F-004.
- S48. Per-contact email history (auto-linked by address match); Schedule Meeting creates
  an event with the contact as participant; newsletters/merge-email (P1); label/envelope
  print templates (P1).
- S49. Views: card/table layouts; custom field columns; advanced filter builder (P1).

## 9. Passwords (P0)

- S50. Entries in tree groups (nested, icons, counts); fields: title, username, password
  (masked, reveal toggle, copy), URL, notes, custom + masked fields, tags, favicon fetch.
- S51. Generator: length + character-class options, exclude-ambiguous, passphrase mode.
- S52. Auto-Type (P1): global hotkey, sequence macros, Linux X11 first (wtype/ydotool on
  Wayland if permitted) — flagged per A-4.
- S53. Password strength indicator; search/filter; Related Items links.

## 10. Today (P0)

- S54. Dashboard replicating reference: Calendar section (day cards: events + weather
  slot), Tasks (grouped, sortable, multi-reschedule), Messages (folder unread counts),
  Anniversaries (upcoming birthdays/anniversary dates). Sections hideable/customizable;
  "New Appointment"/"New Task" quick actions; customization banner UI.

## 11. Sync (P1/P2 per matrix)

- S55. P1: CalDAV (calendar/tasks), CardDAV (contacts), Nextcloud/ownCloud; multiple sync
  profiles, one/two-way, scheduled or manual, structure (groups/hierarchies) preserved.
- S56. P2 (needs user-provided OAuth clients): Google (cal/tasks/contacts), M365,
  Outlook.com, iCloud, Toodledo.

## 12. Settings & platform

- S57. Settings: appearance (theme, accent, density), general (auto-lock, tray, start page),
  mail accounts, sync profiles, backup, shortcuts (remappable), plugins-none.
- S58. .deb: desktop entry, original icon, MIME associations (text/calendar .ics,
  text/vcard .vcf, text/csv, message/rfc822 .eml); single instance; tray icon optional.
- S59. Keyboard: EPIM-equivalent shortcuts (Ctrl+Q search, Ctrl+N new item, Ctrl+1..8
  modules, Ctrl+Enter quick reply, Delete, Ctrl+Z/Y, F5 send/receive) — full table in
  docs/architecture.md.
- S60. i18n-ready strings (en first).

## Non-functional

- N1. 100k-message mail corpus + 50k tasks: list virtualization ≥ 60 fps scroll; global
  search < 200 ms; cold start < 2 s to unlock prompt, < 1 s after unlock.
- N2. All data local; no telemetry; network only for explicitly configured accounts/sync.
- N3. Crash-safe: WAL, backup verification, migrations transactional.
