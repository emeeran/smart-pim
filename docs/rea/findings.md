# REA Findings Log — Smart PIM

Clean-room reverse-engineering findings. Each entry lists its source (URL/doc/observation).
Scope restriction per prompt.md: REA used only for feature inventory, UX flow analysis, and
open format/standard analysis. No proprietary binary or database extraction performed.

---

## F-001 — Product identity: "EPIM Pro" is ambiguous (Gate 0, BLOCKER)

**Status: UNRESOLVED — awaiting user decision. Two distinct products share the name.**

### Observation 1: epim.com is EssentialPIM, NOT a PIM system

- Source: https://www.epim.com (fetched 2026-10-18, content readable without JS rendering)
- "EPIM" at epim.com is a **personal** information manager / email client for Windows by
  **Astonsoft**: email, calendar, tasks, notes, contacts, passwords.
- "EPIM Pro" at that site is a paid edition of this personal organizer
  ($39.95/yr or $79.95 perpetual), alongside a free "EPIM Mail" edition.
- Latest version: EPIM 26.5.3 (released 2026-10-12). Mobile apps for Android/iOS, EPIM Cloud sync.
- Page author metadata: "Astonsoft".
- **This product has no relation to Product Information Management, BMEcat, ETIM, or ECLASS.**

### Observation 2: The PIM product matching prompt.md is Viamedici EPIM

- Source: https://viamedici.com/ (fetched 2026-10-18)
- Viamedici Software GmbH sells an Enterprise Product Information Management platform named
  **"Viamedici EPIM"** (current platform generation "EPIM/5"; predecessor "EPIM4").
- Components: VIA/MDM (multi-domain MDM), VIA/PIM360° (PIM), VIA/DAM (digital asset
  management), VIA/Configuration (CPQ/guided selling).
- Observable capabilities relevant to the feature matrix (from homepage copy, SoftwareReviews
  award citation, and customer quotes): product data administration, marketing information,
  media assets, multi-language (auto-translation cited in Reichle & De-Massari quote;
  "40+ languages" claim), media-neutral PIM + multi-media publishing (Zarges quote),
  "Golden Product Record" rule-based data distribution (Baumit quote), reuse of information
  structures across channels/publications (SICK quote), AI Hub/Gateway, Digital Product
  Passport & sustainability lifecycle, Digital Shelf Optimization.
- Analyst recognition: Gartner Magic Quadrant 2026, Frost & Sullivan Product Leadership 2023,
  Info-Tech/SoftwareReviews PIM Leader/Gold Medal 2022–2023.
- Sources: https://www.softwarereviews.com/products/viamedici-epim ,
  https://www.gartner.com/reviews/product/viamedici-epim ,
  https://navigator.publishing-connect.com/en/details/epim-enterprise-product-information-management

### Inference

- prompt.md says "EPIM Pro (Viamedici EPIM)" and describes PIM functionality
  (product master data, BMEcat, ETIM, ECLASS). That matches **Viamedici EPIM**, not the
  Astonsoft product at epim.com. The URL in prompt.md (https://www.epim.com) points at the
  wrong company — likely a mistaken reference.
- Additional wrinkle: **neither vendor markets an edition literally named "EPIM Pro".**
  Viamedici's editions/generations are "EPIM/5" and "EPIM4"; Astonsoft's "EPIM Pro" is the
  personal-organizer paid tier. So the exact product name in prompt.md matches no real SKU.

### Unknowns

- Whether the user intends the full Viamedici EPIM enterprise platform (MDM+PIM+DAM+CPQ)
  or specifically the PIM component (VIA/PIM360°) as the functional reference.
- No shipped binary/demo installer of Viamedici EPIM was identified as publicly downloadable;
  the feature inventory will need to rely on public docs/datasheets/demos/standards only.

### Action required

Per prompt.md Gate 0 step 1: identity is ambiguous → STOP and ask the user for
confirmation before producing docs/feature-matrix.md and docs/ASSUMPTIONS.md.

### RESOLUTION (2026-10-18)

User selected: **the product at https://www.epim.com** (EssentialPIM / EPIM Pro by
Astonsoft). Target is therefore a personal information manager, consistent with the
project name `EPIM-Lnx` (EssentialPIM has no Linux version). The Viamedici PIM items in
prompt.md are marked N/A — see docs/ASSUMPTIONS.md A-1 and docs/feature-matrix.md.
Consistent with this reading, the prompt.md URL was correct all along; the "Viamedici"
attribution in prompt.md's role section was the error.

---

## F-002 — Environment note

- No REA MCP analysis tools are registered in this session; no local shipped binary target
  exists for Gate 0. Public-source research (web) was used instead, consistent with the
  legal guardrails. Binary/format REA tooling is not needed until/unless a concrete shipped
  artifact becomes the target.

---

## F-003 — Feature inventory observations (Gate 0, 2026-10-18)

Sources: epim.com home, /features, /features/explore-epim/epim-mail-vs-pro,
/features/workspace/{mail,calendar,tasks,notes,contacts,passwords},
/features/explore-epim/integrations, /features/explore-epim/epim-cloud.
Observations only (public marketing/docs pages); no binary or DB analysis performed.

### Editions (observation)
- Two editions: "EPIM Mail" (free) and "EPIM Pro" ($39.95/yr or $79.95 perpetual).
  Pro adds: cloud-service sync, tags + cross-linking, templates, attachment storage +
  version history, automatic backup, Sticky Notes, in-app translation, item version
  history/restore, attachments manager, centralized tags, offline mode, custom fields,
  newsletters, labels/envelopes, multi-user "EPIM Pro Business".
- Latest release: 26.5.3 (2026-10-12); 26.5 highlights: note reminders, notes on sidebar,
  pinned emails, customizable quick actions, holiday categories, two sidebar styles.

### Modules and observable features (observation)
- Mail: IMAP/POP3/Exchange/Gmail/Outlook.com/iCloud; unified inbox; conversation view;
  quick reply; dynamic templates; PGP/PGP-MIME/S/MIME; adaptive spam filtering; message
  rules; send later; flags/follow-ups; convert email→task/event/note; attachment reminder;
  multiple signatures per identity; archiving policies; in-app translation; pinned emails.
- Calendar: Day/Week/Week Agenda/Month/Year/Agenda + adjustable views; recurring events;
  multiple reminders (popup/sound/email); categories incl. holiday categories; event
  templates; saved custom view bar; iCal subscriptions; participant invitations; Google
  Meet links; weather; working hours; printing; tasks sidebar; integrations: Google, M365,
  Outlook.com, iCloud, CalDAV, Nextcloud/ownCloud, EPIM Cloud/Server.
- Tasks: hierarchical subtasks; Tree/Plain/Kanban; floating tasks; recurring tasks;
  Smart Lists; custom fields (text/date/dropdown/yes/no/checkbox); templates; reminders;
  date-based grouping; participants; shown in Calendar; Google Tasks/MS To Do/Toodledo.
- Notes: multilevel tree; Markdown + rich text; images/tables/drawings/links/attachments;
  multi-tab; anchors; find & replace; tags; reminders; sticky notes + groups; text styles;
  rulers/line numbers; formatted titles; convert selection→task/event.
- Contacts: groups (multi-level, multi-assignment); favorites; custom fields; per-contact
  email history; advanced filters; newsletters with merge fields; labels/envelopes;
  click-to-dial; meeting scheduling; CardDAV.
- Passwords: encrypted store; generator; hierarchical groups; website icon fetch;
  auto-type; custom/masked fields; related-item links.
- Today: cross-module daily overview, color-coded, filters/date ranges.
- Trash: restore, filter by type/date.
- General: AES-256 DB encryption; auto-lock; global search; global sidebar; tags;
  cross-links; version history; attachments manager; automatic backup; private items
  excluded from sync/export; import/export CSV/ICS/VCF/EML/HTML/PDF/RTF/XML/MD; portable
  (USB) mode; light/dark customization.
- Sync (integration matrix, observation): Google (mail/cal/tasks/contacts; notes+passwords
  via Drive), M365/Exchange, Outlook.com, iCloud (mail/cal/contacts), CalDAV (cal/tasks),
  CardDAV (contacts), Toodledo (tasks/notes), Nextcloud/ownCloud (cal/tasks/notes/contacts/
  passwords), Dropbox (whole DB file), EPIM Cloud ($19.95/yr; sync+AES backup+web access),
  EPIM Server (LAN multi-user).
- Utilities: EPIM Synchronizer (DB↔DB sync), EPIM Archiver/DupRemover (archive + dedupe).

### Inference
- The Mail-vs-Pro comparison page enumerates features per module — the cleanest
  public source for the feature matrix priorities. Imported into docs/feature-matrix.md.
- No Linux version exists; no official Linux roadmap found on fetched pages (unknown).

### Unknowns
- Exact data file format, DB schema, and behavior of proprietary sync (EPIM Cloud/Server):
  intentionally not investigated (clean-room guardrail).
- Whether EPIM Pro's "Microsoft Exchange support" means EWS, ActiveSync, or MAPI —
  public pages do not say; if ever implemented, assume EWS-level effort (P2).

---

## F-004 — UI/UX replication directive and layout observations (Gate 0→1, 2026-10-18)

**Directive (user):** exactly replicate EPIM's UI/UX and features, except brand name and
icon. Guardrail amendment: layout, workflow, interaction patterns, and visual structure are
replicated in **original code**; no Astonsoft image assets, screenshots, icons, fonts, or
other copyrighted files are embedded or redistributed. Reference screenshots archived
locally at docs/rea/ui-ref/ (12 public slider images, 1280×800, light theme) for
observation only.

### Layout (observation, from all 12 screenshots)

- **Title bar:** "<db path> - <Product Name>" pattern (e.g. shows database file location).
- **Menu bar:** File, Edit, View, Go, Actions, Tools, Help (Notes module adds Insert).
- **Left icon rail (~44px, full height):** Today (home), Mail (unread badge),
  Calendar, Tasks, Notes, Contacts, Passwords (key), Trash; bottom-anchored: user/account
  icon, Settings gear.
- **Toolbar (per module):** primary "New <Item>" button with dropdown, Edit, Delete,
  module-specific actions (Send/Receive, Reply/Forward; Day/Week/Week Agenda/Month/Year/
  Agenda; Tree/Kanban; Sticky; Copy Username/Copy Password/Autofill), Filter, Layout;
  right-aligned global search "Search in: All fields  Ctrl+Q".
- **Tree panel (left, ~230px):** "Smart Lists" section (All / Important / Flagged /
  Completed; Notes: All / Favorites / Recently Viewed / Recently Modified) with counts;
  account section ("Local Tasks", "Local Calendars" with color checkboxes, "Local Notes",
  "Local Contacts", "My Passwords") as colored-dot/folder hierarchies with counts;
  trailing "+ Add List / Add Calendar / Add Group" link. Calendar adds a mini month
  calendar at bottom.
- **List panel (middle):**
  - Mail: date-grouped cards (TODAY / YESTERDAY / LAST WEEK headers), sender avatar,
    sender name, subject, 2-line preview, time, colored left accent bar, unread bold.
  - Tasks: table (SUBJECT, START TIME, DUE TIME, !, CATEGORY, TAGS, ASSIGNEE, follow-up
    flag), inline "Add a task" row, hierarchy via indent + disclosure triangles,
    completed = checkbox + strikethrough, tag pills.
  - Contacts: vertical A–Z letter bar; rows = colored initial avatar, name, email, tag
    pills; "Add a contact" row.
  - Passwords: A–Z letter bar; rows = favicon, title, URL, tag pills; "Add a password
    entry" row.
- **Detail pane (right):**
  - Mail: reading pane, quick-reply box pinned at bottom (Reply dropdown, identity
    selector, Send).
  - Tasks: Notes / Related Items tabs; rich-text toolbar (B I U strikethrough, color,
    highlight, lists, link, date, attach, contact); tag pills with × in header.
  - Notes: breadcrumb bar + history arrows; large title; Text / Related Items tabs; full
    formatting toolbar (font, size, B I U, color, highlight, lists, align, indent, link,
    date, table, image, emoji, Heading dropdown, undo/redo, attach); multi-tab notes at
    bottom ("Meeting Highlights / Next Steps / Materials & Links" + tab); bottom status:
    word count, Modified date, zoom slider.
  - Contacts: header card (big avatar, name, star favorite, tag pills, actions: Send
    E-mail, Schedule Meeting, …); sections: Contact / Phone Numbers / Internet /
    Additional Fields / Membership / Notes; Related Items tab with count.
  - Passwords: Entry / Related Items tabs; fields Title, User Name, Password (masked,
    eye toggle, Generate, copy buttons), URL; Membership; Notes.
- **Status bar:** "Items: N", "Duration: …" (calendar), unread badge, bell with reminder
  count, word count + zoom (notes), globe icon (far right).
- **Kanban:** columns with caps headers "TO DO 3 / IN PROGRESS 2 / COMPLETED 2", per-column
  "+", cards = colored background by list/category, checkbox, priority "!", tag pills,
  date range, progress %, drag between columns.
- **Today:** dismissible yellow "Customized view" banner; columns: CALENDAR (day cards with
  weather + colored event chips), Tasks (grouped, e.g. NO DUE DATE), MESSAGES (account +
  per-folder unread counts), ANNIVERSARIES (date badge + person).
- **New Appointment dialog:** type dropdown + Quick Print; Home / Related Items tabs;
  recurrence info banner; Start/End date+time pickers with duration link + "All day";
  Calendar picker + Category picker (colored swatches); Location; Completion slider
  (None–100%); "Repeat every N <unit>" + gear (custom RRULE) + Ends dropdown; Reminders
  section "+ Add Reminder" with preset dropdown + gear; participants autocomplete;
  "More Options +"; footer: Delete / Save & New / OK / Cancel.
- **Visual language:** light theme (screenshots), white cards on light-gray panels, thin
  separators, blue primary accent, rounded ~6px controls, colored left-accent bars, pill
  tags with soft backgrounds, Segoe UI-style humanist sans (Linux: system UI font stack).
  Dark theme toggle exists per feature docs.

### Inference for implementation

- Single-window shell: module rail swaps the three-pane content; each module = tree/list/
  detail triple with module-specific toolbar. Maps cleanly to a Tauri + TS component shell
  with per-module panes.
- "Every item has Notes + Related Items" is a universal pattern — implement once as a
  shared detail-tab component backed by item_ref links and a per-item rich-text/notes body.

### Unknowns
- Dark-theme palette values, exact dialog layouts beyond New Appointment, settings UI:
  no public screenshots on fetched pages; will approximate with the same design language
  (flagged, P1 refinement during Gate 3+).

---

## F-005 — Sync architecture observations (2026-10-19, full-clone planning)

**Directive (user):** cloud sync (Google Drive, Box, etc.) and Wi-Fi sync with an Android
app are now in scope; full import/export interchange with EPIM Pro required. Supersedes the
feature-matrix P2 rows for device sync and EPIM Cloud. Plan: docs/epim_clone_plan.md.

### Observations (public sources only)
- **Wi-Fi sync (PC Sync):** Android app scans the LAN for PCs running the desktop app;
  sync is initiated from the phone. Failure mode documented on the official forum: "No
  PCs running Windows version of EPIM"; fixes are increasing the scan timeout in PC Sync
  options or Menu → "Connect manually". Sources:
  https://www.epim.com/forum/viewtopic.php?t=14206 ,
  https://m.essentialpim.com/forum/viewtopic.php?t=36206 (fetched 2026-10-19).
- **Cloud file sync model:** EPIM's native cloud support is **whole-database-file** based
  — built-in Dropbox support moves the `.epim` file into Dropbox (File → Database
  Management); users manually place the single DB file on any sync drive (Google Drive,
  Box) the same way. Per-module cloud *protocol* sync (Google/M365/iCloud/CalDAV/CardDAV/
  Toodledo/Nextcloud) is separate, per the integrations matrix in F-003. Sources:
  https://www.epim.com/blog-and-news/blog/an-overview-of-backing-up-and-syncing-epim-data ,
  https://www.epim.com/features/explore-epim/integrations (fetched 2026-10-19).
- **Interchange:** EPIM exports CSV, HTML, PDF, ICS, VCF, RTF (and EML/TXT per module
  pages); supports export/import of selected data into another EPIM database file and
  Google Calendar / web-iCal import. No public documentation of the `.epim` internal
  format (and none sought — clean-room guardrail).

### Inference
- Smart PIM should mirror the same two-layer sync model: (a) whole encrypted DB file to
  any cloud drive/WebDAV, (b) per-module protocol sync (CalDAV/CardDAV/Google) — plus the
  device Wi-Fi sync protocol, for which EPIM gives the UX contract (phone-initiated,
  LAN scan, manual-connect fallback) but not the wire protocol (proprietary, not
  investigated). Smart PIM defines its own protocol over mDNS + TLS + pairing token.

### Unknowns
- EPIM's wire protocol, Android store format, and any EPIM Cloud API: intentionally not
  investigated (clean-room). Smart PIM's Android app will interoperate with Smart PIM
  desktop only, not with EPIM's Android app.
