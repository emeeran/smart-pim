/**
 * Pure windowing + keyset-page accumulation math for virtualized lists
 * (T2.8 / AT-19). Framework-free so it is unit-testable and hot-path cheap.
 */

/**
 * Which rows should be mounted for a given scroll state. `overscan` rows are
 * rendered on each side so scrolling never shows blank gaps at 60 fps.
 */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  rowCount: number,
  overscan = 6,
): { start: number; end: number } {
  if (rowCount === 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const first = Math.floor(scrollTop / rowHeight);
  const visible = Math.ceil(viewportHeight / rowHeight) + 1;
  const start = Math.max(0, first - overscan);
  const end = Math.min(rowCount, first + visible + overscan);
  return { start, end };
}

/** A keyset page appended onto accumulated rows: dedupe by id, keep order. */
export function appendPage<T extends { id: number }>(
  rows: T[],
  page: readonly T[],
): { rows: T[]; appended: number } {
  const seen = new Set(rows.map((r) => r.id));
  const fresh = page.filter((r) => !seen.has(r.id));
  if (fresh.length === 0) return { rows, appended: 0 };
  return { rows: [...rows, ...fresh], appended: fresh.length };
}

/** Cursor for fetching the next page: the last accumulated row. */
export function nextCursor<T extends { id: number; date: string }>(
  rows: readonly T[],
): { date: string; id: number } | undefined {
  const last = rows[rows.length - 1];
  return last ? { date: last.date, id: last.id } : undefined;
}

/**
 * Date-group headers for the mail list (S17): "Today", "Yesterday",
 * weekday, or a date — computed from the row's stored UTC timestamp.
 */
export type GroupLabel = string;

export function groupLabel(dateIso: string, now: Date = new Date()): GroupLabel {
  const d = new Date(dateIso);
  if (Number.isNaN(d.getTime())) return "";
  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7)
    return d.toLocaleDateString(undefined, { weekday: "long" });
  if (d.getFullYear() === now.getFullYear())
    return d.toLocaleDateString(undefined, { month: "long", day: "numeric" });
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/**
 * Flatten grouped rows into list entries with header rows interleaved —
 * headers count as rows in the virtualizer.
 */
export type Row<M> = { type: "header"; label: GroupLabel } | { type: "item"; item: M };

export function withGroupHeaders<M extends { date: string }>(
  items: readonly M[],
  now: Date = new Date(),
): Row<M>[] {
  const out: Row<M>[] = [];
  let last = "";
  for (const item of items) {
    const label = groupLabel(item.date, now);
    if (label !== last) {
      out.push({ type: "header", label });
      last = label;
    }
    out.push({ type: "item", item });
  }
  return out;
}
