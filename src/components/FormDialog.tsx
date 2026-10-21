/**
 * FormDialog: shared modal scaffold for the EPIM-style editors — overlay,
 * title bar, body slot, footer with Delete / Save & New / OK / Cancel slots,
 * Escape closes, basic Tab focus trap.
 */
import { useEffect, useRef } from "preact/hooks";
import type { ComponentChildren, JSX } from "preact";
import "../design/dialogs.css";

export interface FormDialogProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ComponentChildren;
  /** Wider editors (event, settings) pass a px width. */
  width?: number;
  /** Footer slots — omitted slots render nothing. */
  onDelete?: () => void;
  deleteLabel?: string;
  onSaveNew?: () => void;
  onOk: () => void;
  okLabel?: string;
  cancelLabel?: string;
  /** Save failure text shown above the footer. */
  error?: string | null;
}

const FOCUSABLE =
  'button, input, select, textarea, a[href], [tabindex]:not([tabindex="-1"])';

export function FormDialog({
  open,
  title,
  onClose,
  children,
  width,
  onDelete,
  deleteLabel = "Delete",
  onSaveNew,
  onOk,
  okLabel = "OK",
  cancelLabel = "Cancel",
  error,
}: FormDialogProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const root = ref.current;
    const first = root?.querySelector<HTMLElement>(FOCUSABLE);
    first?.focus();

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== "Tab" || !root) return;
      // Cycle focus inside the dialog (basic trap).
      const items = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
        (el) => !el.hasAttribute("disabled"),
      );
      if (items.length === 0) return;
      const idx = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey
        ? items[(idx <= 0 ? items.length : idx) - 1]
        : items[(idx + 1) % items.length];
      e.preventDefault();
      next.focus();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const stop = (e: JSX.TargetedMouseEvent<HTMLDivElement>) => e.stopPropagation();

  return (
    <div class="overlay" onClick={onClose}>
      <div
        ref={ref}
        class="dialog form-dialog"
        style={width ? { width: `${width}px` } : undefined}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={stop}
      >
        <h3>{title}</h3>
        {children}
        {error && <div class="form-dialog-error">{error}</div>}
        <div class="form-footer">
          {onDelete && (
            <button class="danger-text" onClick={onDelete}>
              {deleteLabel}
            </button>
          )}
          <span class="form-footer-spacer" />
          {onSaveNew && (
            <button onClick={onSaveNew}>Save &amp; New</button>
          )}
          <button onClick={onClose}>{cancelLabel}</button>
          <button class="form-ok" onClick={onOk}>
            {okLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
