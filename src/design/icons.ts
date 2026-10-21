/**
 * Original, code-drawn SVG icon set (T0.4). 24×24 grid, 1.6px strokes,
 * currentColor — no third-party assets (clean-room guardrail, A-5).
 */

export type IconName =
  | "today"
  | "mail"
  | "calendar"
  | "tasks"
  | "notes"
  | "contacts"
  | "passwords"
  | "trash"
  | "settings"
  | "account"
  | "search"
  | "bell"
  | "restore"
  | "purge"
  | "undo"
  | "menu"
  | "plus"
  | "chevron-down"
  | "diary"
  | "year"
  | "agenda"
  | "tree"
  | "kanban"
  | "link"
  | "star";

function svg(body: string): string {
  return `<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
}

export function icon(name: IconName): string {
  switch (name) {
    case "today":
      return svg(
        `<circle cx="12" cy="12" r="8.2"/><circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none"/><path d="M12 3.8v2M12 18.2v2M3.8 12h2M18.2 12h2"/>`,
      );
    case "mail":
      return svg(
        `<rect x="3" y="5.5" width="18" height="13" rx="2"/><path d="m3.8 7 8.2 6 8.2-6"/>`,
      );
    case "calendar":
      return svg(
        `<rect x="3.5" y="5" width="17" height="15.5" rx="2"/><path d="M3.5 9.5h17M8 3v3.4M16 3v3.4"/>`,
      );
    case "tasks":
      return svg(
        `<rect x="4" y="4" width="16" height="16" rx="3"/><path d="m8 12.2 2.6 2.6L16 9.4"/>`,
      );
    case "notes":
      return svg(
        `<path d="M6 3.5h9.5L19 7v13.5H6z"/><path d="M15 3.5V7h4M9 11.5h6M9 15h6"/>`,
      );
    case "contacts":
      return svg(
        `<circle cx="12" cy="8.6" r="3.4"/><path d="M5.2 19.4c1.2-3.3 3.8-5 6.8-5s5.6 1.7 6.8 5"/>`,
      );
    case "passwords":
      return svg(
        `<rect x="5" y="10.5" width="14" height="9" rx="2"/><path d="M8.2 10.5V8a3.8 3.8 0 0 1 7.6 0v2.5M12 14.2v2"/>`,
      );
    case "trash":
      return svg(
        `<path d="M5 7h14M9.5 7V4.8h5V7M7 7l.9 12.5h8.2L17 7M10.2 10.5v5.6M13.8 10.5v5.6"/>`,
      );
    case "settings":
      return svg(
        `<circle cx="12" cy="12" r="3"/><path d="M12 3.4v2.4M12 18.2v2.4M3.4 12h2.4M18.2 12h2.4M5.8 5.8l1.7 1.7M16.5 16.5l1.7 1.7M18.2 5.8l-1.7 1.7M7.5 16.5l-1.7 1.7"/>`,
      );
    case "account":
      return svg(
        `<circle cx="12" cy="9" r="3.2"/><circle cx="12" cy="12" r="9" stroke-dasharray="2.4 3"/>`,
      );
    case "search":
      return svg(
        `<circle cx="10.8" cy="10.8" r="5.6"/><path d="m15.2 15.2 4.4 4.4"/>`,
      );
    case "bell":
      return svg(
        `<path d="M6.4 16.5c1-1 1.2-2.6 1.2-4.5a4.4 4.4 0 0 1 8.8 0c0 1.9.2 3.5 1.2 4.5zM10.4 19a1.7 1.7 0 0 0 3.2 0"/>`,
      );
    case "restore":
      return svg(
        `<path d="M4.8 9.6A7.6 7.6 0 1 1 4.4 14"/><path d="M4.4 5v4.8h4.8"/>`,
      );
    case "purge":
      return svg(
        `<path d="m6 6 12 12M18 6 6 18"/>`,
      );
    case "undo":
      return svg(
        `<path d="M8.5 5.5 4.5 9.5l4 4"/><path d="M4.5 9.5h9a6 6 0 0 1 0 12h-3"/>`,
      );
    case "menu":
      return svg(`<path d="M4 7h16"/><path d="M4 12h16"/><path d="M4 17h16"/>`);
    case "plus":
      return svg(`<path d="M12 5v14"/><path d="M5 12h14"/>`);
    case "chevron-down":
      return svg(`<path d="m6 9 6 6 6-6"/>`);
    case "diary":
      return svg(
        `<rect x="5" y="3.5" width="14" height="17" rx="2"/><path d="M9 3.5v17"/><path d="M13 8h3"/><path d="M13 12h3"/>`,
      );
    case "year":
      return svg(
        `<rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 9h16"/><path d="M8 12.5h2"/><path d="M14 12.5h2"/><path d="M8 16h2"/><path d="M14 16h2"/>`,
      );
    case "agenda":
      return svg(`<path d="M5 6h14"/><path d="M5 10h10"/><path d="M5 14h14"/><path d="M5 18h10"/>`);
    case "tree":
      return svg(
        `<path d="M6 5h4"/><path d="M10 7.5h4"/><path d="M10 12h5"/><path d="M8 5v12"/><path d="M8 17h0"/><path d="M8 12a2 2 0 1 0 0-.01"/>`,
      );
    case "kanban":
      return svg(
        `<rect x="4" y="4.5" width="4.5" height="15" rx="1"/><rect x="10" y="4.5" width="4.5" height="10" rx="1"/><rect x="16" y="4.5" width="4.5" height="13" rx="1"/>`,
      );
    case "link":
      return svg(
        `<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 1 0-5.7-5.7l-1.5 1.5"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 1 0 5.7 5.7l1.5-1.5"/>`,
      );
    case "star":
      return svg(
        `<path d="m12 4 2.4 4.9 5.4.8-3.9 3.8.9 5.4-4.8-2.5-4.8 2.5.9-5.4L4.2 9.7l5.4-.8z"/>`,
      );
  }
}
