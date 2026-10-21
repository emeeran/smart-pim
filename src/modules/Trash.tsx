/**
 * Trash module (S8): unified soft-deleted items across all modules, kind
 * filter, restore and permanent delete.
 */
import { useEffect, useState } from "preact/hooks";
import { api, type Item } from "../api";
import { VirtualList } from "../components/VirtualList";
import { refreshCounts, viewCount } from "../store";
import { icon } from "../design/icons";

const KINDS = ["", "event", "task", "note", "contact", "password", "message"];

export function Trash() {
  const [items, setItems] = useState<Item[]>([]);
  const [kind, setKind] = useState("");
  const [error, setError] = useState<string | null>(null);

  const reload = async () => {
    try {
      setItems(await api.trash(kind || undefined, 500));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind]);

  useEffect(() => {
    viewCount.value = items.length;
  }, [items.length]);

  const restore = async (id: number) => {
    await api.trashRestore(id);
    await reload();
    void refreshCounts();
  };
  const purge = async (id: number) => {
    if (!confirm("Delete permanently? This cannot be undone.")) return;
    await api.trashPurge(id);
    await reload();
    void refreshCounts();
  };

  return (
    <div class="trash-module">
      <div class="trash-toolbar">
        <label>
          Type:{" "}
          <select value={kind} onChange={(e) => setKind((e.target as HTMLSelectElement).value)}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {k === "" ? "All" : k}
              </option>
            ))}
          </select>
        </label>
        <span class="muted">Auto-purge policy lands with settings persistence (T3.44).</span>
      </div>
      {error && <div class="lock-error">{error}</div>}
      {items.length === 0 ? (
        <p class="muted empty-pane">Trash is empty.</p>
      ) : (
        <VirtualList
          rowCount={items.length}
          rowHeight={44}
          renderRow={(i) => {
            const item = items[i];
            return (
              <div key={item.id} class="trash-row" style={{ height: "44px" }}>
                <span class={`kind-dot kind-${item.kind}`} title={item.kind} />
                <span class="trash-kind">{item.kind}</span>
                <span class="trash-date" title={item.deleted_at ?? ""}>
                  deleted {item.deleted_at ? new Date(item.deleted_at).toLocaleString() : "?"}
                </span>
                <span class="trash-actions">
                  <button title="Restore" aria-label="Restore" onClick={() => void restore(item.id)}
                    dangerouslySetInnerHTML={{ __html: icon("restore") }} />
                  <button class="danger" title="Delete permanently" aria-label="Delete permanently"
                    onClick={() => void purge(item.id)}
                    dangerouslySetInnerHTML={{ __html: icon("purge") }} />
                </span>
              </div>
            );
          }}
        />
      )}
    </div>
  );
}
