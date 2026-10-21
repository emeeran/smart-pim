/**
 * Status bar (S5), F-004 parity: "Items: N" for the current view, calendar
 * duration slot, unread-mail badge, bell with the pending-reminder count,
 * word count + zoom slider for notes.
 */
import { signal } from "@preact/signals";
import { activeModule, counts, jumpTo } from "../store";
import { icon } from "../design/icons";

/**
 * Editor zoom percent. The Notes module keeps its editor local, so nothing
 * consumes this yet — see the TODO at the zoom slider below.
 */
export const zoomLevel = signal(100);

export function StatusBar({ viewCount }: { viewCount: number }) {
  const c = counts.value;
  const reminders = c?.pending_reminders ?? 0;
  const unread = c?.unread_messages ?? 0;

  return (
    <footer class="statusbar">
      <span>Items: {viewCount.toLocaleString()}</span>
      {activeModule.value === "calendar" && (
        // TODO(duration): occurrence totals live in the Calendar module's local
        // state; lift them into a shared signal to report "Duration: …".
        <span class="muted" title="Total event time is not available yet">
          Duration: —
        </span>
      )}
      <span class="statusbar-spacer" />
      {activeModule.value === "mail" && c && (
        <span>
          {unread.toLocaleString()} unread of {c.messages.toLocaleString()}
        </span>
      )}
      {activeModule.value !== "mail" && unread > 0 && (
        <span class="statusbar-unread" title={`${unread} unread message(s)`}>
          <span dangerouslySetInnerHTML={{ __html: icon("mail") }} />
          {unread.toLocaleString()}
        </span>
      )}
      <button
        class="statusbar-bell"
        title={reminders > 0 ? `${reminders} due reminder(s) — see Today` : "No due reminders"}
        onClick={() => jumpTo("today", "reminders", 0)}
      >
        <span dangerouslySetInnerHTML={{ __html: icon("bell") }} />
        {reminders > 0 && <span class="rail-badge">{reminders}</span>}
      </button>
      {/*
        Word count: the Notes editor keeps its text (and word count) in module
        state — no signal to subscribe to, so no number is shown rather than a
        fake one. The zoom slider is wired to the zoomLevel signal above.
        TODO(zoom): have Notes consume zoomLevel for its editor font size.
      */}
      {activeModule.value === "notes" && (
        <label class="statusbar-zoom">
          <input
            type="range"
            min={50}
            max={200}
            step={10}
            value={zoomLevel.value}
            aria-label="Zoom"
            onInput={(e) => (zoomLevel.value = Number((e.target as HTMLInputElement).value))}
          />
          {zoomLevel.value}%
        </label>
      )}
    </footer>
  );
}
