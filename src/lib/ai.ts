/**
 * AI front-end glue (Gate 4 AI slice): cached ai_status, per-feature gating,
 * a job runner (listens for `ai://job`, polls ai_job as fallback) and small
 * pure parsers for job results. AI results are NEVER written directly —
 * callers render them in a preview and apply through the real APIs.
 */
import { useEffect } from "preact/hooks";
import { signal } from "@preact/signals";
import { api, type AiJobKind, type AiStatus } from "../api";

/** Cached ai_status; null until the first successful refresh. */
export const aiStatus = signal<AiStatus | null>(null);

export async function refreshAiStatus(): Promise<AiStatus | null> {
  try {
    aiStatus.value = await api.aiStatus();
  } catch {
    aiStatus.value = null; // locked / command not landed yet → features hidden
  }
  return aiStatus.value;
}

/** Read the cached status in a component, refreshing it once on mount so
 * entry points appear as soon as the backend reports a reachable provider. */
export function useAiStatus(): AiStatus | null {
  useEffect(() => {
    void refreshAiStatus();
  }, []);
  return aiStatus.value;
}

/** Pure gate: an entry point exists only when the backend is reachable AND
 * the feature is individually opted in (ai_integ_plan §1/§3). */
export function featureEnabled(status: AiStatus | null, name: string): boolean {
  return Boolean(status && status.reachable && status.features?.[name]);
}

/** Convenience read of the cached status. */
export function aiFeatureEnabled(name: string): boolean {
  return featureEnabled(aiStatus.value, name);
}

/** Feature keys used by the UI (ai.feature.<name> settings). */
export const AI_FEATURES = [
  "summarize",
  "digest",
  "reply",
  "capture",
  "action_items",
  "auto_tag",
  "diary",
  "triage",
  "subtasks",
  "contacts",
  "semantic",
] as const;

// ------------------------------------------------------------------ jobs

export interface AiJobOptions {
  /** Fallback poll interval (ms) when the ai://job event is missed. */
  pollMs?: number;
  /** Hard stop so a lost job never hangs the UI forever (0 = never). */
  timeoutMs?: number;
}

const DEFAULT_POLL_MS = 2_000;
const DEFAULT_TIMEOUT_MS = 180_000;

interface JobEventPayload {
  id: number;
  kind: string;
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  result?: unknown;
  error?: string | null;
}

/**
 * Run an AI job and resolve with its result. Subscribes to `ai://job`
 * BEFORE creating the job (no completion race) and polls `ai_job` every
 * `pollMs` in case the event never arrives. Rejects with the backend's
 * error text on failed/cancelled/timeout.
 */
export async function runAiJob(
  kind: AiJobKind,
  params: Record<string, unknown>,
  opts: AiJobOptions = {},
): Promise<unknown> {
  let jobId: number;
  try {
    jobId = await api.aiRun(kind, params);
  } catch (e) {
    throw new Error(errText(e));
  }

  return new Promise<unknown>((resolve, reject) => {
    let unsub: (() => void) | null = null;
    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let timeoutTimer: ReturnType<typeof setTimeout> | null = null;
    let settled = false;

    const cleanup = () => {
      unsub?.();
      unsub = null;
      if (pollTimer !== null) clearInterval(pollTimer);
      pollTimer = null;
      if (timeoutTimer !== null) clearTimeout(timeoutTimer);
      timeoutTimer = null;
    };
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      cleanup();
      fn();
    };
    const fail = (message: string) => finish(() => reject(new Error(message)));

    const settleRow = (state: string, result: unknown, error: string | null) => {
      if (state === "done") finish(() => resolve(result));
      else if (state === "failed") fail(error || "AI job failed");
      else if (state === "cancelled") fail("AI job cancelled");
    };

    void api
      .listen("ai://job", (ev) => {
        const p = ev.payload as JobEventPayload | undefined;
        if (!p || p.id !== jobId) return;
        settleRow(p.state, p.result, p.error ?? null);
      })
      .then((u) => {
        if (settled) u();
        else unsub = u;
      })
      .catch(() => {
        /* listen unavailable → polling only */
      });

    const poll = async () => {
      try {
        const row = await api.aiJob(jobId);
        settleRow(row.state, row.result, row.error);
      } catch {
        /* transient (locked, busy) — keep polling */
      }
    };
    pollTimer = setInterval(() => void poll(), opts.pollMs ?? DEFAULT_POLL_MS);
    void poll();

    const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    if (timeoutMs > 0) {
      timeoutTimer = setTimeout(
        () => fail("AI job timed out — try again or check Settings → AI."),
        timeoutMs,
      );
    }
  });
}

/** Backend rejections arrive as strings ("command not found", …). */
export function errText(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

// ------------------------------------------------- rule prefill hand-off

export interface SuggestedRule {
  account_id: number | null;
  field: string;
  op: string;
  value: string;
  action: string;
  action_value: string | null;
}

/** Mail → RulesDialog hand-off: "Suggest rules" can open the rules dialog
 * with a suggested row pre-filled for editing instead of creating it. */
export const rulePrefill = signal<SuggestedRule | null>(null);

// ------------------------------------------------------- result parsers
// Job results come back as untyped JSON; coerce defensively so a shape
// surprise renders as "nothing to apply" instead of a crash.

/** Pull displayable text out of a text-ish result. */
export function resultText(result: unknown): string {
  if (result === null || result === undefined) return "";
  if (typeof result === "string") return result;
  if (Array.isArray(result)) return result.map(resultText).filter(Boolean).join("\n\n");
  if (typeof result === "object") {
    const o = result as Record<string, unknown>;
    for (const key of ["summary", "digest", "text", "reflection", "body", "output", "result"]) {
      if (typeof o[key] === "string") return o[key] as string;
    }
    return "";
  }
  return String(result);
}

/** action_items → {items: string[]} (or a newline/bullet list as text). */
export function parseActionItems(result: unknown): string[] {
  if (result && typeof result === "object" && !Array.isArray(result)) {
    const o = result as Record<string, unknown>;
    const raw = o.items ?? o.actions ?? o.tasks;
    if (Array.isArray(raw)) {
      return raw
        .map((it) =>
          typeof it === "string"
            ? it.trim()
            : typeof it === "object" && it !== null && typeof (it as Record<string, unknown>).title === "string"
              ? String((it as Record<string, unknown>).title).trim()
              : "",
        )
        .filter(Boolean);
    }
  }
  const text = resultText(result);
  return text
    .split("\n")
    .map((line) => line.replace(/^\s*(?:[-*•]|\d+[.)])\s*/, "").trim())
    .filter(Boolean);
}

/** nl_task → {title, priority, due_utc}; null when nothing usable came back. */
export interface ParsedTask {
  title: string;
  priority: number;
  due_utc: string | null;
}

export function parseNlTask(result: unknown): ParsedTask | null {
  const o =
    result && typeof result === "object" && !Array.isArray(result)
      ? (result as Record<string, unknown>)
      : {};
  const title = typeof o.title === "string" ? o.title.trim() : "";
  if (!title) return null;
  const due = typeof o.due_utc === "string" && o.due_utc ? o.due_utc : null;
  return {
    title,
    priority: priorityFrom(o.priority),
    due_utc: due && !Number.isNaN(new Date(due).getTime()) ? new Date(due).toISOString() : null,
  };
}

/** "urgent"/"high"/2 → task priority 0..3 (unknown → normal). */
export function priorityFrom(value: unknown): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.max(0, Math.min(3, Math.round(value)));
  }
  if (typeof value === "string") {
    const names: Record<string, number> = { low: 0, normal: 1, medium: 1, high: 2, urgent: 3 };
    const n = names[value.trim().toLowerCase()];
    if (n !== undefined) return n;
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return Math.max(0, Math.min(3, Math.round(parsed)));
  }
  return 1;
}

/** draft_reply → {subject, body}; null when the model returned nothing usable. */
export function parseReplyDraft(result: unknown): { subject: string; body: string } | null {
  if (typeof result === "string") {
    return result.trim() ? { subject: "", body: result } : null;
  }
  const o =
    result && typeof result === "object" && !Array.isArray(result)
      ? (result as Record<string, unknown>)
      : {};
  const body = typeof o.body === "string" ? o.body : typeof o.text === "string" ? o.text : "";
  const subject = typeof o.subject === "string" ? o.subject : "";
  if (!body.trim() && !subject.trim()) return null;
  return { subject, body };
}

/** triage_suggest → {rules: [{field, op, value, action, action_value}]}. */
export function parseSuggestedRules(result: unknown): SuggestedRule[] {
  const raw: unknown = Array.isArray(result)
    ? result
    : (result as Record<string, unknown> | null)?.rules;
  if (!Array.isArray(raw)) return [];
  const fields = new Set(["from", "to", "subject"]);
  const ops = new Set(["contains", "equals"]);
  const actions = new Set(["move", "mark_read", "tag"]);
  return raw
    .map((r): SuggestedRule | null => {
      const o = (r ?? {}) as Record<string, unknown>;
      const value = typeof o.value === "string" ? o.value : "";
      if (!value.trim()) return null;
      const field = typeof o.field === "string" && fields.has(o.field) ? o.field : "subject";
      const op = typeof o.op === "string" && ops.has(o.op) ? o.op : "contains";
      const action = typeof o.action === "string" && actions.has(o.action) ? o.action : "move";
      const actionValue = typeof o.action_value === "string" ? o.action_value : null;
      const accountId = Number(o.account_id);
      return {
        account_id: Number.isFinite(accountId) && accountId > 0 ? accountId : null,
        field,
        op,
        value,
        action,
        action_value: action === "mark_read" ? null : actionValue,
      };
    })
    .filter((r): r is SuggestedRule => r !== null);
}

/** subtask_split → string[] of proposed subtask titles (≥1 to apply). */
export function parseSubtasks(result: unknown): string[] {
  const raw: unknown = Array.isArray(result)
    ? result
    : (result as Record<string, unknown> | null)?.subtasks;
  if (!Array.isArray(raw)) return parseActionItems(result);
  return raw
    .map((it) =>
      typeof it === "string"
        ? it.trim()
        : typeof it === "object" && it !== null && typeof (it as Record<string, unknown>).title === "string"
          ? String((it as Record<string, unknown>).title).trim()
          : "",
    )
    .filter(Boolean);
}

/** Signature-contact proposal (backend merges the deterministic scan with
 * model output). Every field is editable in the preview before applying. */
export interface ParsedContact {
  first_name: string;
  last_name: string;
  organization: string;
  emails: string[];
  phones: string[];
  urls: string[];
  /** Existing contacts whose email matches the sender (merge targets). */
  candidates: { contact_id: number; display_name: string; organization: string | null }[];
}

const EMPTY_CONTACT: ParsedContact = {
  first_name: "",
  last_name: "",
  organization: "",
  emails: [],
  phones: [],
  urls: [],
  candidates: [],
};

function strList(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((v): v is string => typeof v === "string" && v.trim() !== "").map((v) => v.trim())
    : [];
}

export function parseSignatureContact(result: unknown): ParsedContact | null {
  if (!result || typeof result !== "object" || Array.isArray(result)) return null;
  const o = result as Record<string, unknown>;
  const s = (k: string) => (typeof o[k] === "string" ? (o[k] as string).trim() : "");
  const candidates = Array.isArray(o.candidates)
    ? o.candidates
        .map((c) => {
          const co = (c ?? {}) as Record<string, unknown>;
          const id = Number(co.contact_id);
          return {
            contact_id: id,
            display_name: typeof co.display_name === "string" ? co.display_name : "",
            organization: typeof co.organization === "string" ? co.organization : null,
          };
        })
        .filter((c) => Number.isFinite(c.contact_id) && c.contact_id > 0)
    : [];
  const out: ParsedContact = {
    first_name: s("first_name"),
    last_name: s("last_name"),
    organization: s("organization"),
    emails: strList(o.emails),
    phones: strList(o.phones),
    urls: strList(o.urls),
    candidates,
  };
  // "Nothing usable" renders as an empty preview, not a crash.
  const hasAny =
    out.first_name || out.last_name || out.organization ||
    out.emails.length || out.phones.length || out.urls.length;
  return hasAny || candidates.length ? out : { ...EMPTY_CONTACT };
}
