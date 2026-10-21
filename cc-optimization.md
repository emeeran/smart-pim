# Claude Code Optimization — Performance & Economy

Applied 2026-10-20. Baseline: z.ai GLM proxy (`glm-5.3-flash`), RTK output condensing (~31% measured savings), auto-compact, medium effort.

## Changes applied

| Change | Detail | File |
|---|---|---|
| TinyFish MCP removed from global scope | ~30 tool schemas no longer injected into every turn; re-add per project when web access is needed | `~/.claude.json` |
| Thinking capped | `MAX_THINKING_TOKENS: "4000"` | `~/.claude/settings.json` (env) |
| Default permission mode | `plan` → `acceptEdits` (fewer round-trips, edits still prompt) | `~/.claude/settings.json` |
| Permission allowlist | 8 patterns from 10K-call transcript history (see below) | `.claude/settings.json` (project) |
| Project CLAUDE.md | Layout, commands, conventions — stops re-derivation each session | `CLAUDE.md` |

## Permission allowlist

Derived from the 50 most recent session transcripts:

| # | Pattern | Count | Notes |
|---|---|---|---|
| 1 | `Bash(cargo test *)` | 133 | mostly `cargo test -p core` |
| 2 | `Bash(cargo test)` | 4 | bare form |
| 3 | `Bash(cargo clippy *)` | 60 | ⚠ also covers `clippy --fix` |
| 4 | `Bash(npm run build)` | 13 | tsc + vite build |
| 5 | `Bash(npx tsc *)` | 10 | typecheck |
| 6 | `Bash(npx vitest run *)` | 9 | UI tests |
| 7 | `Bash(npm test)` / `Bash(npm test *)` | 5 | UI tests |

### Deliberately skipped

- `python3 -c` (99 calls) — interpreter = arbitrary code execution, never allowlist
- `sed -n`, `grep`, `cat`, `ls`, `git status`, `tail`, `pgrep`, `docker ps` — already auto-allowed by Claude Code
- `sed -i`, `git add`, `rm`, `mkdir`, `cargo fmt`, `docker run` — mutating
- `mcp__tinyfish__*` — server removed from global scope

## Ongoing practices

### Prompt-cache hygiene (biggest per-turn lever)
- Cached input costs ~10% of uncached — long focused sessions are cheaper per turn than many short ones
- Never edit `CLAUDE.md` or `settings.json` mid-session (busts the cache for the whole conversation)
- One task per session, but don't `/clear` mid-task or re-explain context

### Context discipline
- Delegate file dumps / log tails / long searches to subagents; keep main context small
- Batch independent tool calls into one message
- Use `offset`/`limit` when reading large files
- Commit via `/safe-commit` so lint + tests run once, not repeatedly

### Config security
- `ANTHROPIC_AUTH_TOKEN` and the TinyFish API key live in plaintext config files — never commit them to a dotfiles repo unencrypted

## Re-enabling TinyFish for a project

```sh
claude mcp add tinyfish -t http --scope project '<url>'
```

(URL and API key are preserved in the old `~/.claude.json` backup / TinyFish setup.)
