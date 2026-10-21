/**
 * Left navigation tree (S2, EPIM-style): ~208px labelled module list with
 * count badges, plus Account / Settings rows and a collapse toggle. Below the
 * module rows sits the F-004 "Tree panel" secondary section — smart lists
 * (Tasks/Notes), local calendars with color dots + mini month (Calendar) —
 * and the trailing "+ Add …" link. Collapsed mode keeps the 44px icon-only
 * density (`pf.nav.collapsed`).
 */
import { useEffect, useState } from "preact/hooks";
import { signal } from "@preact/signals";
import { activeModule, counts, MODULES, refreshCounts, type ModuleId } from "../store";
import { api, type CalendarInfo, type NoteRow, type TaskRow } from "../api";
import { icon, type IconName } from "../design/icons";
import { FormDialog } from "../components/FormDialog";
import { pickMiniMonth, recentlyViewedNotes, smartListSel } from "./minimonth";
import "../design/tree.css";

const NAV_ICONS: Record<ModuleId, IconName> = {
  today: "today",
  mail: "mail",
  calendar: "calendar",
  tasks: "tasks",
  notes: "notes",
  diary: "diary",
  contacts: "contacts",
  passwords: "passwords",
  trash: "trash",
};

const collapsed = signal(localStorage.getItem("pf.nav.collapsed") === "1");

function badgeFor(id: ModuleId): number {
  const c = counts.value;
  if (!c) return 0;
  switch (id) {
    case "mail":
      return c.unread_messages;
    case "calendar":
      return c.events;
    case "tasks":
      return c.tasks;
    case "notes":
      return c.notes;
    case "contacts":
      return c.contacts;
    case "passwords":
      return c.passwords;
    case "trash":
      return c.trash;
    case "today":
      return c.pending_reminders;
    default:
      return 0;
  }
}

function badgeText(n: number): string | null {
  if (n <= 0) return null;
  return n > 999 ? "1k+" : String(n);
}

// ------------------------------------------------------- secondary sections

function SectionTitle({ children }: { children: string }) {
  return <div class="nav-section-title">{children}</div>;
}

function SmartRow({
  id,
  label,
  count,
  hint,
  onSelect,
}: {
  id: string;
  label: string;
  /** undefined = the backend cannot produce this count — render no number. */
  count?: number;
  hint?: string;
  onSelect: (id: string) => void;
}) {
  return (
    <button
      class="nav-subrow"
      data-active={smartListSel.value === id}
      title={hint ?? label}
      onClick={() => onSelect(id)}
    >
      <span class="nav-subrow-label">{label}</span>
      {count !== undefined && (
        <span class="nav-subrow-count" data-zero={count === 0}>
          {count.toLocaleString()}
        </span>
      )}
    </button>
  );
}

function TasksSection() {
  const [rows, setRows] = useState<TaskRow[]>([]);
  const [done, setDone] = useState<TaskRow[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        const [open, completed] = await Promise.all([api.tasksOpen(500), api.tasksCompleted(500)]);
        setRows(open);
        setDone(completed);
      } catch {
        setFailed(true);
      }
    })();
  }, []);

  const select = (row: string) => (smartListSel.value = `tasks:${row}`);
  const important = rows.filter((t) => t.priority > 0).length;
  const all = counts.value?.tasks;

  return (
    <div class="nav-section">
      <SectionTitle>Smart Lists</SectionTitle>
      <SmartRow id="all" label="All" count={all} hint="All tasks" onSelect={select} />
      <SmartRow
        id="important"
        label="Important"
        count={important}
        hint="Open tasks with a priority set"
        onSelect={select}
      />
      <SmartRow
        id="flagged"
        label="Flagged"
        hint="Flags are not stored for tasks yet"
        onSelect={select}
      />
      <SmartRow
        id="completed"
        label="Completed"
        count={done.length}
        hint={`Completed tasks${done.length >= 500 ? " (first 500)" : ""}`}
        onSelect={select}
      />
      <button
        class="nav-addlink"
        disabled
        title="The backend has no task-list create command yet"
      >
        + Add List
      </button>
      {failed && <div class="nav-subrow-count">counts unavailable</div>}
    </div>
  );
}

function NotesSection() {
  const [rows, setRows] = useState<NoteRow[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    void api
      .notesPage(undefined, undefined, 200)
      .then(setRows)
      .catch(() => setFailed(true));
  }, []);

  const select = (row: string) => (smartListSel.value = `notes:${row}`);
  const favorites = rows.filter((r) => r.favorite).length;
  const recentlyModified = [...rows]
    .sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1))
    .slice(0, 10).length;

  return (
    <div class="nav-section">
      <SectionTitle>Smart Lists</SectionTitle>
      <SmartRow id="all" label="All" count={counts.value?.notes} onSelect={select} />
      <SmartRow id="favorites" label="Favorites" count={favorites} onSelect={select} />
      <SmartRow
        id="recent"
        label="Recently Viewed"
        count={recentlyViewedNotes.value.length}
        hint="Notes opened this session (no module reports views yet)"
        onSelect={select}
      />
      <SmartRow
        id="modified"
        label="Recently Modified"
        count={recentlyModified}
        hint="10 most recently updated notes"
        onSelect={select}
      />
      <button
        class="nav-addlink"
        disabled
        title="The backend has no note-list create command yet"
      >
        + Add List
      </button>
      {failed && <div class="nav-subrow-count">notes unavailable</div>}
    </div>
  );
}

function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function addDays(d: Date, n: number): Date {
  const x = new Date(d);
  x.setDate(x.getDate() + n);
  return x;
}

function startOfWeek(d: Date): Date {
  return addDays(d, -((d.getDay() + 6) % 7)); // Monday
}

const DOW = ["M", "T", "W", "T", "F", "S", "S"];

/** 6x7 mini month; today highlighted; a pick is published to Calendar. */
function MiniMonth() {
  const [offset, setOffset] = useState(0); // months from today
  const base = new Date();
  const first = new Date(base.getFullYear(), base.getMonth() + offset, 1);
  const gridStart = startOfWeek(first);
  const cells = Array.from({ length: 42 }, (_, i) => addDays(gridStart, i));
  const today = ymd(new Date());

  return (
    <div class="nav-mini">
      <div class="nav-mini-title">
        <button
          aria-label="Previous month"
          onClick={() => setOffset(offset - 1)}
        >
          ‹
        </button>
        <span>
          {first.toLocaleDateString(undefined, { month: "long", year: "numeric" })}
        </span>
        <button aria-label="Next month" onClick={() => setOffset(offset + 1)}>
          ›
        </button>
      </div>
      <div class="nav-mini-head">
        {DOW.map((d, i) => (
          <span key={`${d}-${i}`}>{d}</span>
        ))}
      </div>
      <div class="nav-mini-grid">
        {cells.map((d) => (
          <button
            key={ymd(d)}
            class="nav-mini-day"
            data-outside={d.getMonth() !== first.getMonth()}
            data-today={ymd(d) === today}
            onClick={() => pickMiniMonth(ymd(d))}
          >
            {d.getDate()}
          </button>
        ))}
      </div>
    </div>
  );
}

const CAL_COLORS = ["#2f6fd0", "#2e8b57", "#c0392b", "#b8860b", "#7c5cbf", "#d97706"];

function CalendarSection() {
  const [calendars, setCalendars] = useState<CalendarInfo[]>([]);
  const [failed, setFailed] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [color, setColor] = useState(CAL_COLORS[0]);

  const reload = () => {
    void api
      .calendarsList()
      .then(setCalendars)
      .catch((e) => setFailed(String(e)));
  };
  useEffect(reload, []);

  const create = async () => {
    if (!name.trim()) return;
    try {
      await api.calendarCreate(name.trim(), color);
      setAdding(false);
      setName("");
      reload();
      void refreshCounts();
    } catch (e) {
      setFailed(String(e));
    }
  };

  return (
    <div class="nav-section">
      <SectionTitle>Local Calendars</SectionTitle>
      {calendars.map((c) => (
        <div key={c.id} class="nav-subrow" title={`${c.name} — events are shown in the Calendar module`}>
          <span class="nav-cal-dot" style={{ background: c.color ?? "var(--accent)" }} />
          <span class="nav-subrow-label">{c.name}</span>
        </div>
      ))}
      {calendars.length === 0 && !failed && (
        <div class="nav-subrow-label" style={{ padding: "2px 22px", color: "var(--fg-muted)" }}>
          No calendars
        </div>
      )}
      <button class="nav-addlink" onClick={() => setAdding(true)}>
        + Add Calendar
      </button>
      <MiniMonth />
      {failed && <div class="nav-subrow-count">{failed}</div>}

      <FormDialog
        open={adding}
        title="New Calendar"
        onClose={() => setAdding(false)}
        onOk={() => void create()}
        okLabel="Create"
        error={failed}
      >
        <div class="form-grid">
          <div class="form-row">
            <label>Name</label>
            <input value={name} onInput={(e) => setName((e.target as HTMLInputElement).value)} />
          </div>
          <div class="form-row">
            <label>Color</label>
            <div class="cal-swatch-row">
              {CAL_COLORS.map((c) => (
                <button
                  key={c}
                  class="cal-swatch"
                  style={{ background: c }}
                  data-selected={color === c}
                  aria-label={`Color ${c}`}
                  onClick={() => setColor(c)}
                />
              ))}
            </div>
          </div>
        </div>
      </FormDialog>
    </div>
  );
}

function SecondaryTree() {
  switch (activeModule.value) {
    case "tasks":
      return <TasksSection />;
    case "notes":
      return <NotesSection />;
    case "calendar":
      return <CalendarSection />;
    default:
      return null;
  }
}

// ------------------------------------------------------------------ shell

export function NavTree({
  onSwitch,
  onSettings,
}: {
  onSwitch: (m: ModuleId) => void;
  onSettings: () => void;
}) {
  return (
    <nav class="nav-tree" data-collapsed={collapsed.value} aria-label="Modules">
      {MODULES.map((m, i) => {
        const badge = badgeText(badgeFor(m.id));
        return (
          <button
            key={m.id}
            class="nav-row"
            data-active={m.id === activeModule.value}
            title={`${m.label} (Ctrl+${i + 1})`}
            aria-current={m.id === activeModule.value ? "page" : undefined}
            onClick={() => {
              smartListSel.value = null;
              onSwitch(m.id);
            }}
          >
            <span class="nav-icon" dangerouslySetInnerHTML={{ __html: icon(NAV_ICONS[m.id]) }} />
            <span class="nav-label">{m.label}</span>
            {badge && <span class="nav-badge">{badge}</span>}
          </button>
        );
      })}
      <SecondaryTree />
      <div class="nav-spacer" />
      <button class="nav-row" title="Account" aria-label="Account">
        <span class="nav-icon" dangerouslySetInnerHTML={{ __html: icon("account") }} />
        <span class="nav-label">Account</span>
      </button>
      <button class="nav-row" title="Settings" aria-label="Settings" onClick={onSettings}>
        <span class="nav-icon" dangerouslySetInnerHTML={{ __html: icon("settings") }} />
        <span class="nav-label">Settings</span>
      </button>
      <button
        class="nav-row"
        title={collapsed.value ? "Expand navigation" : "Collapse navigation"}
        aria-label={collapsed.value ? "Expand navigation" : "Collapse navigation"}
        onClick={() => {
          collapsed.value = !collapsed.value;
          localStorage.setItem("pf.nav.collapsed", collapsed.value ? "1" : "0");
        }}
      >
        <span class="nav-icon" dangerouslySetInnerHTML={{ __html: icon("menu") }} />
        <span class="nav-label">Collapse</span>
      </button>
    </nav>
  );
}
