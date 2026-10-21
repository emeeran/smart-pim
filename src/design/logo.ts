/**
 * Original Smart PIM logo mark (clean-room: our own drawing, no third-party
 * assets). Inlined as a data URI so it works inside the Tauri webview and in
 * static previews alike. The mark: three stacked layers (organised
 * information) with one accent bar — kept simple so it stays legible at 16px.
 */

const BODY = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="{s}" height="{s}"><rect x="16" y="16" width="480" height="480" rx="96" fill="#1e293b"/><rect x="128" y="112" width="256" height="64" rx="32" fill="#f1f5f9"/><rect x="128" y="224" width="256" height="64" rx="32" fill="#f1f5f9" opacity="0.85"/><rect x="128" y="336" width="160" height="64" rx="32" fill="#3b82f6"/></svg>`;

export function logo(size: number): string {
  return `data:image/svg+xml,${encodeURIComponent(BODY.replace(/\{s\}/g, String(size)))}`;
}
