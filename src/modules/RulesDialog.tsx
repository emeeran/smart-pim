/**
 * Rules dialog (S22 / T3.8): list, enable/disable, create, delete mail
 * rules. Evaluation itself is core::rules, applied on every ingest.
 * Mail's "Suggest rules" can hand a suggested row over via the
 * `rulePrefill` signal (src/lib/ai.ts) — it seeds the "New rule" form.
 */
import { useEffect, useState } from "preact/hooks";
import { api, type Account, type RuleRow } from "../api";
import { rulePrefill } from "../lib/ai";
import { refreshCounts } from "../store";

export const rulesOpen = { value: false };
const listeners = new Set<() => void>();

export function openRules(): void {
  rulesOpen.value = true;
  listeners.forEach((l) => l());
}

function describe(rule: RuleRow): string {
  const cond = rule.conditions.map((c) => `${c.field} ${c.op} "${c.value}"`).join(" AND ");
  const acts = rule.actions
    .map((a) => (a.type === "move" ? `move to ${a.folder}` : a.type === "tag" ? `tag ${a.value}` : a.type))
    .join(", ");
  return `IF ${cond} THEN ${acts}`;
}

export function RulesDialog() {
  const [open, setOpen] = useState(false);
  const [rules, setRules] = useState<RuleRow[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [form, setForm] = useState({
    account_id: -1,
    field: "from",
    op: "contains",
    value: "",
    action: "move",
    action_value: "Archive",
  });

  useEffect(() => {
    const l = () => {
      setOpen(true);
      // A suggested rule from Mail's AI flow pre-fills the "New rule" form.
      const pre = rulePrefill.value;
      if (pre) {
        rulePrefill.value = null;
        setForm({
          account_id: pre.account_id ?? -1,
          field: pre.field,
          op: pre.op,
          value: pre.value,
          action: pre.action,
          action_value: pre.action_value ?? "Archive",
        });
      }
      void (async () => {
        try {
          setRules(await api.rulesList());
          setAccounts(await api.mailAccounts());
        } catch {
          /* locked */
        }
      })();
    };
    listeners.add(l);
    return () => listeners.delete(l);
  }, []);

  if (!open && !rulesOpen.value) return null;
  const close = () => {
    rulesOpen.value = false;
    setOpen(false);
  };

  return (
    <div class="overlay" onClick={close}>
      <div class="dialog import-wizard" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Rules">
        <h3>Mail Rules</h3>
        <div class="rules-list">
          {rules.length === 0 && <p class="muted">No rules yet.</p>}
          {rules.map((r) => (
            <div key={r.id} class="rules-row">
              <input
                type="checkbox"
                checked={r.enabled}
                title="Enabled"
                onChange={async (e) => {
                  await api.ruleSetEnabled(r.id, (e.target as HTMLInputElement).checked);
                  setRules(await api.rulesList());
                }}
              />
              <span class="rules-desc">{describe(r)}</span>
              <button
                class="danger-text"
                onClick={async () => {
                  await api.ruleDelete(r.id);
                  setRules(await api.rulesList());
                  void refreshCounts();
                }}
              >
                Delete
              </button>
            </div>
          ))}
        </div>
        <h4>New rule</h4>
        <div class="rules-form">
          <select value={form.account_id} onChange={(e) => setForm({ ...form, account_id: Number((e.target as HTMLSelectElement).value) })}>
            <option value={-1}>All accounts</option>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>{a.name}</option>
            ))}
          </select>
          <select value={form.field} onChange={(e) => setForm({ ...form, field: (e.target as HTMLSelectElement).value })}>
            <option value="from">from</option>
            <option value="to">to</option>
            <option value="subject">subject</option>
          </select>
          <select value={form.op} onChange={(e) => setForm({ ...form, op: (e.target as HTMLSelectElement).value })}>
            <option value="contains">contains</option>
            <option value="equals">equals</option>
          </select>
          <input placeholder="value" value={form.value} onInput={(e) => setForm({ ...form, value: (e.target as HTMLInputElement).value })} />
          <select value={form.action} onChange={(e) => setForm({ ...form, action: (e.target as HTMLSelectElement).value })}>
            <option value="move">move to folder</option>
            <option value="mark_read">mark read</option>
            <option value="tag">tag</option>
          </select>
          {(form.action === "move" || form.action === "tag") && (
            <input
              placeholder={form.action === "move" ? "Folder" : "Tag"}
              value={form.action_value}
              onInput={(e) => setForm({ ...form, action_value: (e.target as HTMLInputElement).value })}
            />
          )}
          <button
            class="primary"
            disabled={!form.value.trim()}
            onClick={async () => {
              try {
                await api.ruleCreate({
                  account_id: form.account_id === -1 ? null : form.account_id,
                  field: form.field,
                  op: form.op,
                  value: form.value,
                  action: form.action,
                  action_value: form.action === "mark_read" ? null : form.action_value,
                });
                setForm({ ...form, value: "" });
                setRules(await api.rulesList());
              } catch (e) {
                alert(String(e));
              }
            }}
          >
            Add rule
          </button>
        </div>
        <div class="dialog-actions">
          <button onClick={close}>Close</button>
        </div>
      </div>
    </div>
  );
}
