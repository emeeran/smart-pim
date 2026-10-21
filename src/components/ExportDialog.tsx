/**
 * Export dialog (T3.44 / F-003 interchange): per-module export to CSV, ICS,
 * VCF, EML, HTML, RTF, XML or Markdown. No native save dialog is available, so
 * the destination is a plain path field defaulting under ~/Documents.
 */
import { useEffect, useState } from "preact/hooks";
import { api, type ExportFormat, type ExportKind } from "../api";
import "../design/io.css";

export const exportOpen = { value: false };
const listeners = new Set<() => void>();

export function openExport(): void {
  exportOpen.value = true;
  listeners.forEach((l) => l());
}

const MODULES: { kind: ExportKind; label: string }[] = [
  { kind: "tasks", label: "Tasks" },
  { kind: "notes", label: "Notes" },
  { kind: "events", label: "Calendar Events" },
  { kind: "contacts", label: "Contacts" },
  { kind: "diary", label: "Diary" },
  { kind: "messages", label: "Mail Messages" },
];

/** Formats each module can round-trip; the picker only offers these. */
const FORMATS: Record<ExportKind, ExportFormat[]> = {
  tasks: ["csv", "html", "rtf", "xml", "md"],
  notes: ["csv", "html", "rtf", "xml", "md"],
  diary: ["csv", "html", "rtf", "xml", "md"],
  events: ["csv", "ics", "html", "xml", "md"],
  contacts: ["csv", "vcf", "html", "rtf", "xml", "md"],
  messages: ["csv", "eml", "html", "rtf", "xml", "md"],
};

const FORMAT_LABELS: Record<ExportFormat, string> = {
  csv: "CSV (spreadsheet)",
  ics: "iCalendar (.ics)",
  vcf: "vCard (.vcf)",
  eml: "E-mail (.eml)",
  html: "HTML",
  rtf: "Rich Text (.rtf)",
  xml: "XML",
  md: "Markdown (.md)",
};

/** ~/Documents/smart-pim-export/<module>-<YYYY-MM-DD>.<ext>, as a plain string. */
function defaultPath(kind: ExportKind, format: ExportFormat): string {
  const day = new Date().toISOString().slice(0, 10);
  return `~/Documents/smart-pim-export/${kind}-${day}.${format}`;
}

export function ExportDialog() {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<ExportKind>("tasks");
  const [format, setFormat] = useState<ExportFormat>("csv");
  const [path, setPath] = useState(defaultPath("tasks", "csv"));
  const [result, setResult] = useState<{ path: string; count: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const l: () => void = () => setOpen(true);
    listeners.add(l);
    return () => listeners.delete(l);
  }, []);

  if (!open && !exportOpen.value) return null;
  const close = () => {
    exportOpen.value = false;
    setOpen(false);
    setResult(null);
    setError(null);
  };

  const pickModule = (next: ExportKind) => {
    const formats = FORMATS[next];
    const fmt = formats.includes(format) ? format : formats[0];
    setKind(next);
    setFormat(fmt);
    setPath(defaultPath(next, fmt));
    setResult(null);
    setError(null);
  };

  const pickFormat = (fmt: ExportFormat) => {
    setFormat(fmt);
    setPath(defaultPath(kind, fmt));
  };

  const run = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const res = await api.exportModule(kind, format, path.trim());
      setResult({ path: res.path, count: res.count });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="overlay" onClick={close}>
      <div class="dialog import-wizard" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Export">
        <h3>Export</h3>
        <div class="export-form">
          <label for="export-module">Module</label>
          <select
            id="export-module"
            value={kind}
            onChange={(e) => pickModule((e.target as HTMLSelectElement).value as ExportKind)}
          >
            {MODULES.map((m) => (
              <option key={m.kind} value={m.kind}>{m.label}</option>
            ))}
          </select>
          <label for="export-format">Format</label>
          <select
            id="export-format"
            value={format}
            onChange={(e) => pickFormat((e.target as HTMLSelectElement).value as ExportFormat)}
          >
            {FORMATS[kind].map((f) => (
              <option key={f} value={f}>{FORMAT_LABELS[f]}</option>
            ))}
          </select>
          <label for="export-path">Destination</label>
          <input
            id="export-path"
            type="text"
            spellcheck={false}
            value={path}
            onInput={(e) => setPath((e.target as HTMLInputElement).value)}
          />
        </div>
        <p class="export-hint">
          Relative paths resolve inside the database folder. Existing files are overwritten.
        </p>
        {result && (
          <div class="export-result">
            Exported {result.count} item(s) to <b>{result.path}</b>
          </div>
        )}
        {error && <div class="lock-error">{error}</div>}
        <div class="dialog-actions">
          <button class="primary" disabled={busy || !path.trim()} onClick={() => void run()}>
            {busy ? "Exporting…" : "Export"}
          </button>
          <button onClick={close}>Close</button>
        </div>
      </div>
    </div>
  );
}
