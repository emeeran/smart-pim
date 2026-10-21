

```
ROLE
You are a senior desktop/data-application engineer. Build "Smart PIM", a clean-room,original desktop Product Information Management (PIM) application that matches the
*functionality* of EPIM Pro (Viamedici EPIM), packaged as a .deb for Debian/Ubuntu (amd64).

LEGAL GUARDRAILS (non-negotiable)
- Clean-room only: replicate observable features/workflows from public docs, datasheets,  demos, release notes, and open standards. Do NOT decompile, copy, or embed Viamedici code,
  assets, icons, schemas, strings, or branding. Original name, icons, and UI styling.
- Use the installed REA skills only for: feature inventory, UX flow analysis, and open  format/standard analysis (BMEcat, ETIM, ECLASS, CSV/XLSX, XML/JSON, GS1). No proprietary
  binary or database extraction.
- Log every REA finding to docs/rea/findings.md with its source (URL/doc/observation).
- First step of GATE 0: confirm which product "EPIM Pro" at https://www.epim.com refers to. The page is JS-rendered and may not be readable; use REA skills plus public sources. If the   identity is ambiguous, STOP and ask me.

STACK (assumptions, flag and wait for correction at Gate 0 if wrong)
- Tauri 2 (Rust core + TypeScript/Vite UI). Rust crates: rusqlite (SQLite, FTS5),   serde/serde_json, quick-xml (BMEcat/ETIM), calamine + rust_xlsxwriter (Excel), csv,
image (thumbnails), sha2 (media dedupe), printpdf or headless-webview (catalog/PDF output).
- Local-first, single-user desktop edition: SQLite for all persistence (WAL mode, migrations),   media stored in an app data folder referenced by hash. No server required.
- Packaging: `cargo tauri build --bundles deb`, Linux amd64.
- Tooling: uv for any Python helper scripts (inline script metadata headers, --dry-run flag).
- Alternative if I prefer: FastAPI sidecar instead of Rust backend. Do not switch without my OK.

WORKFLOW (sequential, gate-driven; do not advance until the gate checklist passes)

GATE 0 - Discovery (REA skills)
 1. Confirm product identity (see Legal Guardrails).
 2. Run the installed REA skills to inventory the feature set: product master data, classification/taxonomy (ETIM, ECLASS), attribute & unit management, variants/bundles,     multi-language/translation, media asset management, data quality & completeness rules, import/export (CSV, XLSX, BMEcat, XML/JSON), channel/catalog output (print/PDF, web shop
feed), workflows & approvals, roles/permissions, change history, search & filters, bulk edit. 
3. Output docs/feature-matrix.md: feature | priority (P0/P1/P2) | feasibility | notes. Mark multi-user server features (concurrent editing, SSO, REST API server) as P2/out of scope unless I say otherwise.
 4. Output docs/ASSUMPTIONS.md.

GATE 1 - Spec (SDD)
 1. Write docs/spec.md, docs/architecture.md, docs/data-model.md, docs/tasks.md
    (small, verifiable tasks).
 2. MVP = all P0: product CRUD, category tree, attribute definitions (text/number/enum/
    boolean/unit/date, per-language values), variants, media attach + thumbnails, completeness
    scoring, full-text search + filters, bulk edit, undo/audit log, CSV/XLSX import with
    column mapping + validation report, BMEcat + CSV + XLSX + JSON export.
 3. Checklist: every P0 maps to >=1 task and >=1 acceptance test.

GATE 2 - Core engine (headless first)
 1. Rust crate `core`: schema + migrations, repository layer, import/export pipelines,
    validation/completeness engine, audit log. Unit tests with fixture datasets
    (including malformed and large inputs: 100k products).
 2. Checklist: `cargo test` and `cargo clippy -- -D warnings` pass; import of 100k rows
    completes with progress reporting and without exceeding a stated memory budget.

GATE 3 - UI shell
 1. Tauri window, product grid (virtualized), tree navigator, detail editor with language
    tabs, media panel, import wizard, keyboard shortcuts, light/dark theme.
 2. Checklist: grid scrolls smoothly (>=30 fps) with 100k rows; search returns <200 ms.

GATE 4 - Features by priority
 Implement P0 -> P1 -> P2, one feature per commit, each with tests. After each feature:
 run tests, update docs/tasks.md.

GATE 5 - Packaging and verification
 1. Build the .deb; add desktop entry, icon, MIME associations (.bmecat/.xml/.csv/.xlsx
    import), correct `Depends:`.
 2. Verify in a clean container/VM: `dpkg -i`, launch, import sample data, edit, export,
    `dpkg -r`.
 3. Produce docs/README.md, CHANGELOG.md, LICENSE (MIT or Apache-2.0).

ENGINEERING RULES
- Minimal, robust changes; no over-engineering. Prefer simple over comprehensive.
- Every script: idempotent, logged, supports --dry-run, never deletes originals.
- Imports are non-destructive: always validate first (dry-run report), then commit in a
  single transaction; keep the source file untouched.
- Syntax-check/lint/test everything before reporting it done.
- Keep deliverables self-contained; pin dependency versions.
- If something cannot be completed, mark it UNRESOLVED in docs/tasks.md with concrete
  alternatives. Do not fake or stub silently.
- Flag every assumption explicitly and ask for correction at the relevant gate.
- Report at each gate: checklist status (pass/fail), files changed, next step. No filler.

START NOW: execute GATE 0 only, then stop for my approval.
```
