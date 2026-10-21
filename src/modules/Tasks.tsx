/**
 * Tasks module (S36–S38 UI slice): List and Kanban views, task editor with
 * priority/due/percent, completion via 100%. Gate 4: inline "Add a task"
 * row in the table view, kanban column counts with per-column "+" and
 * cards carrying a priority "!" and progress bar.
 */
import { useEffect, useState } from "preact/hooks";
import "../design/panels.css";
import { api, type TaskRow, type TaskTreeRow } from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { VirtualList } from "../components/VirtualList";
import { errText, featureEnabled, parseNlTask, parseSubtasks, runAiJob, useAiStatus } from "../lib/ai";
import { AiPreview } from "../components/AiPreview";
import { jumpTarget, viewCount } from "../store";

const ROW_H = 48;

/** Grid columns after Subject — click a header to hide it (EPIM column
 * chooser); hidden ones come back from the toolbar's Columns menu. */
const TASK_COLUMNS: { key: "due" | "priority" | "percent"; label: string }[] = [
  { key: "due", label: "Due" },
  { key: "priority", label: "!" },
  { key: "percent", label: "%" },
];

function loadHiddenCols(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem("pf.tasks.cols") ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

interface EditorTask {
  id: number | null;
  /** `item` row id — the AI job kinds key off items, not task rows. */
  itemId: number | null;
  title: string;
  notes: string;
  due: string; // datetime-local
  priority: number;
  percent: number;
  parentId: number | null;
}

const EMPTY: EditorTask = { id: null, itemId: null, title: "", notes: "", due: "", priority: 0, percent: 0, parentId: null };

/** Inline "Add a task" row (EPIM table pattern): title-only, Enter saves. */
function AddTaskRow({ onSaved }: { onSaved: () => void }) {
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    const t = title.trim();
    if (!t || busy) return;
    setBusy(true);
    try {
      await api.taskSave({
        id: null,
        title: t,
        notes: null,
        due_utc: null,
        priority: 0,
        percent: 0,
      });
      setTitle("");
      onSaved();
    } catch (e) {
      alert(`Could not add task: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="task-add-row">
      <span class="muted" aria-hidden="true">＋</span>
      <input
        placeholder="Add a task"
        value={title}
        disabled={busy}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void submit();
          }
        }}
        onInput={(e) => setTitle((e.target as HTMLInputElement).value)}
      />
      <span class="muted">Enter ↵</span>
    </div>
  );
}

/** Priority "!" badge — EPIM flags High/Urgent tasks in the card corner. */
function PrioBang({ priority }: { priority: number }) {
  if (priority < 2) return null;
  return (
    <span class="prio-bang" data-prio={priority} title={`Priority ${priority}`}>
      !
    </span>
  );
}

function TaskEditor({
  task,
  candidates,
  onClose,
  onSaved,
}: {
  task: EditorTask;
  candidates: TaskRow[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const [form, setForm] = useState(task);
  const set = (p: Partial<EditorTask>) => setForm((f) => ({ ...f, ...p }));
  const status = useAiStatus();
  const canSplit = featureEnabled(status, "subtasks") && task.itemId !== null;
  // AI subtask proposal: editable lines, applied only on Apply.
  const [split, setSplit] = useState<{
    open: boolean;
    busy: boolean;
    error: string | null;
    items: string[];
  }>({ open: false, busy: false, error: null, items: [] });

  const runSplit = async () => {
    if (task.itemId === null) return;
    setSplit({ open: true, busy: true, error: null, items: [] });
    try {
      const items = parseSubtasks(await runAiJob("subtask_split", { task_id: task.itemId }));
      setSplit({ open: true, busy: false, error: items.length ? null : "The model returned no subtasks.", items });
    } catch (e) {
      setSplit({ open: true, busy: false, error: errText(e), items: [] });
    }
  };

  const applySplit = async () => {
    const parent = task.id;
    if (parent === null || !split.items.length) return;
    for (const title of split.items.map((s) => s.trim()).filter(Boolean)) {
      const childId = await api.taskSave({
        id: null,
        title,
        notes: null,
        due_utc: null,
        priority: form.priority,
        percent: 0,
      });
      await api.taskSetParent(childId, parent);
    }
    setSplit({ open: false, busy: false, error: null, items: [] });
    onSaved();
  };

  const save = async () => {
    if (!form.title.trim()) return;
    const id = await api.taskSave({
      id: form.id,
      title: form.title,
      notes: form.notes || null,
      due_utc: form.due ? new Date(form.due).toISOString() : null,
      priority: form.priority,
      percent: form.percent,
    });
    if (form.id !== null) {
      await api.taskSetParent(id, form.parentId);
    }
    onSaved();
    onClose();
  };

  return (
    <div class="overlay" onClick={onClose}>
      <div class="dialog" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Task">
        <h3>{form.id === null ? "New Task" : "Edit Task"}</h3>
        <label class="settings-row">Title <input value={form.title} onInput={(e) => set({ title: (e.target as HTMLInputElement).value })} /></label>
        <label class="settings-row">
          Due
          <input type="datetime-local" value={form.due} onInput={(e) => set({ due: (e.target as HTMLInputElement).value })} />
        </label>
        <label class="settings-row">
          Priority
          <select value={form.priority} onChange={(e) => set({ priority: Number((e.target as HTMLSelectElement).value) })}>
            <option value={0}>Low</option>
            <option value={1}>Normal</option>
            <option value={2}>High</option>
            <option value={3}>Urgent</option>
          </select>
        </label>
        {form.id !== null && (
          <label class="settings-row">
            Parent task
            <select
              value={form.parentId ?? ""}
              onChange={(e) => {
                const v = (e.target as HTMLSelectElement).value;
                set({ parentId: v === "" ? null : Number(v) });
              }}
            >
              <option value="">— none (top level) —</option>
              {candidates
                .filter((c) => c.id !== form.id)
                .map((c) => (
                  <option key={c.id} value={c.id}>{c.title}</option>
                ))}
            </select>
          </label>
        )}
        <label class="settings-row">
          % complete
          <input
            type="number"
            min={0}
            max={100}
            value={form.percent}
            onInput={(e) => set({ percent: Number((e.target as HTMLInputElement).value) })}
          />
        </label>
        <textarea
          class="compose-body"
          placeholder="Notes"
          value={form.notes}
          onInput={(e) => set({ notes: (e.target as HTMLTextAreaElement).value })}
        />
        {canSplit && (
          <button onClick={() => void runSplit()} title="Break this task into subtasks with AI">
            Suggest subtasks…
          </button>
        )}
        <div class="dialog-actions">
          {form.id !== null && (
            <button
              class="danger-text"
              onClick={async () => {
                if (!confirm("Move this task to trash?")) return;
                await api.taskDelete(form.id!);
                onSaved();
                onClose();
              }}
            >
              Delete
            </button>
          )}
          <button onClick={onClose}>Cancel</button>
          <button class="primary" onClick={() => void save()}>Save</button>
        </div>
      </div>
      <AiPreview
        open={split.open}
        title="Suggested subtasks"
        onClose={() => setSplit({ open: false, busy: false, error: null, items: [] })}
        busy={split.busy}
        busyLabel="Splitting…"
        error={split.error}
        actions={[
          {
            label: `Add ${split.items.filter((s) => s.trim()).length} subtasks`,
            primary: true,
            disabled: split.busy || !split.items.some((s) => s.trim()),
            onClick: () => void applySplit(),
          },
        ]}
      >
        <label class="settings-row">
          Proposed subtasks (one per line — edit before applying)
          <textarea
            class="compose-body"
            rows={5}
            placeholder="No proposal yet"
            value={split.items.join("\n")}
            onInput={(e) =>
              setSplit((s) => ({
                ...s,
                items: (e.target as HTMLTextAreaElement).value.split("\n"),
              }))
            }
          />
        </label>
        <div class="ai-note">
          Each line becomes a subtask under “{task.title}”. Nothing is added until you apply.
        </div>
      </AiPreview>
    </div>
  );
}

export function TasksModule() {
  const [view, setView] = useState<"list" | "tree" | "kanban">("list");
  const [open, setOpen] = useState<TaskRow[]>([]);
  const [completed, setCompleted] = useState<TaskRow[]>([]);
  const [tree, setTree] = useState<TaskTreeRow[]>([]);
  const [editor, setEditor] = useState<EditorTask | null>(null);

  const reload = async () => {
    try {
      setOpen(await api.tasksOpen());
      setCompleted(await api.tasksCompleted());
      api.tasksTree().then(setTree).catch(() => setTree([]));
    } catch {
      /* locked */
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  // Related Items / Today click-through: open the jumped task's editor.
  useEffect(() => {
    const jt = jumpTarget.value;
    if (jt?.kind !== "task") return;
    jumpTarget.value = null;
    const find = async () => {
      const all = [...open, ...(await api.tasksCompleted().catch(() => []))];
      const t = all.find((x) => x.id === jt.id || x.item_id === jt.id);
      if (t) {
        setEditor({
          id: t.id,
          itemId: t.item_id,
          title: t.title,
          notes: t.notes ?? "",
          due: t.due_utc ? t.due_utc.slice(0, 16) : "",
          priority: t.priority,
          percent: t.percent,
          parentId: null,
        });
      }
    };
    void find();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jumpTarget.value]);

  useEffect(() => {
    viewCount.value = open.length;
  }, [open.length]);

  /** EPIM indent (→): make the row a child of the visible row above.
   * Outdent (⇤): promote it to its grandparent. Pure tree moves through
   * the existing `task_set_parent` (cycle-safe in the backend). */
  const indent = async (index: number) => {
    const above = tree[index - 1];
    const row = tree[index];
    if (!above || !row || row.parent_id === above.id) return;
    await api.taskSetParent(row.id, above.id);
    void reload();
  };
  const outdent = async (index: number) => {
    const row = tree[index];
    if (!row || row.parent_id === null) return;
    const parent = tree.find((t) => t.id === row.parent_id);
    await api.taskSetParent(row.id, parent?.parent_id ?? null);
    void reload();
  };

  const rows = view === "list" ? open : view === "kanban" ? completed : [];

  // -------------------------------------------------- hideable columns
  const [hiddenCols, setHiddenCols] = useState<string[]>(loadHiddenCols);
  const toggleCol = (key: string) =>
    setHiddenCols((h) => {
      const next = h.includes(key) ? h.filter((k) => k !== key) : [...h, key];
      localStorage.setItem("pf.tasks.cols", JSON.stringify(next));
      return next;
    });
  const show = (key: string) => !hiddenCols.includes(key);
  /** Grid header: click a label to hide that column. */
  const colHeader = (
    <div class="task-header-row" role="row">
      <span class="task-row-title">Subject</span>
      {TASK_COLUMNS.filter((c) => show(c.key)).map((c) => (
        <button
          key={c.key}
          class="task-col-head"
          title={`Hide the ${c.label} column (restore from Columns)`}
          onClick={() => toggleCol(c.key)}
        >
          {c.label}
        </button>
      ))}
    </div>
  );

  // -------------------------------------------------- AI natural-language capture
  // "call the dentist next week, urgent" → parsed fields shown in the
  // existing editor for correction. Nothing is saved from the model output
  // without the user pressing Save.
  const status = useAiStatus();
  const canCapture = featureEnabled(status, "capture");
  const [nl, setNl] = useState("");
  const [nlBusy, setNlBusy] = useState(false);

  const captureNl = async () => {
    const text = nl.trim();
    if (!text || nlBusy) return;
    setNlBusy(true);
    try {
      const parsed = parseNlTask(await runAiJob("nl_task", { text }));
      if (parsed === null) {
        alert("Could not read a task from that text — try rephrasing it.");
        return;
      }
      setEditor({
        id: null,
        itemId: null,
        title: parsed.title,
        notes: "",
        due: parsed.due_utc ? parsed.due_utc.slice(0, 16) : "",
        priority: parsed.priority,
        percent: 0,
        parentId: null,
      });
      setNl("");
    } catch (e) {
      alert(`AI capture failed: ${errText(e)}`);
    } finally {
      setNlBusy(false);
    }
  };

  useToolbar(
    <div class="mail-toolbar">
      <button class="primary" onClick={() => setEditor({ ...EMPTY })}>
        New Task
      </button>
      {canCapture && (
        <>
          <input
            class="mail-search"
            placeholder="Capture with AI: “call the dentist next week, urgent”"
            value={nl}
            disabled={nlBusy}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void captureNl();
              }
            }}
            onInput={(e) => setNl((e.target as HTMLInputElement).value)}
          />
          <button disabled={nlBusy || !nl.trim()} onClick={() => void captureNl()}>
            {nlBusy ? "Parsing…" : "Capture"}
          </button>
        </>
      )}
      <details class="col-menu">
        <summary title="Show or hide grid columns">
          Columns{hiddenCols.length > 0 ? ` (${hiddenCols.length} hidden)` : ""}
        </summary>
        <div class="col-menu-body">
          {TASK_COLUMNS.map((c) => (
            <label key={c.key}>
              <input
                type="checkbox"
                checked={show(c.key)}
                onChange={() => toggleCol(c.key)}
              />
              {c.label}
            </label>
          ))}
        </div>
      </details>
    </div>,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [canCapture, nl, nlBusy, hiddenCols],
  );

  return (
    <div class="tasks-module">
      <ThreePane
        module="tasks"
        tree={
          <nav class="smartlists" aria-label="Task lists">
            <button class="smartlist-entry" data-active={view === "list"} onClick={() => setView("list")}>
              <span>All Tasks</span>
              <span class="smartlist-count">{open.length}</span>
            </button>
            <button class="smartlist-entry" data-active={view === "tree"} onClick={() => setView("tree")}>
              <span>Tree</span>
              <span class="smartlist-count">{tree.length}</span>
            </button>
            <button class="smartlist-entry" data-active={view === "kanban"} onClick={() => setView("kanban")}>
              <span>Kanban</span>
            </button>
          </nav>
        }
        list={
          view === "tree" ? (
            <div class="mail-list">
              <AddTaskRow onSaved={() => void reload()} />
              {colHeader}
              {tree.length === 0 ? (
                <p class="muted empty-pane">No tasks here yet.</p>
              ) : (
                tree.map((t, i) => (
                  <div
                    key={t.id}
                    class="task-row"
                    style={{ height: `${ROW_H}px`, paddingLeft: `${16 + t.depth * 20}px` }}
                    onClick={() =>
                      setEditor({
                        id: t.id,
                        itemId: t.item_id,
                        title: t.title,
                        notes: t.notes ?? "",
                        due: t.due_utc ? t.due_utc.slice(0, 16) : "",
                        priority: t.priority,
                        percent: t.percent,
                        parentId: t.parent_id,
                      })
                    }
                  >
                    {show("priority") && (
                      <span class={`prio prio-${t.priority}`} title={`Priority ${t.priority}`} />
                    )}
                    <span class="task-row-title" data-done={t.completed_at !== null}>
                      {t.depth > 0 && <span class="muted">↳ </span>}
                      {t.title}
                    </span>
                    {show("percent") && t.percent > 0 && (
                      <span class="task-row-due muted">{t.percent}%</span>
                    )}
                    {show("due") && (
                      <span class="task-row-due muted">
                        {t.due_utc ? t.due_utc.slice(0, 10) : ""}
                      </span>
                    )}
                    <span class="task-tree-actions">
                      <button
                        title="Outdent (promote one level)"
                        disabled={t.depth === 0}
                        onClick={(e) => {
                          e.stopPropagation();
                          void outdent(i);
                        }}
                      >
                        ⇤
                      </button>
                      <button
                        title="Indent (make a child of the row above)"
                        disabled={i === 0}
                        onClick={(e) => {
                          e.stopPropagation();
                          void indent(i);
                        }}
                      >
                        ⇥
                      </button>
                    </span>
                  </div>
                ))
              )}
            </div>
          ) : view === "kanban" ? (
            <div class="kanban">
              {[
                {
                  label: "To Do",
                  items: open.filter((t) => t.percent <= 0),
                  preset: 0,
                },
                {
                  label: "In Progress",
                  items: open.filter((t) => t.percent > 0 && t.percent < 100),
                  preset: 50,
                },
                { label: "Done", items: completed, preset: 100 },
              ].map((col) => (
                <div class="kanban-col" key={col.label}>
                  <div class="kanban-col-title">
                    {col.label}{" "}
                    <span class="smartlist-count">{col.items.length}</span>
                    <button
                      class="kanban-col-add"
                      title={`New task in ${col.label}`}
                      onClick={() =>
                        setEditor({ ...EMPTY, percent: col.preset })
                      }
                    >
                      +
                    </button>
                  </div>
                  {col.items.map((t) => (
                    <button
                      key={t.id}
                      class="kanban-card"
                      onClick={() =>
                        setEditor({
                          id: t.id,
                          itemId: t.item_id,
                          title: t.title,
                          notes: t.notes ?? "",
                          due: t.due_utc ? t.due_utc.slice(0, 16) : "",
                          priority: t.priority,
                          percent: t.percent,
                          parentId: null,
                        })
                      }
                    >
                      <span class="kanban-card-top">
                        <PrioBang priority={t.priority} />
                        <span class="kanban-card-title">{t.title}</span>
                        <span class="kanban-card-percent">{t.percent}%</span>
                      </span>
                      <span class="kanban-card-bar">
                        <span
                          style={{ width: `${Math.min(100, Math.max(0, t.percent))}%` }}
                        />
                      </span>
                      {t.due_utc && (
                        <span class="kanban-card-due">{t.due_utc.slice(0, 10)}</span>
                      )}
                    </button>
                  ))}
                </div>
              ))}
            </div>
          ) : (
            <div class="mail-list">
              <AddTaskRow onSaved={() => void reload()} />
              {colHeader}
              {rows.length === 0 ? (
                <p class="muted empty-pane">No tasks here.</p>
              ) : (
                <VirtualList
                  rowCount={rows.length}
                  rowHeight={ROW_H}
                  renderRow={(i) => {
                    const t = rows[i];
                    return (
                      <div key={t.id} class="task-row" style={{ height: `${ROW_H}px` }}
                        onClick={() =>
                          setEditor({
                            id: t.id,
                            itemId: t.item_id,
                            title: t.title,
                            notes: t.notes ?? "",
                            due: t.due_utc ? t.due_utc.slice(0, 16) : "",
                            priority: t.priority,
                            percent: t.percent,
                            parentId: null,
                          })
                        }
                      >
                        {show("priority") && (
                          <span class={`prio prio-${t.priority}`} title={`Priority ${t.priority}`} />
                        )}
                        <span class="task-row-title" data-done={t.completed_at !== null}>
                          {t.title}
                        </span>
                        {show("percent") && t.percent > 0 && (
                          <span class="task-row-due muted">{t.percent}%</span>
                        )}
                        {show("due") && (
                          <span class="task-row-due muted">
                            {t.due_utc ? t.due_utc.slice(0, 10) : ""}
                          </span>
                        )}
                      </div>
                    );
                  }}
                />
              )}
            </div>
          )
        }
        detail={
          <div class="muted empty-pane">
            Task details (custom fields, subtasks, reminders) arrive with the full
            task module — editing works from the list and Kanban cards.
          </div>
        }
      />
      {editor && (
        <TaskEditor
          task={editor}
          candidates={open}
          onClose={() => setEditor(null)}
          onSaved={() => void reload()}
        />
      )}
    </div>
  );
}
