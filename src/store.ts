/**
 * Shell-wide signals: session state, active module (Ctrl+1..8), counts,
 * theme. Kept deliberately small — module-local state lives in modules/.
 */
import { signal } from "@preact/signals";
import { api, type Counts } from "./api";
import { applyTheme as themeApply, watchSystem, THEME_KEY, type ThemePref } from "./lib/theme";

export type ModuleId =
  | "today"
  | "mail"
  | "calendar"
  | "tasks"
  | "notes"
  | "diary"
  | "contacts"
  | "passwords"
  | "trash";

/** S2 navigation ordering — Ctrl+1..N map onto this array. */
export const MODULES: readonly { id: ModuleId; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "mail", label: "Mail" },
  { id: "calendar", label: "Calendar" },
  { id: "tasks", label: "Tasks" },
  { id: "notes", label: "Notes" },
  { id: "diary", label: "Diary" },
  { id: "contacts", label: "Contacts" },
  { id: "passwords", label: "Passwords" },
  { id: "trash", label: "Trash" },
];

export const locked = signal(true);
export const activeModule = signal<ModuleId>("today");
export const counts = signal<Counts | null>(null);

/** Cross-module jump request (Related Items click-through): navigate to
 * `module` and, if the module understands the target, select it. Modules
 * consume and clear it. */
export interface JumpTarget {
  module: ModuleId;
  kind: string; // message | task | note | contact | event | diary
  id: number;
}
export const jumpTarget = signal<JumpTarget | null>(null);

export function jumpTo(module: ModuleId, kind: string, id: number): void {
  jumpTarget.value = { module, kind, id };
  activeModule.value = module;
}
/** Item count of the current view — drives the status bar (S5). */
export const viewCount = signal(0);
export const searchOpen = signal(false);
export const settingsOpen = signal(false);
export type Theme = ThemePref;
export const theme = signal<Theme>("system");

export async function refreshCounts(): Promise<void> {
  try {
    counts.value = await api.counts();
  } catch {
    /* locked or not-yet-unlocked; badges refresh on unlock */
  }
}

export function applyTheme(next: Theme): void {
  theme.value = next;
  themeApply(next);
}

export function restoreTheme(): void {
  const saved = localStorage.getItem(THEME_KEY) as Theme | null;
  applyTheme(saved ?? "system");
  // Live-follow the OS scheme while the preference is "system".
  watchSystem(() => {
    if (theme.value === "system") themeApply("system");
  });
}

export function moduleByIndex(i: number): ModuleId | null {
  return MODULES[i]?.id ?? null;
}
