/**
 * Menu bar (S1): File, Edit, View, Go, Actions, Tools, Help — mirroring the
 * reference structure with our equivalents. Items without a Gate 3 action
 * behind them are present but inert (module features arrive in Gate 4).
 */
import { useState } from "preact/hooks";
import { useEffect, useRef } from "preact/hooks";
import { api } from "../api";
import { ExportDialog, openExport } from "../components/ExportDialog";
import { openImport } from "../modules/ImportWizard";
import { openRules } from "../modules/RulesDialog";
import { activeModule, MODULES, searchOpen, settingsOpen } from "../store";

interface MenuItem {
  label: string;
  shortcut?: string;
  action?: () => void;
}
interface Menu {
  label: string;
  items: MenuItem[];
}

export function MenuBar() {
  const [open, setOpen] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (root.current && !root.current.contains(e.target as Node)) setOpen(null);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, []);

  const lock = async () => {
    await api.lock();
  };
  const backup = async () => {
    try {
      await api.backupNow();
    } catch (e) {
      alert(`Backup failed: ${e}`);
    }
  };

  const menus: Menu[] = [
    {
      label: "File",
      items: [
        { label: "Import…", action: () => openImport() },
        { label: "Export…", action: () => openExport() },
        { label: "Lock Database", shortcut: "Ctrl+Shift+L", action: lock },
        { label: "Exit", shortcut: "Alt+F4", action: () => window.close() },
      ],
    },
    {
      label: "Edit",
      items: [{ label: "Undo", shortcut: "Ctrl+Z", action: () => void api.undo() }],
    },
    { label: "View", items: [{ label: "Theme & Layout…", action: () => settingsOpen.value = true }] },
    {
      label: "Go",
      items: MODULES.map((m, i) => ({
        label: m.label,
        shortcut: `Ctrl+${i + 1}`,
        action: () => (activeModule.value = m.id),
      })),
    },
    { label: "Actions", items: [{ label: "Search Everywhere", shortcut: "Ctrl+Q", action: () => (searchOpen.value = true) }] },
    {
      label: "Tools",
      items: [
        { label: "Mail Rules…", action: () => openRules() },
        { label: "Backup Now", action: backup },
        { label: "Settings…", action: () => (settingsOpen.value = true) },
      ],
    },
    { label: "Help", items: [{ label: "About Smart PIM", action: () => alert("Smart PIM 0.0.1 — local-first personal information manager.\nOriginal clean-room implementation.") }] },
  ];

  return (
    <>
      <div class="menubar" ref={root} role="menubar">
      <span class="menubar-title">Smart PIM</span>
      {menus.map((m) => (
        <div key={m.label} class="menubar-menu">
          <button
            class="menubar-label"
            aria-expanded={open === m.label}
            onClick={() => setOpen(open === m.label ? null : m.label)}
            onMouseEnter={() => open && setOpen(m.label)}
          >
            {m.label}
          </button>
          {open === m.label && (
            <div class="menu-popup" role="menu">
              {m.items.map((item) => (
                <button
                  key={item.label}
                  role="menuitem"
                  onClick={() => {
                    setOpen(null);
                    item.action?.();
                  }}
                >
                  <span>{item.label}</span>
                  {item.shortcut && <kbd>{item.shortcut}</kbd>}
                </button>
              ))}
            </div>
          )}
        </div>
      ))}
      </div>
      <ExportDialog />
    </>
  );
}
