import { describe, expect, it } from "vitest";
import {
  appendPage,
  groupLabel,
  nextCursor,
  visibleRange,
  withGroupHeaders,
} from "./list-model";

describe("visibleRange", () => {
  it("mounts only the visible window plus overscan", () => {
    const { start, end } = visibleRange(1000, 500, 50, 100_000, 6);
    expect(start).toBe(14); // 1000/50=20 − 6
    expect(end).toBeLessThanOrEqual(37);
    expect(end).toBeGreaterThan(21); // at least the visible rows
  });

  it("clamps at both ends", () => {
    expect(visibleRange(0, 400, 50, 10)).toEqual({ start: 0, end: 10 });
    expect(visibleRange(0, 400, 50, 0)).toEqual({ start: 0, end: 0 });
  });

  it("is monotonic in scroll position and independent of total count", () => {
    const a = visibleRange(5_000, 600, 50, 100_000);
    const b = visibleRange(6_000, 600, 50, 100_000);
    expect(b.start).toBeGreaterThanOrEqual(a.start);
    expect(visibleRange(5_000, 600, 50, 10_000)).toEqual(
      visibleRange(5_000, 600, 50, 100_000),
    );
  });
});

describe("appendPage / nextCursor (keyset pagination)", () => {
  const row = (id: number) => ({ id, date: `2026-01-01T00:00:${String(id % 60).padStart(2, "0")}Z` });

  it("dedupes overlapping pages without reordering", () => {
    let rows = [1, 2, 3].map(row);
    const overlap = [3, 4, 5].map(row);
    const res = appendPage(rows, overlap);
    expect(res.appended).toBe(2);
    rows = res.rows;
    expect(rows.map((r) => r.id)).toEqual([1, 2, 3, 4, 5]);
    expect(appendPage(rows, overlap).appended).toBe(0);
  });

  it("cursor is the last row and absent when empty", () => {
    expect(nextCursor([])).toBeUndefined();
    const rows = [7, 8].map(row);
    expect(nextCursor(rows)).toEqual({ date: rows[1].date, id: 8 });
  });
});

describe("groupLabel / withGroupHeaders (S17 date grouping)", () => {
  const now = new Date(2026, 9, 19, 12, 0, 0); // Mon Oct 19 2026, local

  it("labels today and yesterday", () => {
    expect(groupLabel(new Date(2026, 9, 19, 8, 0).toISOString(), now)).toBe("Today");
    expect(groupLabel(new Date(2026, 9, 18, 8, 0).toISOString(), now)).toBe("Yesterday");
  });

  it("labels the rest of the week by weekday, older by date", () => {
    expect(groupLabel(new Date(2026, 9, 15, 8, 0).toISOString(), now)).toBe("Thursday");
    expect(groupLabel(new Date(2026, 8, 30, 8, 0).toISOString(), now)).toMatch(/Sep/);
  });

  it("interleaves one header per group", () => {
    const items = [
      { id: 1, date: new Date(2026, 9, 19, 9, 0).toISOString() },
      { id: 2, date: new Date(2026, 9, 19, 7, 0).toISOString() },
      { id: 3, date: new Date(2026, 9, 18, 9, 0).toISOString() },
    ];
    const rows = withGroupHeaders(items, now);
    expect(rows.map((r) => r.type)).toEqual(["header", "item", "item", "header", "item"]);
  });
});
