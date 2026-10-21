# smart-pim

Local-first personal information manager (PIM): mail, calendar, contacts, tasks, notes, diary, passwords. Tauri desktop app.

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
