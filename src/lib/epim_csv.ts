/**
 * EssentialPIM CSV import presets (F-003 interchange): EPIM exports CSVs with
 * recognizable header sets, so the import wizard can detect the layout and
 * auto-apply a column mapping instead of making the user map by hand.
 *
 * Pure module — no DOM, no API calls — so it is fully unit-tested.
 *
 * Dates: EPIM CSVs use MM/DD/YYYY, DD.MM.YYYY or YYYY-MM-DD. No client-side
 * transformation is needed: the mapping only points at column indices and the
 * backend parses all three forms, so a detected date column is passed through
 * verbatim.
 */

export interface EpimPresetMatch {
  /** Preset id: "contacts" | "tasks" | "notes" | "events". */
  preset: string;
  /** Human-readable banner text, e.g. "EssentialPIM Contacts export". */
  label: string;
  /** Target field → CSV column index (first matching, never reused). */
  mapping: Record<string, number>;
}

/** One mappable target field and the header spellings it answers to. */
interface PresetField {
  field: string;
  aliases: string[];
}

interface Preset {
  id: string;
  label: string;
  /** Minimum number of fields that must match before we claim the layout. */
  threshold: number;
  fields: PresetField[];
}

/** Lowercase, trim and strip separators so "E-mail 1" ≡ "email1". */
function normalize(header: string): string {
  return header.trim().toLowerCase().replace(/[^a-z0-9%]/g, "");
}

const CONTACTS: Preset = {
  id: "contacts",
  label: "EssentialPIM Contacts export",
  threshold: 3,
  fields: [
    { field: "first_name", aliases: ["firstname", "givenname", "first"] },
    { field: "last_name", aliases: ["lastname", "surname", "familyname", "last"] },
    { field: "display_name", aliases: ["displayname", "fullname", "name"] },
    {
      field: "email",
      aliases: [
        "email",
        "emailaddress",
        "email1",
        "homeemail",
        "businessemail",
        "primaryemail",
        "mail",
      ],
    },
    { field: "email2", aliases: ["email2", "secondaryemail", "email2address"] },
    {
      field: "phone",
      aliases: [
        "phone",
        "phonenumber",
        "tel",
        "mobile",
        "cell",
        "homephone",
        "businessphone",
        "workphone",
        "primaryphone",
      ],
    },
    { field: "organization", aliases: ["organization", "company", "employer"] },
    { field: "address", aliases: ["address", "street", "streetaddress", "homeaddress", "businessaddress"] },
    { field: "url", aliases: ["url", "website", "webpage", "homepage", "web"] },
    { field: "birthday", aliases: ["birthday", "birthdate", "dob"] },
    { field: "notes", aliases: ["notes", "note", "comment", "comments"] },
    // Not a contact field on the backend today; kept so the mapping round-trips.
    { field: "categories", aliases: ["categories", "category", "tags", "groups"] },
  ],
};

const TASKS: Preset = {
  id: "tasks",
  label: "EssentialPIM Tasks export",
  threshold: 2,
  fields: [
    { field: "subject", aliases: ["tasksubject", "subject", "tasktitle", "title", "summary"] },
    { field: "start_date", aliases: ["startdate", "begindate", "start"] },
    { field: "due_date", aliases: ["duedate", "enddate", "deadline", "due"] },
    { field: "categories", aliases: ["categories", "category", "tags", "groups"] },
    { field: "percent_complete", aliases: ["percentcomplete", "%complete", "complete", "progress"] },
    { field: "priority", aliases: ["priority", "importance"] },
    { field: "status", aliases: ["status", "state"] },
    { field: "notes", aliases: ["notes", "note", "comment", "comments", "description"] },
  ],
};

const NOTES: Preset = {
  id: "notes",
  label: "EssentialPIM Notes export",
  threshold: 2,
  fields: [
    { field: "title", aliases: ["notetitle", "title", "subject", "name"] },
    { field: "content", aliases: ["notetext", "content", "text", "body", "note"] },
    { field: "categories", aliases: ["categories", "category", "tags", "groups"] },
  ],
};

const EVENTS: Preset = {
  id: "events",
  label: "EssentialPIM Appointments export",
  threshold: 3,
  fields: [
    { field: "subject", aliases: ["subject", "event", "eventtitle", "appointment", "title", "summary"] },
    { field: "start_date", aliases: ["startdate", "begindate", "start", "from"] },
    { field: "start_time", aliases: ["starttime", "begintime"] },
    { field: "end_date", aliases: ["enddate", "finishdate", "to"] },
    { field: "end_time", aliases: ["endtime", "finishtime"] },
    { field: "location", aliases: ["location", "place", "where"] },
    { field: "description", aliases: ["description", "notes", "note", "comments"] },
    { field: "categories", aliases: ["categories", "category", "tags", "groups"] },
    { field: "repeat", aliases: ["repeat", "recurrence", "recurring", "recurrence rule"] },
  ],
};

/** Order matters only for tie-breaking; more distinctive layouts come first. */
const PRESETS: Preset[] = [CONTACTS, TASKS, EVENTS, NOTES];

/**
 * Detect an EPIM CSV layout from its header row. Returns the best-scoring
 * preset (must meet its threshold) with the implied column mapping, or null
 * when nothing matches well enough and the user should map by hand.
 */
export function detectEpimCsv(headers: string[]): EpimPresetMatch | null {
  const normalized = headers.map(normalize);
  let best: EpimPresetMatch | null = null;
  for (const preset of PRESETS) {
    const mapping: Record<string, number> = {};
    const used = new Set<number>();
    for (const { field, aliases } of preset.fields) {
      const idx = normalized.findIndex((h, i) => !used.has(i) && aliases.includes(h));
      if (idx !== -1) {
        mapping[field] = idx;
        used.add(idx);
      }
    }
    if (Object.keys(mapping).length < preset.threshold) continue;
    // Strictly better only, so earlier (more distinctive) presets win ties.
    if (!best || Object.keys(mapping).length > Object.keys(best.mapping).length) {
      best = { preset: preset.id, label: preset.label, mapping };
    }
  }
  return best;
}
