/**
 * Theme resolution (S6): the user picks a *preference* (light / grey / dark /
 * system); "system" resolves live against the OS `prefers-color-scheme`. The
 * resolved theme lands on `<html data-theme>` — every token block in
 * tokens.css is keyed by the resolved value, never the preference.
 */

export type ThemePref = "light" | "grey" | "dark" | "system";
export type ResolvedTheme = "light" | "grey" | "dark";

export const THEME_KEY = "pf.theme";

export function resolveTheme(pref: ThemePref, prefersDark: boolean): ResolvedTheme {
  if (pref !== "system") return pref;
  return prefersDark ? "dark" : "light";
}

export function applyTheme(pref: ThemePref): void {
  const prefersDark =
    typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
  document.documentElement.dataset.theme = resolveTheme(pref, prefersDark);
  document.documentElement.dataset.themePref = pref;
  localStorage.setItem(THEME_KEY, pref);
}

/** Watch OS scheme changes; returns an unsubscribe function. */
export function watchSystem(cb: (prefersDark: boolean) => void): () => void {
  if (typeof matchMedia !== "function") return () => {};
  const mq = matchMedia("(prefers-color-scheme: dark)");
  const onChange = (e: MediaQueryListEvent) => cb(e.matches);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}
