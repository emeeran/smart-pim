/**
 * Calendar module (S28–S34 UI slice): Month / Week / Day grids over
 * RRULE-expanded occurrences, the shared EventDialog editor, calendar sidebar
 * with colors + mini month (F-004), ICS import (file picker) / export.
 */
import { useEffect, useMemo, useState } from "preact/hooks";
import {
  api,
  type CalendarInfo,
  type Occurrence,
} from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { EventDialog, type EventDraft } from "../components/EventDialog";
import { FormDialog } from "../components/FormDialog";
import { AiPreview } from "../components/AiPreview";
import { aiFeatureEnabled } from "../lib/ai";
import { miniMonthPick } from "../shell/minimonth";
import { refreshCounts, viewCount } from "../store";

type ViewMode = "month" | "week" | "day" | "year" | "agenda";

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

// ------------------------------------------------------------------ views

function DayCells({
  days,
  occurrences,
  onOpen,
  hourSpan,
}: {
  days: Date[];
  occurrences: Occurrence[];
  onOpen: (date: Date, occ?: Occurrence) => void;
  hourSpan: boolean;
}) {
  const byDay = new Map<string, Occurrence[]>();
  for (const o of occurrences) {
    const key = o.start.slice(0, 10);
    (byDay.get(key) ?? byDay.set(key, []).get(key)!).push(o);
  }
  return (
    <div class={`cal-grid ${hourSpan ? "cal-week" : "cal-month"}`}>
      {days.map((d) => {
        const key = ymd(d);
        const items = byDay.get(key) ?? [];
        const today = key === ymd(new Date());
        const outside = !hourSpan && d.getMonth() !== days[6].getMonth();
        return (
          <div key={key} class="cal-day" data-today={today} data-outside={outside}>
            <div class="cal-day-head">
              <span class="cal-day-num">{d.getDate()}</span>
            </div>
            <div class="cal-day-items">
              {items.map((o) => {
                const when = o.all_day ? "all day" : o.start.slice(11, 16) + "Z";
                return (
                  <button
                    key={o.event_id + "-" + o.start}
                    class="cal-event"
                    style={{ borderLeftColor: o.calendar_color ?? "var(--accent)" }}
                    title={o.title + " (" + when + ")"}
                    onClick={() => onOpen(d, o)}
                  >
                    {!o.all_day && <span class="cal-event-time">{o.start.slice(11, 16)}</span>}
                    <span class="cal-event-title">{o.title}</span>
                  </button>
                );
              })}
            </div>
            <button class="cal-day-add" title="New event" onClick={() => onOpen(d)}>+</button>
          </div>
        );
      })}
    </div>
  );
}

// ----------------------------------------------------------------- module

export function CalendarModule() {
  const [mode, setMode] = useState<ViewMode>(
    () => (localStorage.getItem("pf.cal.view") as ViewMode) || "month",
  );
  const [anchor, setAnchor] = useState(new Date());
  const [agendaDays, setAgendaDays] = useState(30);
  const [calendars, setCalendars] = useState<CalendarInfo[]>([]);
  const [occurrences, setOccurrences] = useState<Occurrence[]>([]);
  const [editor, setEditor] = useState<EventDraft | null>(null);
  const [importMsg, setImportMsg] = useState<string | null>(null);
  const [slotOpen, setSlotOpen] = useState(false);
  const [slotBusy, setSlotBusy] = useState(false);
  const [slotError, setSlotError] = useState<string | null>(null);
  const [slots, setSlots] = useState<{ start: string; end: string }[]>([]);
  const [slotMinutes, setSlotMinutes] = useState(60);
  const [addingCalendar, setAddingCalendar] = useState(false);
  const [calendarName, setCalendarName] = useState("");
  const [calendarColor, setCalendarColor] = useState("#2f6fd0");
  /** Per-calendar visibility (EPIM checkbox column), client-side filter. */
  const [hidden, setHidden] = useState<Set<number>>(() => {
    try {
      return new Set(JSON.parse(localStorage.getItem("pf.cal.hidden") ?? "[]") as number[]);
    } catch {
      return new Set();
    }
  });

  const toggleCalendar = (id: number) => {
    setHidden((h) => {
      const next = new Set(h);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      localStorage.setItem("pf.cal.hidden", JSON.stringify([...next]));
      return next;
    });
  };

  const range = useMemo(() => {
    if (mode === "day") return { from: anchor, to: anchor };
    if (mode === "week") return { from: startOfWeek(anchor), to: addDays(startOfWeek(anchor), 6) };
    if (mode === "year") {
      const jan1 = new Date(anchor.getFullYear(), 0, 1);
      return { from: jan1, to: new Date(anchor.getFullYear(), 11, 31) };
    }
    if (mode === "agenda") return { from: anchor, to: addDays(anchor, agendaDays - 1) };
    const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
    const gridStart = startOfWeek(first);
    return { from: gridStart, to: addDays(gridStart, 41) };
  }, [mode, anchor, agendaDays]);

  const reload = async () => {
    try {
      const cals = await api.calendarsList();
      setCalendars(cals);
      const fromIso = `${ymd(range.from)}T00:00:00+00:00`;
      const toIso = `${ymd(range.to)}T23:59:59+00:00`;
      setOccurrences(await api.eventsWindow(null, fromIso, toIso));
    } catch {
      /* locked */
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ymd(range.from), mode]);

  useEffect(() => {
    viewCount.value = occurrences.length;
  }, [occurrences.length]);

  // Nav-tree mini month: jump to the picked date in Day view.
  useEffect(() => {
    const picked = miniMonthPick.value;
    if (!picked) return;
    setAnchor(new Date(`${picked}T00:00:00`));
    setMode("day");
    localStorage.setItem("pf.cal.view", "day");
  }, [miniMonthPick.value]);

  const visibleOccurrences = useMemo(
    () => occurrences.filter((o) => !hidden.has(o.calendar_id)),
    [occurrences, hidden],
  );

  useEffect(() => {
    viewCount.value = visibleOccurrences.length;
  }, [visibleOccurrences.length]);

  const days =
    mode === "day" ? [anchor] : mode === "week" ? Array.from({ length: 7 }, (_, i) => addDays(startOfWeek(anchor), i)) : mode === "year" ? [] : Array.from({ length: 42 }, (_, i) => addDays(range.from, i));

  const setView = (m: ViewMode) => {
    setMode(m);
    localStorage.setItem("pf.cal.view", m);
  };

  const move = (dir: number) => {
    if (mode === "year") {
      setAnchor(new Date(anchor.getFullYear() + dir, anchor.getMonth(), 1));
    } else if (mode === "agenda") {
      setAnchor(addDays(anchor, dir * agendaDays));
    } else {
      setAnchor(addDays(anchor, dir * (mode === "day" ? 1 : mode === "week" ? 7 : 28)));
    }
  };

  const title = mode === "year"
    ? String(anchor.getFullYear())
    : mode === "day"
      ? anchor.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric", year: "numeric" })
      : anchor.toLocaleDateString(undefined, { month: "long", year: "numeric" });

  const openEditor = (date: Date, occ?: Occurrence) => {
    if (occ) {
      setEditor({
        id: occ.event_id,
        title: occ.title,
        start: occ.start,
        end: occ.end,
        allDay: occ.all_day,
        location: occ.location,
        description: null,
        rrule: null,
        reminderMinutes: null,
      });
    } else {
      setEditor({
        id: null,
        title: "",
        start: `${ymd(date)}T00:00:00`,
        end: null,
        allDay: false,
        location: null,
        description: null,
        rrule: null,
        reminderMinutes: null,
      });
    }
  };

  const createCalendar = async () => {
    if (!calendarName.trim()) return;
    try {
      await api.calendarCreate(calendarName.trim(), calendarColor);
      setAddingCalendar(false);
      setCalendarName("");
      void reload();
    } catch (e) {
      setImportMsg(`Could not create calendar: ${e}`);
    }
  };

  const importIcs = async (file: File) => {
    try {
      const text = await file.text();
      const res = await api.eventsImportIcs(text);
      setImportMsg(`Imported ${res.imported} event(s)${res.warnings.length ? `, ${res.warnings.length} warning(s)` : ""}.`);
      void reload();
      void refreshCounts();
    } catch (e) {
      setImportMsg(`Import failed: ${e}`);
    }
  };

  const findSlots = async (minutes: number) => {
    setSlotBusy(true);
    setSlotError(null);
    try {
      const from = new Date(anchor);
      from.setHours(0, 0, 0, 0);
      const to = new Date(from);
      to.setDate(to.getDate() + 14);
      const fmt = (d: Date) => d.toISOString();
      setSlots(await api.freeSlots(null, fmt(from), fmt(to), minutes));
    } catch (e) {
      setSlots([]);
      setSlotError(String(e));
    } finally {
      setSlotBusy(false);
    }
  };

  useToolbar(
    <div class="cal-toolbar">
      <button onClick={() => move(-1)}>‹</button>
      <button onClick={() => setAnchor(new Date())}>Today</button>
      <button onClick={() => move(1)}>›</button>
      <span class="cal-title">{title}</span>
      <span class="cal-toolbar-spacer" />
      {mode === "agenda" && (
        <select
          value={agendaDays}
          onChange={(e) => setAgendaDays(Number((e.target as HTMLSelectElement).value))}
          aria-label="Agenda horizon"
        >
          {[7, 14, 30, 60].map((n) => (
            <option key={n} value={n}>{n} days</option>
          ))}
        </select>
      )}
      {aiFeatureEnabled("capture") && (
        <button
          onClick={() => {
            setSlotOpen(true);
            void findSlots(slotMinutes);
          }}
        >
          Find slot
        </button>
      )}
      {(["day", "week", "month", "year", "agenda"] as ViewMode[]).map((m) => (
        <button key={m} data-active={mode === m} onClick={() => setView(m)}>
          {m[0].toUpperCase() + m.slice(1)}
        </button>
      ))}
    </div>,
    [mode, anchor, agendaDays],
  );

  return (
    <div class="calendar-module">
      <ThreePane
        module="calendar"
        tree={
          <nav class="smartlists" aria-label="Calendars">
            {calendars.map((c) => (
              <div key={c.id} class="smartlist-entry cal-sidebar-entry">
                <input
                  type="checkbox"
                  checked={!hidden.has(c.id)}
                  title={`Show ${c.name} in the calendar`}
                  onChange={() => toggleCalendar(c.id)}
                />
                <span class="cal-color-dot" style={{ background: c.color ?? "var(--accent)" }} />
                <span>{c.name}</span>
              </div>
            ))}
            <button class="smartlist-entry" onClick={() => setAddingCalendar(true)}>
              + New calendar
            </button>
            <label class="smartlist-entry" style={{ cursor: "pointer" }}>
              Import ICS…
              <input
                type="file"
                accept=".ics,text/calendar"
                style={{ display: "none" }}
                onChange={(e) => {
                  const f = (e.target as HTMLInputElement).files?.[0];
                  if (f) void importIcs(f);
                }}
              />
            </label>
            <button
              class="smartlist-entry"
              onClick={async () => {
                const path = await api.eventsExportIcs();
                setImportMsg(`Exported to ${path}`);
              }}
            >
              Export all to ICS
            </button>
            {importMsg && <div class="muted cal-import-msg">{importMsg}</div>}
          </nav>
        }
        list={
          <div class="cal-main">
            {mode === "year" ? (
              <YearGrid
                year={anchor.getFullYear()}
                occurrences={visibleOccurrences}
                onPickDay={(d) => {
                  setAnchor(d);
                  setView("month");
                }}
              />
            ) : mode === "agenda" ? (
              <AgendaList occurrences={visibleOccurrences} onOpen={openEditor} />
            ) : (
              <DayCells days={days} occurrences={visibleOccurrences} onOpen={openEditor} hourSpan={mode !== "month"} />
            )}
          </div>
        }
        detail={
          <div class="muted empty-pane">
            <b>Upcoming</b>
            <ul class="cal-upcoming">
              {visibleOccurrences.slice(0, 12).map((o) => (
                <li key={`${o.event_id}-${o.start}`}>
                  {o.all_day
                    ? o.start
                    : new Date(o.start).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}
                  {" — "}
                  {o.title}
                </li>
              ))}
            </ul>
          </div>
        }
      />
      {editor && (
        <EventDialog
          draft={editor}
          onClose={() => setEditor(null)}
          onSaved={() => {
            void reload();
            void refreshCounts();
          }}
        />
      )}
      <FormDialog
        open={addingCalendar}
        title="New Calendar"
        onClose={() => setAddingCalendar(false)}
        onOk={() => void createCalendar()}
        okLabel="Create"
      >
        <div class="form-grid">
          <div class="form-row">
            <label>Name</label>
            <input
              value={calendarName}
              onInput={(e) => setCalendarName((e.target as HTMLInputElement).value)}
            />
          </div>
          <div class="form-row">
            <label>Color</label>
            <div class="cal-swatch-row">
              {["#2f6fd0", "#2e8b57", "#c0392b", "#b8860b", "#7c5cbf", "#d97706"].map((c) => (
                <button
                  key={c}
                  class="cal-swatch"
                  style={{ background: c }}
                  data-selected={calendarColor === c}
                  aria-label={`Color ${c}`}
                  onClick={() => setCalendarColor(c)}
                />
              ))}
            </div>
          </div>
        </div>
      </FormDialog>
      <AiPreview
        open={slotOpen}
        title={`Free slots - next 14 days (${slotMinutes} min)`}
        body={
          slots.length === 0
            ? slotBusy
              ? "Looking..."
              : "No free slot found in work hours (08:00-18:00 UTC)."
            : slots.map((sl) => `${sl.start}  ->  ${sl.end}`).join("\n")
        }
        busy={slotBusy}
        error={slotError}
        actions={[
          {
            label: "30 min",
            onClick: () => {
              setSlotMinutes(30);
              void findSlots(30);
            },
          },
          {
            label: "60 min",
            onClick: () => {
              setSlotMinutes(60);
              void findSlots(60);
            },
          },
        ]}
        onClose={() => setSlotOpen(false)}
      />
    </div>

  );
}

// ------------------------------------------------------------- year / agenda

/** 12 mini-months; day tint intensity maps to occurrence count. */
function YearGrid({
  year,
  occurrences,
  onPickDay,
}: {
  year: number;
  occurrences: Occurrence[];
  onPickDay: (d: Date) => void;
}) {
  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const o of occurrences) {
      const k = o.start.slice(0, 10);
      m.set(k, (m.get(k) ?? 0) + 1);
    }
    return m;
  }, [occurrences]);

  const months = Array.from({ length: 12 }, (_, i) => {
    const first = new Date(year, i, 1);
    const gridStart = startOfWeek(first);
    return {
      first,
      cells: Array.from({ length: 42 }, (_, j) => addDays(gridStart, j)),
    };
  });

  return (
    <div class="year-grid">
      {months.map((mo) => (
        <div key={mo.first.getMonth()} class="mini-month">
          <div class="mini-month-title">
            {mo.first.toLocaleDateString(undefined, { month: "long" })}
          </div>
          <div class="mini-month-grid">
            {mo.cells.map((d) => {
              const inMonth = d.getMonth() === mo.first.getMonth();
              const n = counts.get(ymd(d)) ?? 0;
              return (
                <button
                  key={ymd(d)}
                  class="mini-day"
                  data-outside={!inMonth}
                  data-count={Math.min(n, 3)}
                  title={n > 0 ? `${n} event${n > 1 ? "s" : ""}` : undefined}
                  onClick={() => onPickDay(d)}
                >
                  {d.getDate()}
                </button>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

/** Flat day-grouped list of upcoming occurrences. */
function AgendaList({
  occurrences,
  onOpen,
}: {
  occurrences: Occurrence[];
  onOpen: (date: Date, occ?: Occurrence) => void;
}) {
  const sorted = [...occurrences].sort((a, b) => (a.start < b.start ? -1 : 1));
  let lastDay = "";
  return (
    <div class="agenda-list">
      {sorted.length === 0 && <p class="muted empty-pane">Nothing scheduled in this range.</p>}
      {sorted.map((o) => {
        const day = o.start.slice(0, 10);
        const header = day !== lastDay ? day : null;
        lastDay = day;
        return (
          <div key={`${o.event_id}-${o.start}`}>
            {header && <div class="list-date-header">{header}</div>}
            <button class="agenda-row" onClick={() => onOpen(new Date(o.start), o)}>
              <span class="agenda-time">
                {o.all_day
                  ? "all day"
                  : o.start.slice(11, 16)}
              </span>
              <span class="agenda-title">{o.title}</span>
              {o.location && <span class="muted agenda-loc">{o.location}</span>}
            </button>
          </div>
        );
      })}
    </div>
  );
}
