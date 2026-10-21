/**
 * EPIM-style list presentation helpers (Gate 4): mail date-group headers
 * (TODAY / YESTERDAY / LAST WEEK / <MONTH YEAR>), deterministic avatar
 * initials + hue, and the A–Z letter rail. Pure and framework-free so the
 * virtualizer can interleave header rows without touching the data model.
 */

/** One flattened list entry: a sticky-looking header row or a data row. */
export type GroupedRow<M> =
  | { type: "header"; label: string }
  | { type: "item"; item: M };

/**
 * Date-group label for the mail list, computed locally from the message's
 * stored timestamp. Older than a week collapses into "MONTH YEAR" (the year
 * is always shown so multi-year inboxes stay unambiguous).
 */
export function mailGroupLabel(dateIso: string, now: Date = new Date()): string {
  const d = new Date(dateIso);
  if (Number.isNaN(d.getTime())) return "";
  const startOfDay = (x: Date) =>
    new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (days <= 0) return "TODAY";
  if (days === 1) return "YESTERDAY";
  if (days < 7) return "LAST WEEK";
  return d
    .toLocaleDateString("en-US", { month: "long", year: "numeric" })
    .toUpperCase();
}

/** Interleave one header per group; headers count as virtualizer rows. */
export function withMailGroupHeaders<M extends { date: string }>(
  items: readonly M[],
  now: Date = new Date(),
): GroupedRow<M>[] {
  const out: GroupedRow<M>[] = [];
  let last = "";
  for (const item of items) {
    const label = mailGroupLabel(item.date, now);
    if (label !== last && label !== "") {
      out.push({ type: "header", label });
      last = label;
    }
    out.push({ type: "item", item });
  }
  return out;
}

/** Avatar initials: first letter of the first and last word, else "?". */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  const first = parts[0]![0]!;
  return (
    parts.length > 1 ? first + parts[parts.length - 1]![0]! : first
  ).toUpperCase();
}

/** Deterministic hue bucket (0..buckets-1) from any display string. */
export function hueIndex(name: string, buckets = 8): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return buckets > 0 ? h % buckets : 0;
}

/** Sorted initials actually present in a set of names ("#" for non A–Z). */
export function availableLetters(names: readonly string[]): string[] {
  const set = new Set<string>();
  for (const n of names) {
    const c = (n.trim()[0] ?? "#").toUpperCase();
    set.add(/[A-Z]/.test(c) ? c : "#");
  }
  return [...set].sort((a, b) => (a === "#" ? 1 : b === "#" ? -1 : a < b ? -1 : 1));
}

/** First letter used for grouping a name ("#" when it is not A–Z). */
export function letterOf(name: string): string {
  const c = (name.trim()[0] ?? "#").toUpperCase();
  return /[A-Z]/.test(c) ? c : "#";
}

/** Word count of an HTML fragment: tags stripped, runs of space split. */
export function wordCount(html: string): number {
  return html
    .replace(/<[^>]*>/g, " ")
    .replace(/&nbsp;/gi, " ")
    .split(/\s+/)
    .filter(Boolean).length;
}
