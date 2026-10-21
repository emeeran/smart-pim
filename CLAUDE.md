# smart-pim

Local-first personal information manager (PIM): mail, calendar, contacts, tasks, notes, diary, passwords. Tauri desktop app.

## Layout

- `core/` — Rust workspace crate: domain logic, storage, sync, search. One module per domain (`mailstore.rs`, `calrepo.rs`, `contacts.rs`, `tasks.rs`, `notes.rs`, `diary.rs`, `pwrepo.rs`), plus `schema.rs` (migrations, current V5), `repo.rs`/`store.rs` (persistence), `sync.rs`, `related.rs` (entity relations), `ai.rs`/`aisearch.rs` (AI providers, embeddings).
- `src-tauri/` — Tauri shell, commands, Rust glue.
- `src/` — Preact + TypeScript UI (vite, tiptap, @preact/signals). `src/components/`, `src/modules/`, `src/shell/`, `src/lib/api.ts` (IPC bridge).
- `docs/` — plans, specs, changelog.

## Commands

- Rust tests: `rtk cargo test -p core`
- Lint: `cargo clippy --workspace` (pedantic warnings are on — keep them clean)
- UI typecheck+build: `npm run build` (runs `tsc --noEmit`)
- UI tests: `npm test` (vitest)
- Dev: `npm run dev` (vite) / `cargo tauri dev`

## Conventions

- Conventional commits (`feat:`, `fix:`, `docs:`), feature branches only — never commit to `main`.
- `unsafe_code = "forbid"` at workspace level; no AI co-author trailers in commits.
- Schema changes go through `schema.rs` migrations — bump version, never mutate old migrations.
