/**
 * Global search overlay (S4): Ctrl+Q from anywhere, FTS across modules,
 * results grouped by kind, Enter/click opens (Gate 3: routes to the module;
 * deep-open lands with module views in Gate 4). Gate 4 AI: an optional
 * "Semantic" toggle switches the query to embedding search — the chip only
 * exists when the provider is reachable and semantic search is opted in.
 */
import { useEffect, useRef, useState } from "preact/hooks";
import { api, type SearchHit } from "../api";
import { activeModule, searchOpen, type ModuleId } from "../store";
import { featureEnabled, useAiStatus } from "../lib/ai";
import "../design/ai.css";

const KIND_LABEL: Record<string, string> = {
  message: "Mail",
  mail: "Mail",
  task: "Tasks",
  note: "Notes",
};

export function GlobalSearch() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [selected, setSelected] = useState(0);
  const [semantic, setSemantic] = useState(false);
  const status = useAiStatus();
  const semanticAvailable = featureEnabled(status, "semantic");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (searchOpen.value) {
      setQuery("");
      setHits([]);
      setSelected(0);
      // focus after mount
      setTimeout(() => input.current?.focus(), 0);
    }
  }, [searchOpen.value]);

  useEffect(() => {
    if (!query.trim()) {
      setHits([]);
      return;
    }
    const t = setTimeout(async () => {
      try {
        setHits(
          semantic && semanticAvailable
            ? await api.aiSearchSemantic(query, 50)
            : await api.globalSearch(query, 50),
        );
        setSelected(0);
      } catch {
        setHits([]);
      }
    }, 120); // debounce; AT-5 keeps the round-trip well under this
    return () => clearTimeout(t);
  }, [query, semantic, semanticAvailable]);

  const open = (hit: SearchHit) => {
    const map: Record<string, ModuleId> = {
      message: "mail",
      mail: "mail",
      task: "tasks",
      note: "notes",
    };
    activeModule.value = map[hit.kind] ?? "today";
    searchOpen.value = false;
  };

  if (!searchOpen.value) return null;

  // Chip exists only while the feature is opted in; losing the provider
  // (or the opt-in) drops the toggle instead of leaving a dead control.
  const chip = semanticAvailable ? (
    <div class="search-chips">
      <button
        class="search-chip"
        data-active={semantic}
        title="Search by meaning (embeddings) instead of keywords"
        onClick={() => setSemantic((v) => !v)}
      >
        Semantic
      </button>
    </div>
  ) : null;

  const grouped = new Map<string, SearchHit[]>();
  for (const h of hits) {
    const list = grouped.get(h.kind) ?? [];
    list.push(h);
    grouped.set(h.kind, list);
  }

  return (
    <div class="overlay" onClick={() => (searchOpen.value = false)}>
      <div class="search-pop" onClick={(e) => e.stopPropagation()}>
        <input
          ref={input}
          class="search-input"
          placeholder="Search in: All fields"
          value={query}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") searchOpen.value = false;
            if (e.key === "ArrowDown") setSelected((s) => Math.min(s + 1, hits.length - 1));
            if (e.key === "ArrowUp") setSelected((s) => Math.max(s - 1, 0));
            if (e.key === "Enter" && hits[selected]) open(hits[selected]);
          }}
          aria-label="Global search"
        />
        {chip}
        <div class="search-results">
          {query.trim() && hits.length === 0 && <div class="search-empty">No results.</div>}
          {[...grouped.entries()].map(([kind, list]) => (
            <div key={kind}>
              <div class="search-group">{KIND_LABEL[kind] ?? kind}</div>
              {list.map((hit) => {
                const idx = hits.indexOf(hit);
                return (
                  <button
                    key={`${hit.kind}-${hit.id}`}
                    class="search-hit"
                    data-selected={idx === selected}
                    onMouseEnter={() => setSelected(idx)}
                    onClick={() => open(hit)}
                  >
                    <span class="search-hit-title">{hit.title}</span>
                    {hit.snippet && <span class="search-hit-snippet">{hit.snippet}</span>}
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
