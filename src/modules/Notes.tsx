/**
 * Notes module (S42–S43 UI slice): smart lists | note cards | Tiptap editor
 * with debounced autosave (version snapshots server-side on real changes),
 * favorites, trash. Gate 4: breadcrumb + back/forward view history and a
 * word count in the detail status bar. Note tabs + find/replace arrive with
 * T3.30.
 */
import { useEffect, useRef, useState } from "preact/hooks";
import "../design/panels.css";
import { api, type NoteDetail, type NoteRow } from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { DetailTabs } from "../components/DetailTabs";
import { VirtualList } from "../components/VirtualList";
import { RichEditor } from "../components/RichEditor";
import { AiPreview } from "../components/AiPreview";
import {
  errText,
  featureEnabled,
  parseActionItems,
  resultText,
  runAiJob,
  useAiStatus,
} from "../lib/ai";
import { appendPage } from "../lib/list-model";
import { wordCount } from "../lib/grouping";
import { refreshCounts, viewCount } from "../store";

const ROW_H = 56;

interface SummaryState {
  open: boolean;
  body: string;
  busy: boolean;
  error: string | null;
}

interface ActionItemsState {
  open: boolean;
  items: string[];
  checked: boolean[];
  busy: boolean;
  applying: boolean;
  error: string | null;
  note: string | null;
}

const SUMMARY_IDLE: SummaryState = { open: false, body: "", busy: false, error: null };
const ACTIONS_IDLE: ActionItemsState = {
  open: false,
  items: [],
  checked: [],
  busy: false,
  applying: false,
  error: null,
  note: null,
};

// --------------------------------------------------------------- editor

// ---------------------------------------------------------------- module

export function NotesModule() {
  const [rows, setRows] = useState<NoteRow[]>([]);
  const [exhausted, setExhausted] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  const [detail, setDetail] = useState<NoteDetail | null>(null);
  const [onlyFavorites, setOnlyFavorites] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [title, setTitle] = useState("");
  const [html, setHtml] = useState("");
  /** Session-local view history for the Back/Forward arrows. */
  const [history, setHistory] = useState<number[]>([]);
  const [hIndex, setHIndex] = useState(-1);
  const busy = useRef(false);
  const done = useRef(false);
  const saveTimer = useRef<number | null>(null);
  done.current = exhausted;

  const loadMore = async () => {
    if (busy.current || done.current) return;
    busy.current = true;
    try {
      const last = rows[rows.length - 1];
      const page = await api.notesPage(last?.updated_at, last?.id, 100);
      if (page.length === 0) setExhausted(true);
      else setRows((r) => appendPage(r, page).rows);
    } catch {
      setExhausted(true);
    } finally {
      busy.current = false;
    }
  };

  useEffect(() => {
    void loadMore();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows.length]);

  useEffect(() => {
    viewCount.value = onlyFavorites ? rows.filter((r) => r.favorite).length : rows.length;
  }, [rows.length, onlyFavorites]);

  const loadNote = (id: number) => {
    setSelected(id);
    setDirty(false);
    void api.noteGet(id).then((n) => {
      setDetail(n);
      setTitle(n.title);
      setHtml(n.content);
    });
  };

  /** Open a note, recording it in the session history (unless navigating). */
  const openNote = (id: number, record = true) => {
    if (record) {
      const base = history.slice(0, hIndex + 1);
      if (base[base.length - 1] !== id) {
        setHistory([...base, id]);
        setHIndex(base.length);
      }
    }
    loadNote(id);
  };

  /** Back/Forward within the notes already viewed this session. */
  const stepHistory = (delta: -1 | 1) => {
    const next = hIndex + delta;
    const id = history[next];
    if (id === undefined) return;
    setHIndex(next);
    loadNote(id);
  };

  const scheduleSave = (nextTitle: string, nextHtml: string) => {
    if (selected === null) return;
    setDirty(true);
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(async () => {
      await api.noteSave(selected, nextTitle, nextHtml);
      setDirty(false);
      setRows((r) =>
        r.map((n) =>
          n.id === selected
            ? { ...n, title: nextTitle, snippet: nextHtml.replace(/<[^>]*>/g, " ").trim().slice(0, 120) }
            : n,
        ),
      );
    }, 900);
  };

  const createNote = async () => {
    const id = await api.noteCreate("Untitled note", "<p></p>");
    setRows((r) => [
      {
        id,
        item_id: -1,
        title: "Untitled note",
        snippet: "",
        favorite: false,
        updated_at: new Date().toISOString(),
      },
      ...r,
    ]);
    openNote(id);
    void refreshCounts();
  };

  const visible = onlyFavorites ? rows.filter((r) => r.favorite) : rows;

  // ------------------------------------------------------------- AI actions
  // Entry points exist only when the provider is reachable AND the feature
  // is opted in; every result is preview-only until the user applies.
  const status = useAiStatus();
  const canSummarize = featureEnabled(status, "summarize") && detail !== null;
  const canExtract = featureEnabled(status, "action_items") && detail !== null;
  const [summary, setSummary] = useState<SummaryState>(SUMMARY_IDLE);
  const [aiItems, setAiItems] = useState<ActionItemsState>(ACTIONS_IDLE);

  const runSummary = async () => {
    if (detail === null) return;
    setSummary({ open: true, body: "", busy: true, error: null });
    try {
      const text = resultText(await runAiJob("summarize_note", { note_id: detail.id }));
      setSummary({ open: true, body: text, busy: false, error: null });
    } catch (e) {
      setSummary({ open: true, body: "", busy: false, error: errText(e) });
    }
  };

  const runActionItems = async () => {
    if (detail === null) return;
    setAiItems({ ...ACTIONS_IDLE, open: true, busy: true });
    try {
      const items = parseActionItems(await runAiJob("action_items", { note_id: detail.id }));
      setAiItems({
        open: true,
        items,
        checked: items.map(() => true),
        busy: false,
        applying: false,
        error: null,
        note:
          items.length > 0
            ? "Checked items become tasks linked to this note on Apply."
            : "No action items found in this note.",
      });
    } catch (e) {
      setAiItems({ ...ACTIONS_IDLE, open: true, error: errText(e) });
    }
  };

  const applyActionItems = async () => {
    if (detail === null) return;
    setAiItems((a) => ({ ...a, applying: true, error: null }));
    let created = 0;
    let linked = 0;
    try {
      for (let i = 0; i < aiItems.items.length; i++) {
        if (!aiItems.checked[i]) continue;
        const taskId = await api.taskSave({
          id: null,
          title: aiItems.items[i],
          notes: null,
          due_utc: null,
          priority: 1,
          percent: 0,
        });
        created++;
        // Link by item ids: resolve the fresh task's item_id (task_save
        // returns the task row id), best-effort — the task exists either way.
        try {
          if (detail.item_id > 0) {
            const all = await api.tasksOpen();
            const row = all.find((t) => t.id === taskId);
            if (row) {
              await api.itemLink(detail.item_id, row.item_id);
              linked++;
            }
          }
        } catch {
          /* linking is best-effort */
        }
      }
      void refreshCounts();
      setAiItems((a) => ({
        ...a,
        applying: false,
        items: [],
        checked: [],
        note: `Created ${created} task(s)${linked > 0 ? `, linked ${linked} to this note` : ""}.`,
      }));
    } catch (e) {
      setAiItems((a) => ({ ...a, applying: false, error: errText(e) }));
    }
  };

  const copySummary = () => {
    try {
      void navigator.clipboard?.writeText(summary.body);
    } catch {
      /* clipboard unavailable */
    }
  };

  useToolbar(
    <div class="mail-toolbar">
      <button class="primary" onClick={() => void createNote()}>
        New Note
      </button>
      {canSummarize && (
        <button title="Summarize this note with the local model" onClick={() => void runSummary()}>
          AI Summarize
        </button>
      )}
      {canExtract && (
        <button
          title="Extract action items as tasks"
          onClick={() => void runActionItems()}
        >
          AI Action Items
        </button>
      )}
    </div>,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [detail, status],
  );

  return (
    <div class="notes-module">
      <ThreePane
        module="notes"
        tree={
          <nav class="smartlists" aria-label="Note lists">
            <button class="smartlist-entry" data-active={!onlyFavorites} onClick={() => setOnlyFavorites(false)}>
              <span>All Notes</span>
              <span class="smartlist-count">{rows.length}</span>
            </button>
            <button class="smartlist-entry" data-active={onlyFavorites} onClick={() => setOnlyFavorites(true)}>
              <span>Favorites</span>
              <span class="smartlist-count">{rows.filter((r) => r.favorite).length}</span>
            </button>
          </nav>
        }
        list={
          <div class="notes-list">
            {visible.length === 0 ? (
              <p class="muted empty-pane">No notes yet — create one.</p>
            ) : (
              <VirtualList
                rowCount={visible.length}
                rowHeight={ROW_H}
                onNearEnd={() => void loadMore()}
                renderRow={(i) => {
                  const n = visible[i];
                  return (
                    <div
                      key={n.id}
                      class="note-card"
                      data-selected={n.id === selected}
                      style={{ height: `${ROW_H}px` }}
                      onClick={() => openNote(n.id)}
                    >
                      <span class="note-card-main">
                        <span class="note-card-title">
                          {n.favorite && <span class="note-fav" title="Favorite">★</span>}
                          {n.title || "Untitled"}
                        </span>
                        <span class="note-card-snippet">{n.snippet}</span>
                      </span>
                    </div>
                  );
                }}
              />
            )}
          </div>
        }
        detail={
          detail === null ? (
            <div class="muted empty-pane">Select a note to edit it.</div>
          ) : (
            <DetailTabs
              itemId={detail.item_id}
              notes={
                <div class="note-editor-pane">
                  <div class="note-breadcrumb">
                    <button
                      class="note-history-btn"
                      title="Back"
                      aria-label="Back"
                      disabled={hIndex <= 0}
                      onClick={() => stepHistory(-1)}
                    >
                      ◀
                    </button>
                    <button
                      class="note-history-btn"
                      title="Forward"
                      aria-label="Forward"
                      disabled={hIndex >= history.length - 1}
                      onClick={() => stepHistory(1)}
                    >
                      ▶
                    </button>
                    {/* Notes have no parent chain in the data model, so the
                        path is module name > title (A-NOTE-PATH). */}
                    <span>Notes</span>
                    <span aria-hidden="true">›</span>
                    <span class="crumb-current">{title || "Untitled"}</span>
                  </div>
                  <div class="note-editor-header">
                    <input
                  class="note-title-input"
                  value={title}
                  placeholder="Title"
                  onInput={(e) => {
                    const v = (e.target as HTMLInputElement).value;
                    setTitle(v);
                    scheduleSave(v, html);
                  }}
                />
                <button
                  data-active={detail.favorite}
                  title="Favorite"
                  onClick={() => {
                    const fav = !detail.favorite;
                    detail.favorite = fav;
                    void api.noteSetFavorite(detail.id, fav);
                    setRows((r) => r.map((n) => (n.id === detail.id ? { ...n, favorite: fav } : n)));
                  }}
                >
                  ★
                </button>
                <button
                  title="Move to trash"
                  onClick={async () => {
                    if (!confirm("Move this note to trash?")) return;
                    await api.noteDelete(detail.id);
                    setDetail(null);
                    setSelected(null);
                    setRows((r) => r.filter((n) => n.id !== detail.id));
                    const rest = history.filter((h) => h !== detail.id);
                    setHistory(rest);
                    setHIndex((i) => Math.min(i, rest.length - 1));
                    void refreshCounts();
                  }}
                >
                  🗑
                </button>
              </div>
              <RichEditor
                key={detail.id}
                html={html}
                onChange={(next) => {
                  setHtml(next);
                  scheduleSave(title, next);
                }}
              />
              <div class="note-statusbar">
                {dirty ? "Saving…" : "Saved"} · words:{" "}
                <span class="note-wordcount">{wordCount(html)}</span>
              </div>
                </div>
              }
            />
          )
        }
      />
      <AiPreview
        open={summary.open}
        title="AI summary"
        onClose={() => setSummary(SUMMARY_IDLE)}
        body={summary.body}
        busy={summary.busy}
        busyLabel="Summarizing…"
        error={summary.error}
        actions={[
          { label: "Copy", onClick: copySummary, disabled: !summary.body },
        ]}
      />
      <AiPreview
        open={aiItems.open}
        title="Action items"
        onClose={() => setAiItems(ACTIONS_IDLE)}
        busy={aiItems.busy}
        busyLabel="Extracting action items…"
        error={aiItems.error}
        actions={[
          {
            label: "Apply checked",
            primary: true,
            disabled: aiItems.applying || aiItems.items.length === 0,
            onClick: () => void applyActionItems(),
          },
        ]}
      >
        {aiItems.items.length > 0 && (
          <div class="ai-checklist">
            {aiItems.items.map((item, i) => (
              <label key={`${i}-${item}`}>
                <input
                  type="checkbox"
                  checked={aiItems.checked[i]}
                  disabled={aiItems.applying}
                  onChange={(e) => {
                    const on = (e.target as HTMLInputElement).checked;
                    setAiItems((a) => ({
                      ...a,
                      checked: a.checked.map((c, j) => (j === i ? on : c)),
                    }));
                  }}
                />
                <span>{item}</span>
              </label>
            ))}
          </div>
        )}
        {aiItems.note && <div class="ai-note">{aiItems.note}</div>}
      </AiPreview>
    </div>
  );
}
