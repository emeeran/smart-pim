/**
 * Session-local shell state shared between the nav tree and the modules that
 * consume it (Calendar owns the mini-month, Notes view tracking is opt-in).
 * Deliberately kept out of store.ts so the core store stays module-agnostic.
 */
import { signal } from "@preact/signals";

/** yyyy-mm-dd picked in the nav mini month; Calendar switches to Day view. */
export const miniMonthPick = signal<string | null>(null);

export function pickMiniMonth(date: string): void {
  miniMonthPick.value = date;
}

/** Highlighted row of the nav secondary tree, keyed "<module>:<row>". */
export const smartListSel = signal<string | null>(null);

/**
 * Note ids opened this session, most recent first. Nothing populates it yet —
 * the Notes module keeps its selection local, so "Recently Viewed" stays empty
 * (and reports 0) until that selection is lifted into a shared signal.
 */
export const recentlyViewedNotes = signal<number[]>([]);
