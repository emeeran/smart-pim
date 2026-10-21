/**
 * RelatedPanel: EPIM's "Related Items" tab — everything associated with an
 * item, grouped by kind and clickable across modules (a contact and their
 * mails, tasks, events; a note and its linked items). Data comes from the
 * `related_items` command; nothing here is editable — navigation only.
 */
import { useEffect, useState } from "preact/hooks";
import { api, type RelatedItem } from "../api";
import { jumpTo, type ModuleId } from "../store";

const GROUP_LABELS: Record<string, string> = {
  message: "Mail",
  task: "Tasks",
  event: "Calendar",
  note: "Notes",
  diary: "Diary",
  contact: "Contacts",
  other: "Linked",
};

const KIND_MODULE: Record<string, ModuleId> = {
  message: "mail",
  task: "tasks",
  event: "calendar",
  note: "notes",
  diary: "diary",
  contact: "contacts",
};

function fmtWhen(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" });
}

export function RelatedPanel({
  kind,
  id,
  refreshKey = 0,
}: {
  kind: string;
  id: number;
  /** Bump to re-query after the user links/unlinks something. */
  refreshKey?: number;
}) {
  const [items, setItems] = useState<RelatedItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setItems(null);
    setError(null);
    void api
      .relatedItems(kind, id)
      .then(setItems)
      .catch((e) => setError(String(e)));
  }, [kind, id, refreshKey]);

  if (error) return <p class="muted">Related items unavailable: {error}</p>;
  if (items === null) return <p class="muted">Loading related items…</p>;
  if (items.length === 0) {
    return (
      <p class="muted">
        Nothing linked yet — assign tasks to this contact or use their
        address in mail and the connections appear here.
      </p>
    );
  }

  const groups = new Map<string, RelatedItem[]>();
  for (const it of items) {
    const g = GROUP_LABELS[it.kind] ?? GROUP_LABELS.other;
    const list = groups.get(g) ?? [];
    list.push(it);
    groups.set(g, list);
  }

  return (
    <div class="related-panel">
      {[...groups.entries()].map(([label, list]) => (
        <div class="related-group" key={label}>
          <h4>
            {label} <span class="smartlist-count">{list.length}</span>
          </h4>
          <ul class="today-list">
            {list.map((it) => (
              <li key={`${it.kind}-${it.id}-${it.item_id}`}>
                <button
                  class="today-row"
                  onClick={() => {
                    const module = KIND_MODULE[it.kind] ?? "trash";
                    jumpTo(module, it.kind, it.id !== 0 ? it.id : it.item_id);
                  }}
                >
                  <span class="today-row-title">
                    {it.title}
                    {it.detail && <span class="muted"> · {it.detail}</span>}
                  </span>
                  <span class="muted">{fmtWhen(it.when)}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}
