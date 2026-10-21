/**
 * DupeDialog: contact dedupe candidates (dupe_candidates) with a
 * pick-a-primary + preview-then-merge flow per group. Merge goes through
 * the real merge_contacts command — nothing is written before the user
 * confirms which record is kept.
 */
import { useEffect, useState } from "preact/hooks";
import { api, type ContactRecord } from "../api";
import { errText } from "../lib/ai";
import "../design/ai.css";

export interface DupeGroup {
  contact_ids: number[];
  reason: string;
}

export interface DupeDialogProps {
  open: boolean;
  contacts: ContactRecord[];
  onClose: () => void;
  /** Called after any successful merge so the list can refresh. */
  onMerged: () => void;
}

/** One-line summary of the record that will survive a merge. */
function describe(c: ContactRecord): string {
  const bits = [
    c.display_name || "(unnamed)",
    c.emails[0],
    c.phones[0],
    c.organization,
  ].filter(Boolean);
  return bits.join(" · ");
}

export function DupeDialog({ open, contacts, onClose, onMerged }: DupeDialogProps) {
  const [groups, setGroups] = useState<DupeGroup[]>([]);
  const [primary, setPrimary] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setBusy(true);
    setError(null);
    setNote(null);
    api
      .dupeCandidates()
      .then((g) => {
        setGroups(g);
        const seed: Record<string, number> = {};
        for (const group of g) {
          seed[group.contact_ids.join("-")] = group.contact_ids[0];
        }
        setPrimary(seed);
      })
      .catch((e) => setError(errText(e)))
      .finally(() => setBusy(false));
  }, [open]);

  if (!open) return null;

  const byId = new Map(contacts.map((c) => [c.id, c]));

  const mergeGroup = async (group: DupeGroup) => {
    const key = group.contact_ids.join("-");
    const keep = primary[key] ?? group.contact_ids[0];
    const rest = group.contact_ids.filter((id) => id !== keep);
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      for (const id of rest) {
        await api.mergeContacts(keep, id);
      }
      setNote(
        rest.length > 0
          ? `Merged ${rest.length} contact(s) into "${byId.get(keep)?.display_name ?? `#${keep}`}".`
          : "Nothing to merge.",
      );
      setGroups((gs) => gs.filter((g) => g !== group));
      onMerged();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="overlay" onClick={busy ? undefined : onClose}>
      <div
        class="dialog ai-preview"
        style={{ width: "620px" }}
        role="dialog"
        aria-modal="true"
        aria-label="Find duplicates"
        onClick={(e) => e.stopPropagation()}
      >
        <h3>Find duplicates</h3>
        {busy && !note && <p class="muted">Loading candidates…</p>}
        {error && <div class="ai-error">{error}</div>}
        {note && <div class="settings-ok ai-reindex-note">{note}</div>}
        <div class="ai-preview-body">
          {!busy && groups.length === 0 && !error && (
            <p class="muted">No likely duplicates found.</p>
          )}
          {groups.map((g) => {
            const key = g.contact_ids.join("-");
            const keep = primary[key] ?? g.contact_ids[0];
            const kept = byId.get(keep);
            return (
              <div key={key} class="ai-dupe-group">
                <div class="ai-dupe-reason">{g.reason || "Similar contacts"}</div>
                {g.contact_ids.map((id) => {
                  const c = byId.get(id);
                  return (
                    <label key={id} class="ai-dupe-member">
                      <input
                        type="radio"
                        name={`dupe-${key}`}
                        checked={keep === id}
                        onChange={() => setPrimary((p) => ({ ...p, [key]: id }))}
                      />
                      <span>
                        {c ? describe(c) : `Contact #${id}`}
                        {id === keep && <span class="ai-note"> — will be kept</span>}
                      </span>
                    </label>
                  );
                })}
                <div class="ai-dupe-actions">
                  <button
                    class="primary"
                    disabled={busy || g.contact_ids.length < 2}
                    onClick={() => void mergeGroup(g)}
                  >
                    Merge into kept
                  </button>
                  <span class="ai-note">
                    {kept
                      ? `Keeps "${kept.display_name || `#${kept.id}`}" and merges the rest into it.`
                      : "Merge the others into the kept record."}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
        <div class="form-footer">
          <span class="form-footer-spacer" />
          <button onClick={onClose} disabled={busy}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
