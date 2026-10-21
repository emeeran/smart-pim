import { describe, expect, it } from "vitest";
import {
  availableLetters,
  hueIndex,
  initials,
  letterOf,
  mailGroupLabel,
  withMailGroupHeaders,
  wordCount,
} from "./grouping";

describe("mailGroupLabel (EPIM-style date headers)", () => {
  const now = new Date(2026, 9, 19, 12, 0, 0); // Mon Oct 19 2026, local

  it("labels today, yesterday and the rest of the week", () => {
    expect(mailGroupLabel(new Date(2026, 9, 19, 8, 0).toISOString(), now)).toBe("TODAY");
    expect(mailGroupLabel(new Date(2026, 9, 18, 23, 59).toISOString(), now)).toBe("YESTERDAY");
    expect(mailGroupLabel(new Date(2026, 9, 17, 8, 0).toISOString(), now)).toBe("LAST WEEK");
    expect(mailGroupLabel(new Date(2026, 9, 13, 8, 0).toISOString(), now)).toBe("LAST WEEK");
  });

  it("labels older mail by month and year", () => {
    expect(mailGroupLabel(new Date(2026, 8, 30, 8, 0).toISOString(), now)).toBe(
      "SEPTEMBER 2026",
    );
    expect(mailGroupLabel(new Date(2024, 0, 2, 8, 0).toISOString(), now)).toBe(
      "JANUARY 2024",
    );
  });

  it("tolerates garbage timestamps", () => {
    expect(mailGroupLabel("not-a-date", now)).toBe("");
  });

  it("keeps a future-dated message in TODAY", () => {
    expect(mailGroupLabel(new Date(2026, 9, 20, 8, 0).toISOString(), now)).toBe("TODAY");
  });
});

describe("withMailGroupHeaders", () => {
  const now = new Date(2026, 9, 19, 12, 0, 0);

  it("interleaves exactly one header per group", () => {
    const items = [
      { id: 1, date: new Date(2026, 9, 19, 9, 0).toISOString() },
      { id: 2, date: new Date(2026, 9, 19, 7, 0).toISOString() },
      { id: 3, date: new Date(2026, 9, 18, 9, 0).toISOString() },
      { id: 4, date: new Date(2026, 8, 2, 9, 0).toISOString() },
    ];
    const rows = withMailGroupHeaders(items, now);
    expect(rows.map((r) => r.type)).toEqual([
      "header",
      "item",
      "item",
      "header",
      "item",
      "header",
      "item",
    ]);
    expect(rows[0]).toEqual({ type: "header", label: "TODAY" });
    expect(rows[3]).toEqual({ type: "header", label: "YESTERDAY" });
    expect(rows[5]).toEqual({ type: "header", label: "SEPTEMBER 2026" });
  });

  it("drops items with unusable dates rather than labelling them", () => {
    const rows = withMailGroupHeaders([{ id: 9, date: "bogus" }], now);
    expect(rows).toEqual([{ type: "item", item: { id: 9, date: "bogus" } }]);
  });
});

describe("initials / hueIndex (avatar tiles)", () => {
  it("takes the first letters of the first and last word", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("madonna")).toBe("M");
    expect(initials("  van  der Berg ")).toBe("VB");
    expect(initials("")).toBe("?");
  });

  it("hashes names deterministically into the palette", () => {
    expect(hueIndex("Ada Lovelace")).toBe(hueIndex("Ada Lovelace"));
    expect(hueIndex("Ada Lovelace")).toBeGreaterThanOrEqual(0);
    expect(hueIndex("Ada Lovelace")).toBeLessThan(8);
    expect(hueIndex("", 8)).toBe(0);
    expect(hueIndex("x", 0)).toBe(0);
  });
});

describe("availableLetters / letterOf (A–Z rail)", () => {
  it("lists only initials that exist, digit/symbol names last under #", () => {
    expect(availableLetters(["Mara", "adam", "42 Corp", "Zoe"])).toEqual([
      "A",
      "M",
      "Z",
      "#",
    ]);
  });

  it("maps each name to its rail letter", () => {
    expect(letterOf("Zoe")).toBe("Z");
    expect(letterOf("1password")).toBe("#");
    expect(letterOf("")).toBe("#");
  });
});

describe("wordCount", () => {
  it("counts words in an HTML fragment", () => {
    expect(wordCount("<p>Hello <b>cruel</b> world</p>")).toBe(3);
    expect(wordCount("<p>a&nbsp;b</p>")).toBe(2);
    expect(wordCount("<p></p>")).toBe(0);
  });
});
