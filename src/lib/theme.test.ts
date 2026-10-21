import { beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolveTheme, THEME_KEY, type ThemePref } from "./theme";

describe("resolveTheme", () => {
  const cases: [ThemePref, boolean, string][] = [
    ["light", false, "light"],
    ["light", true, "light"],
    ["grey", false, "grey"],
    ["grey", true, "grey"],
    ["dark", false, "dark"],
    ["dark", true, "dark"],
    ["system", false, "light"],
    ["system", true, "dark"],
  ];
  it.each(cases)("resolves %s with prefersDark=%s → %s", (pref, dark, want) => {
    expect(resolveTheme(pref, dark)).toBe(want);
  });
});

describe("applyTheme", () => {
  beforeEach(() => {
    vi.stubGlobal("matchMedia", (q: string) => ({
      matches: q.includes("dark"),
      addEventListener: () => {},
      removeEventListener: () => {},
    }));
    const store = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
    });
    const dataset: Record<string, string> = {};
    vi.stubGlobal("document", { documentElement: { dataset } });
    delete dataset.theme;
    delete dataset.themePref;
  });

  it("writes the resolved theme and persists the preference", async () => {
    const { applyTheme } = await import("./theme");
    const el = (document as unknown as { documentElement: { dataset: Record<string, string> } })
      .documentElement;
    applyTheme("system");
    expect(el.dataset.theme).toBe("dark"); // stub matches dark
    expect(el.dataset.themePref).toBe("system");
    expect(localStorage.getItem(THEME_KEY)).toBe("system");

    applyTheme("grey");
    expect(el.dataset.theme).toBe("grey");
    expect(localStorage.getItem(THEME_KEY)).toBe("grey");
  });
});

describe("tokens.css coverage", () => {
  // Read from disk: vitest returns an empty string for `.css?raw` imports.
  const css = readFileSync(new URL("../design/tokens.css", import.meta.url), "utf8");

  function declaredProps(blockSelector: string): Set<string> {
    const start = css.indexOf(blockSelector);
    expect(start).toBeGreaterThan(-1);
    const open = css.indexOf("{", start);
    let depth = 0;
    let end = open;
    for (let i = open; i < css.length; i++) {
      if (css[i] === "{") depth++;
      if (css[i] === "}") {
        depth--;
        if (depth === 0) {
          end = i;
          break;
        }
      }
    }
    const body = css.slice(open + 1, end);
    return new Set([...body.matchAll(/--[\w-]+(?=\s*:)/g)].map((m) => m[0]));
  }

  it("grey declares every custom property light declares", () => {
    const light = declaredProps(':root[data-theme="light"]');
    const grey = declaredProps(':root[data-theme="grey"]');
    const missing = [...light].filter((p) => !grey.has(p));
    expect(missing).toEqual([]);
  });

  it("dark declares every custom property light declares", () => {
    const light = declaredProps(':root[data-theme="light"]');
    const dark = declaredProps(':root[data-theme="dark"]');
    const missing = [...light].filter((p) => !dark.has(p));
    expect(missing).toEqual([]);
  });

  it("no partial system-theme override block remains", () => {
    expect(css).not.toContain('data-theme="system"');
  });
});
