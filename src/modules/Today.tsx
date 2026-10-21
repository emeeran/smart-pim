/**
 * Today dashboard (S54 / EPIM clone): a live widget board over one
 * `today_dashboard` snapshot — Agenda (today's events), TO DO (due +
 * overdue tasks, checkable in place), Messages (per-account unread with
 * click-through), Anniversaries (next birthdays/anniversaries with ages).
 * Deliberately no weather widget: local-first means no background network
 * calls (docs/ai_integ_plan.md §1).
 */
import { useEffect, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import { api, type TodayDashboard } from "../api";
import { activeModule, counts, jumpTo, MODULES, type ModuleId } from "../store";

function fmtTime(iso: string): string {
  if (/^\d{4}-\d{2}-\d{2}$/.test(iso)) return "all-day";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function fmtWhen(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString([], { month: "short", day: "numeric" });
}

function Widget({
  title,
  count,
  children,
  empty,
}: {
  title: string;
  count?: number;
  children: ComponentChildren;
  empty: boolean;
}) {
  return (
    <section class="today-widget">
      <header class="today-widget-head">
        <h3>{title}</h3>
        {count !== undefined && <span class="smartlist-count">{count}</span>}
      </header>
      {empty ? <p class="muted today-widget-empty">Nothing here.</p> : children}
    </section>
  );
}

export function Today() {
  const c = counts.value;
  const [dash, setDash] = useState<TodayDashboard | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    const today = new Date();
    const iso = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
    void api
      .todayDashboard(iso)
      .then((d) => {
        setDash(d);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  };

  useEffect(() => {
    load();
  }, []);

  // Toggle a Today-widget task to 100% (or back to 0) in place — the same
  // task_save path the editor uses, so completion and undo still apply.
  const toggleTask = async (id: number, done: boolean) => {
    const t = dash?.tasks.find((x) => x.id === id);
    if (!t) return;
    try {
      await api.taskSave({
        id: t.id,
        title: t.title,
        notes: t.notes,
        due_utc: t.due_utc,
        priority: t.priority,
        percent: done ? 100 : 0,
      });
      load();
    } catch (e) {
      setError(String(e));
    }
  };

  const cards: { id: ModuleId; label: string; count: number }[] = MODULES.filter(
    (m) => m.id !== "today",
  ).map((m) => ({
    id: m.id,
    label: m.label,
    count: c
      ? m.id === "mail"
        ? c.unread_messages
        : m.id === "trash"
          ? c.trash
          : m.id === "tasks"
            ? c.tasks
            : m.id === "notes"
              ? c.notes
              : m.id === "contacts"
                ? c.contacts
                : m.id === "passwords"
                  ? c.passwords
                  : m.id === "calendar"
                    ? c.events
                    : 0
      : 0,
  }));

  return (
    <div class="today">
      <h2>Today</h2>
      {error && <p class="settings-error">{error}</p>}
      {!dash && !error && <p class="muted">Loading…</p>}
      {dash && (
        <div class="today-widgets">
          <Widget title="Agenda" count={dash.events.length} empty={dash.events.length === 0}>
            <ul class="today-list">
              {dash.events.map((e) => (
                <li key={`${e.event_id}-${e.start}`}>
                  <button
                    class="today-row"
                    onClick={() => jumpTo("calendar", "event", e.event_id)}
                  >
                    <span class="today-row-time">{fmtTime(e.start)}</span>
                    <span class="today-row-title">
                      {e.title}
                      {e.location && <span class="muted"> · {e.location}</span>}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </Widget>

          <Widget
            title="TO DO"
            count={dash.tasks.length}
            empty={dash.tasks.length === 0}
          >
            <ul class="today-list">
              {dash.tasks.map((t) => {
                const overdue =
                  t.due_utc !== null && new Date(t.due_utc).getTime() < Date.now();
                return (
                  <li key={t.id}>
                    <label class="today-row today-todo">
                      <input
                        type="checkbox"
                        onChange={(e) =>
                          void toggleTask(t.id, (e.target as HTMLInputElement).checked)
                        }
                      />
                      <span
                        class="today-row-title"
                        data-done={t.percent >= 100}
                        style={overdue ? "color: var(--danger, #c0392b)" : undefined}
                        onClick={() => jumpTo("tasks", "task", t.id)}
                      >
                        {t.title}
                        {t.due_utc && (
                          <span class="muted"> · {fmtWhen(t.due_utc)}</span>
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          </Widget>

          <Widget
            title="Messages"
            count={dash.unread_messages}
            empty={dash.accounts.every((a) => a.unread === 0)}
          >
            <ul class="today-list">
              {dash.accounts.map((a) => (
                <li key={a.id}>
                  <button
                    class="today-row"
                    disabled={a.unread === 0}
                    onClick={() => {
                      activeModule.value = "mail";
                    }}
                  >
                    <span class="today-row-title">{a.name}</span>
                    <span class="smartlist-count">{a.unread} unread</span>
                  </button>
                </li>
              ))}
            </ul>
          </Widget>

          <Widget
            title="Anniversaries"
            count={dash.anniversaries.length}
            empty={dash.anniversaries.length === 0}
          >
            <ul class="today-list">
              {dash.anniversaries.map((a) => (
                <li key={`${a.contact_id}-${a.kind}`}>
                  <button
                    class="today-row"
                    onClick={() => jumpTo("contacts", "contact", a.contact_id)}
                  >
                    <span class="today-row-time">{a.date.slice(5)}</span>
                    <span class="today-row-title">
                      {a.name}
                      <span class="muted">
                        {" "}
                        · {a.kind === "birthday" ? "turns" : "celebrates"} {a.years}
                      </span>
                    </span>
                    <span class="muted">
                      {a.days_ahead === 0 ? "today" : `in ${a.days_ahead}d`}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </Widget>
        </div>
      )}

      <div class="today-cards">
        {cards.map((card) => (
          <button
            key={card.id}
            class="today-card"
            onClick={() => {
              activeModule.value = card.id;
            }}
          >
            <span class="today-card-label">{card.label}</span>
            <span class="today-card-count">{card.count.toLocaleString()}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
