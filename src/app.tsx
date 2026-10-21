/**
 * App root: lock screen ↔ shell, keyboard map (Ctrl+1..N, Ctrl+Q,
 * Ctrl+Shift+L), module routing, status bar. Today is a full-width dashboard;
 * other modules run the shared layout (S3).
 */
import { useEffect } from "preact/hooks";
import { api } from "./api";
import { GlobalSearch } from "./shell/GlobalSearch";
import { AppToolbar } from "./shell/AppToolbar";
import { NavTree } from "./shell/NavTree";
import { LockScreen } from "./shell/LockScreen";
import { MenuBar } from "./shell/MenuBar";
import { StatusBar } from "./shell/StatusBar";
import { ThreePane } from "./shell/ThreePane";
import { MailModule } from "./modules/Mail";
import { NotesModule } from "./modules/Notes";
import { DiaryModule } from "./modules/Diary";
import { CalendarModule } from "./modules/Calendar";
import { TasksModule } from "./modules/Tasks";
import { ContactsModule } from "./modules/Contacts";
import { PasswordsModule } from "./modules/Passwords";
import { Today } from "./modules/Today";
import { Trash } from "./modules/Trash";
import { Settings } from "./modules/Settings";
import { ImportWizard } from "./modules/ImportWizard";
import { RulesDialog } from "./modules/RulesDialog";
import { MODULES } from "./store";
import {
  activeModule,
  locked,
  moduleByIndex,
  refreshCounts,
  restoreTheme,
  searchOpen,
  settingsOpen,
  viewCount,
} from "./store";

const ACTIVITY_EVENTS = ["mousemove", "keydown", "mousedown", "wheel"] as const;

export function App() {
  useEffect(() => {
    restoreTheme();
    void api.listen("session:locked", () => (locked.value = true));
    void api.listen("session:unlocked", () => {
      locked.value = false;
      void refreshCounts();
    });

    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return;
      if (e.shiftKey && e.code === "KeyL") {
        e.preventDefault();
        void api.lock().then(() => (locked.value = true));
        return;
      }
      if (e.code === "KeyQ") {
        e.preventDefault();
        searchOpen.value = true;
        return;
      }
      const n = Number(e.key);
      if (n >= 1 && n <= MODULES.length) {
        const m = moduleByIndex(n - 1);
        if (m) {
          e.preventDefault();
          activeModule.value = m;
        }
      }
    };
    const beat = () => void api.heartbeat();
    document.addEventListener("keydown", onKey);
    // user activity keeps the auto-lock timer fed (S7)
    for (const ev of ACTIVITY_EVENTS) {
      document.addEventListener(ev, beat, { passive: true });
    }
    return () => {
      document.removeEventListener("keydown", onKey);
      for (const ev of ACTIVITY_EVENTS) {
        document.removeEventListener(ev, beat);
      }
    };
  }, []);

  useEffect(() => {
    if (locked.value) return;
    void refreshCounts();
    void api.dbStatus().then((s) => void api.setTitle(s.title));
  }, [locked.value]);

  useEffect(() => {
    viewCount.value = 0; // list modules report their own count on mount
  }, [activeModule.value]);

  // Reminder poll (S30): fires each due reminder once; missed fires are
  // recovered on start and flagged.
  useEffect(() => {
    if (locked.value) return;
    const tick = async () => {
      try {
        const due = await api.remindersTake();
        for (const r of due) {
          const prefix = r.missed ? "MISSED reminder" : "Reminder";
          alert(`${prefix}: item #${r.item_id} (due ${new Date(r.due_at).toLocaleString()})`);
        }
        if (due.length > 0) void refreshCounts();
      } catch {
        /* locked */
      }
    };
    void tick();
    const t = window.setInterval(tick, 30_000);
    return () => window.clearInterval(t);
  }, [locked.value]);

  if (locked.value) {
    return <LockScreen onUnlocked={() => (locked.value = false)} />;
  }

  const body = {
    today: <Today />,
    mail: <MailModule />,
    calendar: <CalendarModule />,
    tasks: <TasksModule />,
    notes: <NotesModule />,
    diary: <DiaryModule />,
    contacts: <ContactsModule />,
    passwords: <PasswordsModule />,
    trash: <Trash />,
  }[activeModule.value] ?? (
    <div class="muted empty-pane">This module's views arrive with Gate 4 — one feature per commit.</div>
  );
  const wide = activeModule.value === "today";

  return (
    <div class="app">
      <MenuBar />
      <AppToolbar />
      <div class="app-body">
        <NavTree
          onSwitch={(m) => (activeModule.value = m)}
          onSettings={() => (settingsOpen.value = true)}
        />
        <div class={wide ? "app-main app-main-single" : "app-main"}>
          {activeModule.value === "trash" ? (
            <ThreePane
              module="trash"
              tree={<div class="muted empty-pane">Deleted items, grouped by kind.</div>}
              list={body}
              detail={<div class="muted empty-pane">Select an item to see details.</div>}
            />
          ) : (
            body
          )}
          <StatusBar viewCount={wide ? 1 : viewCount.value} />
        </div>
      </div>
      <GlobalSearch />
      <Settings />
      <ImportWizard />
      <RulesDialog />
    </div>
  );
}
