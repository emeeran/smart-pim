/**
 * Shared detail-pane tabs (T2.7): "Notes | Related Items" pattern replicated
 * across modules. Loads links for `itemId` itself and supports link/unlink;
 * write actions run through the symmetric item_link table.
 */
import { useEffect, useState } from "preact/hooks";
import { api } from "../api";

interface LinkedItem {
  id: number;
  kind: string;
  title: string;
}

export function DetailTabs({
  notes,
  itemId,
  canEdit = true,
  emptyHint = "Nothing linked yet.",
  /** Rich related-items renderer (e.g. a contact's mails/tasks/events).
   * When given, the Related Items tab shows this instead of the raw
   * link list; link editing still lives below it. */
  related,
}: {
  notes: preact.ComponentChildren;
  itemId: number | null;
  canEdit?: boolean;
  emptyHint?: string;
  related?: preact.ComponentChildren;
}) {
  const [tab, setTab] = useState<"notes" | "related">("notes");
  const [links, setLinks] = useState<LinkedItem[]>([]);
  const [linkInput, setLinkInput] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTab("notes");
    setLinks([]);
    setError(null);
    if (itemId === null || itemId < 0) return;
    let cancelled = false;
    void api
      .itemLinks(itemId)
      .then((l) => {
        if (!cancelled) setLinks(l);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [itemId]);

  const reload = () => {
    if (itemId === null || itemId < 0) return;
    void api.itemLinks(itemId).then(setLinks).catch(() => {});
  };

  const unlink = (other: number) => {
    if (itemId === null) return;
    void api
      .itemUnlink(itemId, other)
      .then(reload)
      .catch((e) => setError(String(e)));
  };

  const linkById = () => {
    const other = Number(linkInput);
    if (itemId === null || !Number.isInteger(other) || other <= 0) return;
    void api
      .itemLink(itemId, other)
      .then(() => {
        setLinkInput("");
        reload();
      })
      .catch((e) => setError(String(e)));
  };

  return (
    <div class="detail-tabs">
      <div class="detail-tabbar" role="tablist">
        <button role="tab" aria-selected={tab === "notes"} onClick={() => setTab("notes")}>
          Notes
        </button>
        <button role="tab" aria-selected={tab === "related"} onClick={() => setTab("related")}>
          Related Items {links.length > 0 ? `(${links.length})` : ""}
        </button>
      </div>
      {/* Both panes stay mounted (hidden, not unmounted) so editors keep
          their state when flipping tabs. */}
      <div class="detail-tab-body" hidden={tab !== "notes"}>
        {notes}
      </div>
      <div class="detail-tab-body" hidden={tab !== "related"}>
        {related}
        {error && <div class="lock-error" role="alert">{error}</div>}
        {links.length === 0 && <p class="muted">{emptyHint}</p>}
        {links.length > 0 && (
          <ul class="related-list">
            {links.map((l) => (
              <li key={l.id}>
                <span class="related-kind" data-kind={l.kind}>
                  {l.kind}
                </span>
                <span class="related-title">{l.title || `Item #${l.id}`}</span>
                {canEdit && itemId !== null && (
                  <button class="related-unlink" title="Unlink" onClick={() => unlink(l.id)}>
                    ✕
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
        {canEdit && itemId !== null && itemId >= 0 && (
          <div class="related-add">
            <input
              type="number"
              min={1}
              placeholder="Item # to link"
              value={linkInput}
              onInput={(e) => setLinkInput((e.target as HTMLInputElement).value)}
            />
            <button onClick={linkById}>Link</button>
          </div>
        )}
      </div>
    </div>
  );
}

/** Pill tags (S6 language): render a tag list; colors come from tokens. */
export function TagPills({ tags }: { tags: string[] }) {
  if (tags.length === 0) return null;
  return (
    <span class="tag-pills">
      {tags.map((t) => (
        <span key={t} class="tag-pill">{t}</span>
      ))}
    </span>
  );
}

/** Smart Lists panel (T2.7): static entries + counts, per-module labels. */
export function SmartLists({
  entries,
  onSelect,
}: {
  entries: { label: string; count: number }[];
  onSelect: (label: string) => void;
}) {
  const [sel, setSel] = useState(entries[0]?.label ?? "");
  return (
    <nav class="smartlists" aria-label="Smart lists">
      {entries.map((e) => (
        <button
          key={e.label}
          class="smartlist-entry"
          data-active={e.label === sel}
          onClick={() => {
            setSel(e.label);
            onSelect(e.label);
          }}
        >
          <span>{e.label}</span>
          {e.count > 0 && <span class="smartlist-count">{e.count}</span>}
        </button>
      ))}
    </nav>
  );
}
