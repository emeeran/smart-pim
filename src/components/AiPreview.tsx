/**
 * AiPreview: shared "the model produced this — apply or discard" panel.
 * Every AI write path renders through here so preview-then-apply stays
 * uniform (ai_integ_plan §1: AI never writes without a diff preview).
 * Body is plain text/sections, or a children slot for interactive bodies
 * (checklists, editable rule rows). Actions replace the footer buttons.
 */
import { useEffect, useRef } from "preact/hooks";
import type { ComponentChildren, JSX } from "preact";
import "../design/dialogs.css";
import "../design/ai.css";

export interface AiSection {
  heading: string;
  text: string;
}

export interface AiPreviewAction {
  label: string;
  onClick: () => void;
  primary?: boolean;
  disabled?: boolean;
}

export interface AiPreviewProps {
  open: boolean;
  title: string;
  onClose: () => void;
  /** Plain text, or labelled sections. Interactive bodies use children. */
  body?: string | AiSection[];
  children?: ComponentChildren;
  busy?: boolean;
  busyLabel?: string;
  /** Backend error text — shown inline, never thrown. */
  error?: string | null;
  actions?: AiPreviewAction[];
  width?: number;
}

const FOCUSABLE =
  'button, input, select, textarea, a[href], [tabindex]:not([tabindex="-1"])';

export function AiPreview({
  open,
  title,
  onClose,
  body,
  children,
  busy = false,
  busyLabel = "Working…",
  error,
  actions = [],
  width,
}: AiPreviewProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    ref.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) {
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose, busy]);

  if (!open) return null;

  const sections: AiSection[] | null =
    body === undefined || typeof body === "string"
      ? null
      : body;

  return (
    <div class="overlay" onClick={busy ? undefined : onClose}>
      <div
        ref={ref}
        class="dialog ai-preview"
        style={width ? { width: `${width}px` } : undefined}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={(e: JSX.TargetedMouseEvent<HTMLDivElement>) => e.stopPropagation()}
      >
        <h3>{title}</h3>
        {busy && (
          <div class="ai-busy" role="status">
            <span class="ai-busy-spin" aria-hidden="true" />
            {busyLabel}
          </div>
        )}
        {error && <div class="ai-error">{error}</div>}
        {!busy && (
          <div class="ai-preview-body">
            {typeof body === "string" && body.trim() !== "" && (
              <pre class="ai-text">{body}</pre>
            )}
            {sections?.map((s) => (
              <section key={s.heading} class="ai-section">
                <h4>{s.heading}</h4>
                <pre class="ai-text">{s.text}</pre>
              </section>
            ))}
            {children}
          </div>
        )}
        <div class="form-footer">
          <span class="form-footer-spacer" />
          {actions.map((a) => (
            <button
              key={a.label}
              class={a.primary ? "form-ok" : undefined}
              disabled={busy || a.disabled}
              onClick={a.onClick}
            >
              {a.label}
            </button>
          ))}
          <button onClick={onClose} disabled={busy}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
