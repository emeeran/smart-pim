/**
 * EventDialog: the F-004 "New Appointment" editor — title + type, Home /
 * Related Items tabs, start/end with All day, calendar color picker,
 * location, completion slider, recurrence builder mapped onto an RRULE,
 * reminders, participant chips, and the Delete / Save & New / OK / Cancel
 * footer (from FormDialog). Persists through api.eventSave / eventDelete.
 */
import { useEffect, useMemo, useState } from "preact/hooks";
import {
  api,
  type CalendarInfo,
  type ContactRecord,
  type EventInputMsg,
} from "../api";
import { FormDialog } from "./FormDialog";

/** What the caller hands in — mirrors EventInputMsg in UI-friendly shapes. */
export interface EventDraft {
  id: number | null;
  title: string;
  /** ISO instant, or yyyy-mm-dd when all_day. */
  start: string;
  end: string | null;
  allDay: boolean;
  location: string | null;
  description: string | null;
  rrule: string | null;
  reminderMinutes: number | null;
}

/** One attendee while editing: email is the key, contact link optional. */
export interface ParticipantDraft {
  contact_id: number | null;
  email: string;
}

type RepeatUnit = "day" | "week" | "month" | "year";
type RepeatEnd = "never" | "on" | "after";

const FREQ: Record<RepeatUnit, string> = {
  day: "DAILY",
  week: "WEEKLY",
  month: "MONTHLY",
  year: "YEARLY",
};

const REMINDER_PRESETS: { label: string; minutes: number }[] = [
  { label: "At time of event", minutes: 0 },
  { label: "5 minutes before", minutes: 5 },
  { label: "10 minutes before", minutes: 10 },
  { label: "30 minutes before", minutes: 30 },
  { label: "1 hour before", minutes: 60 },
];

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function localParts(iso: string | null, fallback: Date): { date: string; time: string } {
  const d = iso ? new Date(iso) : fallback;
  if (Number.isNaN(d.getTime())) return { date: "", time: "09:00" };
  return {
    date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    time: `${pad(d.getHours())}:${pad(d.getMinutes())}`,
  };
}

/** "Repeat every N <unit>" + Ends → RRULE (RFC 5545 subset). */
export function buildRrule(
  every: number,
  unit: RepeatUnit,
  end: RepeatEnd,
  until: string,
  count: number,
): string | null {
  if (every < 1) return null;
  let r = `FREQ=${FREQ[unit]};INTERVAL=${every}`;
  if (end === "on" && until) r += `;UNTIL=${until.replace(/-/g, "")}`;
  if (end === "after" && count > 0) r += `;COUNT=${count}`;
  return r;
}

/** Inverse of buildRrule — fills the recurrence fields from a stored RRULE. */
export function parseRrule(
  rrule: string | null,
): { every: number; unit: RepeatUnit; end: RepeatEnd; until: string; count: number } {
  const out = { every: 1, unit: "day" as RepeatUnit, end: "never" as RepeatEnd, until: "", count: 1 };
  if (!rrule) return out;
  const kv = new Map(rrule.split(";").map((p) => p.split("=", 2) as [string, string]));
  const freq = kv.get("FREQ") ?? "";
  out.unit = (Object.keys(FREQ) as RepeatUnit[]).find((u) => FREQ[u] === freq) ?? "day";
  out.every = Math.max(1, Number(kv.get("INTERVAL") ?? 1) || 1);
  const until = kv.get("UNTIL");
  const count = kv.get("COUNT");
  if (until) {
    out.end = "on";
    const s = until.slice(0, 8);
    out.until = `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}`;
  } else if (count) {
    out.end = "after";
    out.count = Math.max(1, Number(count) || 1);
  }
  return out;
}

interface FormState {
  title: string;
  type: string;
  date: string;
  start: string;
  end: string;
  allDay: boolean;
  calendarId: number | null;
  location: string;
  description: string;
  percent: number;
  every: number;
  unit: RepeatUnit;
  endMode: RepeatEnd;
  until: string;
  count: number;
  reminders: number[];
  participants: ParticipantDraft[];
}

export function EventDialog({
  draft,
  onClose,
  onSaved,
}: {
  draft: EventDraft;
  onClose: () => void;
  onSaved: () => void;
}) {
  const isNew = draft.id === null;
  const [calendars, setCalendars] = useState<CalendarInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"home" | "related">("home");
  const [links, setLinks] = useState<{ id: number; kind: string; title: string }[]>([]);
  const [linkId, setLinkId] = useState("");
  const [form, setForm] = useState<FormState>(() => {
    const startDate = localParts(draft.allDay ? null : draft.start, new Date());
    const endDate = localParts(draft.allDay || !draft.end ? null : draft.end, new Date());
    const r = parseRrule(draft.rrule);
    const fallbackDate = draft.allDay ? draft.start.slice(0, 10) : startDate.date;
    return {
      title: draft.title,
      // EventInputMsg carries no type column — kept for UI parity only.
      type: draft.allDay ? "All day event" : "Appointment",
      date: fallbackDate,
      start: draft.allDay ? "09:00" : startDate.time,
      end: draft.allDay ? "10:00" : endDate.time,
      allDay: draft.allDay,
      calendarId: null,
      location: draft.location ?? "",
      description: draft.description ?? "",
      // EventInputMsg has no completion column — slider is UI parity only.
      percent: 0,
      every: r.every,
      unit: r.unit,
      endMode: r.end,
      until: r.until,
      count: r.count,
      reminders: draft.reminderMinutes === null || draft.reminderMinutes === undefined ? [] : [draft.reminderMinutes],
      participants: [],
    };
  });
  /** Attendee text being typed (Enter/comma commits a chip). */
  const [participantInput, setParticipantInput] = useState("");
  /** Contact emails for the picker's autocomplete. */
  const [contactEmails, setContactEmails] = useState<ContactRecord[]>([]);
  const set = (patch: Partial<FormState>) => setForm((f) => ({ ...f, ...patch }));

  useEffect(() => {
    void (async () => {
      try {
        const cals = await api.calendarsList();
        setCalendars(cals);
        setForm((f) => ({
          ...f,
          calendarId: cals.some((c) => c.id === f.calendarId)
            ? f.calendarId
            : cals[0]?.id ?? null,
        }));
        // Autocomplete source: contact emails (never any network lookup).
        api
          .contactsList()
          .then(setContactEmails)
          .catch(() => setContactEmails([]));
        // Existing attendees load into the editor for editing.
        if (draft.id !== null) {
          const parts = await api.eventParticipants(draft.id).catch(() => []);
          setForm((f) => ({
            ...f,
            participants: parts.map((p) => ({ contact_id: p.contact_id, email: p.email })),
          }));
        }
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  useEffect(() => {
    if (tab !== "related" || draft.id === null) return;
    void api
      .itemLinks(draft.id)
      .then(setLinks)
      .catch((e) => setError(String(e)));
  }, [tab, draft.id]);

  const rrule = useMemo(
    () => buildRrule(form.every, form.unit, form.endMode, form.until, form.count),
    [form.every, form.unit, form.endMode, form.until, form.count],
  );

  /** Commit the typed attendee; match a contact by email when possible. */
  const addParticipant = () => {
    const email = participantInput.trim().replace(/[,;]$/, "");
    setParticipantInput("");
    if (!email || !email.includes("@")) return;
    setForm((f) =>
      f.participants.some((p) => p.email.toLowerCase() === email.toLowerCase())
        ? f
        : {
            ...f,
            participants: [
              ...f.participants,
              {
                contact_id:
                  contactEmails.find((c) => c.emails.some((e) => e.toLowerCase() === email.toLowerCase()))?.id ?? null,
                email,
              },
            ],
          },
    );
  };

  // "Save & New" flips this so later saves create events instead of
  // overwriting the one we started from.
  const [asNew, setAsNew] = useState(false);
  const editId = asNew ? null : draft.id;

  const payload = (): EventInputMsg => {
    const startIso = form.allDay
      ? form.date
      : new Date(`${form.date}T${form.start}:00`).toISOString();
    const endIso = form.allDay
      ? null
      : new Date(`${form.date}T${form.end}:00`).toISOString();
    return {
      id: editId,
      calendar_id: form.calendarId ?? -1,
      title: form.title.trim(),
      start: startIso,
      end: endIso,
      all_day: form.allDay,
      location: form.location.trim() || null,
      description: form.description.trim() || null,
      rrule,
      // Only one reminder is persisted; extra rows are a UI affordance.
      reminder_minutes: form.reminders.length > 0 ? form.reminders[0] : null,
      participants: form.participants,
    };
  };

  const validate = (): string | null => {
    if (!form.title.trim()) return "Title is required.";
    if (!form.date) return "A start date is required.";
    if (form.calendarId === null) return "Create a calendar first.";
    if (!form.allDay && form.end < form.start) return "End time is before start time.";
    return null;
  };

  const save = async (): Promise<boolean> => {
    const problem = validate();
    if (problem) {
      setError(problem);
      return false;
    }
    try {
      await api.eventSave(payload());
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  };

  const remove = async () => {
    if (draft.id === null) return;
    if (!confirm("Move this event to trash?")) return;
    try {
      await api.eventDelete(draft.id);
      onSaved();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  const addLink = async () => {
    if (draft.id === null || !linkId.trim()) return;
    try {
      await api.itemLink(draft.id, Number(linkId));
      setLinks(await api.itemLinks(draft.id));
      setLinkId("");
    } catch (e) {
      setError(String(e));
    }
  };

  const unlink = async (otherId: number) => {
    if (draft.id === null) return;
    try {
      await api.itemUnlink(draft.id, otherId);
      setLinks(await api.itemLinks(draft.id));
    } catch (e) {
      setError(String(e));
    }
  };

  const calendarPicker = (
    <div class="form-row">
      <label>Calendar</label>
      <div class="cal-swatch-row">
        {calendars.map((c) => (
          <button
            key={c.id}
            class="cal-swatch"
            title={c.name}
            style={{ background: c.color ?? "var(--accent)" }}
            data-selected={form.calendarId === c.id}
            onClick={() => set({ calendarId: c.id })}
          />
        ))}
        {calendars.length === 0 && <span class="form-subtle">No calendars yet.</span>}
      </div>
    </div>
  );

  return (
    <FormDialog
      open
      width={520}
      title={isNew ? "New Appointment" : "Edit Appointment"}
      onClose={onClose}
      error={error}
      onDelete={isNew ? undefined : () => void remove()}
      onSaveNew={async () => {
        if (await save()) {
          onSaved();
          setAsNew(true);
          setForm((f) => ({
            ...f,
            title: "",
            location: "",
            description: "",
            percent: 0,
            reminders: [],
            participants: [],
            every: 1,
            endMode: "never",
            until: "",
            count: 1,
          }));
          setError(null);
        }
      }}
      onOk={async () => {
        if (await save()) {
          onSaved();
          onClose();
        }
      }}
    >
      <div class="form-grid">
        <div class="form-row">
          <label>Title</label>
          <input
            value={form.title}
            placeholder="New Appointment"
            onInput={(e) => set({ title: (e.target as HTMLInputElement).value })}
          />
          <select
            value={form.type}
            title="Item type (not persisted)"
            onChange={(e) => set({ type: (e.target as HTMLSelectElement).value })}
          >
            <option>Appointment</option>
            <option>All day event</option>
            <option>Anniversary</option>
            <option>Meeting</option>
          </select>
        </div>

        <div class="form-tabs" role="tablist">
          <button role="tab" aria-selected={tab === "home"} onClick={() => setTab("home")}>
            Home
          </button>
          <button role="tab" aria-selected={tab === "related"} onClick={() => setTab("related")}>
            Related Items{!isNew && links.length > 0 ? ` (${links.length})` : ""}
          </button>
        </div>

        {tab === "home" ? (
          <div class="form-tabpane form-grid">
            <div class="form-row">
              <label>Start</label>
              <div class="form-inline">
                <input
                  type="date"
                  value={form.date}
                  onChange={(e) => set({ date: (e.target as HTMLInputElement).value })}
                />
                {!form.allDay && (
                  <input
                    type="time"
                    value={form.start}
                    onChange={(e) => set({ start: (e.target as HTMLInputElement).value })}
                  />
                )}
                <label class="form-check">
                  <input
                    type="checkbox"
                    checked={form.allDay}
                    onChange={(e) => set({ allDay: (e.target as HTMLInputElement).checked })}
                  />
                  All day
                </label>
              </div>
            </div>
            <div class="form-row">
              <label>End</label>
              <div class="form-inline">
                <input type="date" value={form.date} disabled title="Same day as start" />
                {!form.allDay && (
                  <input
                    type="time"
                    value={form.end}
                    onChange={(e) => set({ end: (e.target as HTMLInputElement).value })}
                  />
                )}
              </div>
            </div>

            {calendarPicker}

            <div class="form-row">
              <label>Location</label>
              <input
                value={form.location}
                onInput={(e) => set({ location: (e.target as HTMLInputElement).value })}
              />
            </div>

            <div class="form-row">
              <label>Completion</label>
              <div class="form-inline">
                <input
                  type="range"
                  min={0}
                  max={100}
                  step={5}
                  value={form.percent}
                  title="Completion (not persisted yet)"
                  onInput={(e) => set({ percent: Number((e.target as HTMLInputElement).value) })}
                />
                <span class="form-subtle">{form.percent === 0 ? "None" : `${form.percent}%`}</span>
              </div>
            </div>

            <div class="form-row">
              <label>Repeat</label>
              <div class="form-inline">
                <span class="form-subtle">Every</span>
                <input
                  type="number"
                  min={1}
                  max={999}
                  style={{ width: "64px", flex: "none" }}
                  value={form.every}
                  onInput={(e) => set({ every: Number((e.target as HTMLInputElement).value) })}
                />
                <select
                  value={form.unit}
                  onChange={(e) => set({ unit: (e.target as HTMLSelectElement).value as RepeatUnit })}
                >
                  <option value="day">day(s)</option>
                  <option value="week">week(s)</option>
                  <option value="month">month(s)</option>
                  <option value="year">year(s)</option>
                </select>
              </div>
            </div>
            <div class="form-row">
              <label>Ends</label>
              <div class="form-inline">
                <select
                  value={form.endMode}
                  onChange={(e) =>
                    set({ endMode: (e.target as HTMLSelectElement).value as RepeatEnd })
                  }
                >
                  <option value="never">Never</option>
                  <option value="on">On date</option>
                  <option value="after">After</option>
                </select>
                {form.endMode === "on" && (
                  <input
                    type="date"
                    value={form.until}
                    onChange={(e) => set({ until: (e.target as HTMLInputElement).value })}
                  />
                )}
                {form.endMode === "after" && (
                  <input
                    type="number"
                    min={1}
                    style={{ width: "72px", flex: "none" }}
                    title="Number of occurrences"
                    value={form.count}
                    onInput={(e) => set({ count: Number((e.target as HTMLInputElement).value) })}
                  />
                )}
              </div>
            </div>
            <div class="form-subtle">{rrule ? `RRULE: ${rrule}` : "Does not repeat."}</div>

            <div class="form-row">
              <label>Reminders</label>
              <div class="form-grid" style={{ flex: 1 }}>
                {form.reminders.map((m, i) => (
                  <div class="reminder-row" key={`${m}-${i}`}>
                    <select
                      value={String(m)}
                      onChange={(e) => {
                        const v = Number((e.target as HTMLSelectElement).value);
                        set({ reminders: form.reminders.map((x, j) => (j === i ? v : x)) });
                      }}
                    >
                      {REMINDER_PRESETS.map((p) => (
                        <option key={p.minutes} value={p.minutes}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                    <button
                      class="form-remove"
                      title="Remove reminder"
                      onClick={() => set({ reminders: form.reminders.filter((_, j) => j !== i) })}
                    >
                      ×
                    </button>
                  </div>
                ))}
                <button
                  class="nav-addlink"
                  style={{ padding: "2px 0" }}
                  onClick={() => set({ reminders: [...form.reminders, 15] })}
                >
                  + Add Reminder
                </button>
                <span class="form-subtle">Only the first reminder is stored per event.</span>
              </div>
            </div>

            <div class="form-row">
              <label>Participants</label>
              <div class="form-grid" style={{ flex: 1 }}>
                <input
                  value={participantInput}
                  list="participant-emails"
                  placeholder="Attendee e-mail — Enter to add"
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      addParticipant();
                    }
                  }}
                  onInput={(e) => setParticipantInput((e.target as HTMLInputElement).value)}
                />
                <datalist id="participant-emails">
                  {contactEmails.flatMap((c) =>
                    c.emails.map((e) => <option key={e} value={e}>{c.display_name}</option>),
                  )}
                </datalist>
                <div class="chip-row">
                  {form.participants.map((p) => {
                    const who = contactEmails.find((c) => c.id === p.contact_id)?.display_name;
                    return (
                      <span class="tag-pill" key={p.email} title={who}>
                        {p.email}
                        <button
                          class="related-unlink"
                          title="Remove participant"
                          onClick={() =>
                            set({
                              participants: form.participants.filter(
                                (x) => x.email !== p.email,
                              ),
                            })
                          }
                        >
                          ×
                        </button>
                      </span>
                    );
                  })}
                </div>
                <span class="form-subtle">
                  Attendees are stored with the event and connect it to their
                  contacts (see Related Items).
                </span>
              </div>
            </div>

            <div class="form-row">
              <label>Notes</label>
              <textarea
                rows={3}
                value={form.description}
                onInput={(e) => set({ description: (e.target as HTMLTextAreaElement).value })}
              />
            </div>
          </div>
        ) : (
          <div class="form-tabpane form-grid">
            {isNew ? (
              <p class="form-subtle">Save the event to link related items.</p>
            ) : (
              <>
                <ul class="related-list">
                  {links.map((l) => (
                    <li key={l.id}>
                      <span class="related-kind">{l.kind}</span>
                      <span class="related-title">{l.title}</span>
                      <button class="related-unlink" title="Unlink" onClick={() => void unlink(l.id)}>
                        ×
                      </button>
                    </li>
                  ))}
                  {links.length === 0 && <li class="form-subtle">No related items.</li>}
                </ul>
                <div class="related-add">
                  <input
                    value={linkId}
                    placeholder="Item id"
                    onInput={(e) => setLinkId((e.target as HTMLInputElement).value)}
                  />
                  <button class="primary" onClick={() => void addLink()}>
                    Link
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </FormDialog>
  );
}
