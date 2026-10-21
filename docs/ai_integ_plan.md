# AI Integration Plan — Smart PIM

Status: **implemented (2026-10-20)** — Phases 0–2 complete with two
documented deviations: (1) semantic search stores embeddings as f32 BLOBs in an `embedding`
table inside the SQLCipher database instead of a `sqlite-vec` extension (loadable-
extension packaging against bundled SQLCipher is impractical here; cosine runs in Rust
over an FTS prefilter — `core/src/aisearch.rs`); (2) §4's per-action cost estimate
before send is realized as cumulative per-1k-rate estimation in Settings (schema v6
`tokens_in`/`tokens_out` on `ai_job`, `ai_usage` command) rather than a pre-send token
count — no tokenizer ships with the app. Also completed after the main pass:
task subtask splitting (`subtask_split` job + task editor preview) and
signature → contact enrichment (`signature_contact` job with a deterministic
local scan plus model-filled names, merged or created through a preview diff —
`ai.feature.subtasks` / `ai.feature.contacts`). Implemented: providers (`src-tauri/src/ai.rs`),
job worker (`aiworker.rs`), commands (`aicmd.rs`: `ai_status`, `ai_settings_get/set`,
`ai_run`, `ai_job`, `ai_cancel`, `ai_reindex`, `ai_search_semantic`, `free_slots`,
`dupe_candidates`, `merge_contacts`, `item_tag_add/remove`, `ai_usage`,
`ai_usage_reset`), and the preview-then-apply
UI in Settings/Notes/Mail/Tasks/Diary/Contacts/GlobalSearch/Calendar. The list below is
the original plan, kept for reference.

## 1. Principles and local-first constraints

Smart PIM is local-first and its database is SQLCipher-encrypted
(argon2id-derived key, `core/src/store.rs`). AI features must respect that:

- **Nothing leaves the machine by default.** The default provider is local (see §2). A cloud provider requires explicit opt-in, per feature, and a visible indicator while content is being sent.
- **Embeddings live inside the encrypted DB or nowhere.** Any vector store must be a table in the same SQLCipher database (e.g. a `sqlite-vec` virtual table keyed by `item_id`), so encrypted-at-rest guarantees hold. If a practical constraint (size, extension availability) rules that out, the feature waits — embeddings in a plaintext sidecar file are not acceptable.
- **Opt-in, never a prerequisite.** Every AI feature degrades gracefully:
  no provider configured → the feature is simply absent from the UI.
- **No telemetry.** No usage stats, no crash reporting from AI paths.
- **Model downloads are user-initiated** (Ollama pulls its own models; we never fetch weights ourselves).
- **AI never writes without a diff preview.** Writes go through the existing `item_version` snapshot + `undo` machinery (`core/src/repo.rs`); the UI shows what will change and requires confirmation.

## 2. Provider strategy

Two tiers behind one internal trait (`src-tauri` side; no new crates in
`core` — plain HTTP from the shell):

| Tier        | Provider                                                                        | Transport          | Notes                                                                         |
| ----------- | ------------------------------------------------------------------------------- | ------------------ | ----------------------------------------------------------------------------- |
| 1 (default) | Ollama `http://127.0.0.1:11434` or any llama.cpp/OpenAI-compatible local server | HTTP, no auth      | Streaming; model choice user-configured                                       |
| 2 (opt-in)  | Cloud OpenAI/Anthropic-compatible endpoints                                     | HTTPS, BYO API key | Key stored in the `setting` table (SQLCipher-encrypted at rest), never logged |

Streaming: Tauri events (`app.emit`) carrying partial output, mirroring the existing event pattern (`session:unlocked`, reminder events).

## 3. Per-module opportunities

### Mail (highest value)

- **Thread digests** — summarize a thread (`thread_messages` command
  already returns the hierarchy) into 2–3 sentences + open questions.
- **Triage suggestions** — propose rule conditions for the *existing* rules
  engine (`core/src/rules.rs`, `mail_rule` table): "mail from X with Y in
  subject → move to Z", rendered as an editable rule, never auto-created.
- **Reply drafting** — draft a reply in the compose box from bullet points; user edits before send (compose + Send Later outbox already exist).
- Explicitly out: automatic sending, automatic flagging without preview.

### Search

- **Semantic search** alongside FTS5 (`core/src/search.rs`): embed note/task/
  diary/mail body text into a `sqlite-vec` table keyed by `item_id`; hybrid
  rank fusion (FTS5 `rank` + cosine). FTS stays the default; semantic is a
  toggle ("similar to this note").

### Calendar

- **Natural-language capture** — "lunch with Ana Thursday noon for an hour"
  → a pre-filled event editor (not a silent save). Parses against
  `events_window` for conflict display.
- **Find-a-slot** — given attendees' events in `events_window`, propose free
  hours. Pure computation first; LLM only for phrasing.

### Tasks

- **NL capture → task** ("call the dentist next week, urgent") mapped onto
  `task_save` fields (due date, priority) with the parsed values shown for
  correction.
- **Subtask splitting** — expand a task into 3–5 subtasks (task tree backing
  exists via `item.parent_id`; the tree view ships with this milestone).

### Notes & Diary

- Summarize / extract action items from a note (results open as new tasks,
  linked via `item_link`).
- Auto-tag suggestions against the existing `tag`/`item_tag` tables.
- Diary: optional "journal prompt of the day"; end-of-week reflection digest.
- Diary content is the most sensitive text in the app — diary embedding is
  opt-in separately from everything else.

### Contacts

- **Dedupe/fuzzy merge candidates** ("same name, same email domain") —
  deterministic matching does most of the work; LLM only ranks ambiguous
  cases. Merge always previews a diff.
- Signature → contact enrichment (create/update a contact from an email
  signature block), user-confirmed.

### Passwords

- **Nothing AI touches this module.** No password material, no metadata, no
  strength text is ever sent to any model — local or cloud. The generator
  and strength heuristic stay deterministic.

## 4. Infrastructure needed (when implemented)

- Commands: `ai_status`, `ai_run` (job-based), `ai_cancel`, `ai_settings_get/set`.
- Settings keys in the `setting` table: `ai.provider`, `ai.endpoint`,
  `ai.model`, `ai.api_key_ref`, `ai.feature.<name>` (per-feature opt-in).
- A job queue table + background worker following the existing
  `mailworker.rs` / `autolock.rs` spawn pattern; cancellable; results land
  as Tauri events.
- Token/cost accounting for tier-2 providers, surfaced in Settings.
- Diff-preview component for every write path (leverages `item_version`).

## 5. Risks

| Risk                                  | Mitigation                                                                             |
| ------------------------------------- | -------------------------------------------------------------------------------------- |
| Privacy: prompt content leaves device | Local default; per-feature cloud opt-in; banner during send; diary excluded by default |
| Hallucinated edits                    | Never auto-apply; propose + confirm; undo via existing machinery                       |
| Prompt injection from mail bodies     | Mail-derived content is data, never instructions; summaries quote, they don't execute  |
| Cost (tier 2)                         | Token accounting + per-action estimates before send                                    |
| Offline / no provider                 | Features disappear; core PIM unaffected                                                |
| Embedding drift after schema changes  | Vectors keyed by `item_id` + `updated_at`; stale entries re-embedded lazily            |
| Latency on 100k+ items                | Embeddings computed in background batches; search fuses FTS first                      |

## 6. Phased roadmap

- **Phase 0 — plumbing:** provider trait, settings UI, `ai_status`, one
  no-risk command (note summarize → preview panel). No writes.
- **Phase 1 — quick wins:** mail thread digest, NL task capture, note
  action-item extraction (all preview-then-apply).
- **Phase 2 — deeper:** semantic search (`sqlite-vec`), smart scheduling,
  contact dedupe, background triage suggestions, diary digest (separate
  opt-in).

Each phase ships behind per-feature settings flags and lands one feature per
commit with tests, matching the project's engineering rules.
