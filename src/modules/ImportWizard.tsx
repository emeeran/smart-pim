/**
 * Import wizard (S13 / T3.44): CSV → column mapping preview → dry-run
 * validation report → single-transaction commit. ICS/VCF reuse their
 * module import paths.
 */
import { useEffect, useState } from "preact/hooks";
import { api, type CsvPreview, type ImportReport } from "../api";
import { detectEpimCsv, type EpimPresetMatch } from "../lib/epim_csv";
import { refreshCounts } from "../store";
import "../design/io.css";

export const importOpen = { value: false };
type Listener = () => void;
const listeners = new Set<Listener>();

export function openImport(): void {
  importOpen.value = true;
  listeners.forEach((l) => l());
}

export function ImportWizard() {
  const [open, setOpen] = useState(false);
  const [preview, setPreview] = useState<CsvPreview | null>(null);
  const [text, setText] = useState("");
  const [mapping, setMapping] = useState<Record<string, number>>({});
  const [report, setReport] = useState<ImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [preset, setPreset] = useState<EpimPresetMatch | null>(null);

  useEffect(() => {
    const l: Listener = () => setOpen(true);
    listeners.add(l);
    return () => listeners.delete(l);
  }, []);

  if (!open && !importOpen.value) return null;
  const close = () => {
    importOpen.value = false;
    setOpen(false);
    setPreview(null);
    setText("");
    setReport(null);
    setError(null);
    setDone(null);
    setPreset(null);
  };

  const pick = async (file: File) => {
    setError(null);
    setReport(null);
    setDone(null);
    setPreset(null);
    const content = await file.text();
    setText(content);
    const lower = file.name.toLowerCase();
    if (lower.endsWith(".csv")) {
      try {
        const p = await api.importCsvPreview(content);
        setPreview(p);
        const match = detectEpimCsv(p.headers);
        // An EPIM layout pre-empts the generic guess; the user can still
        // override any column from the selects below. Date columns need no
        // transformation — the backend parses MM/DD/YYYY, DD.MM.YYYY and
        // YYYY-MM-DD.
        setMapping(match ? match.mapping : p.mapping);
        setPreset(match);
      } catch (e) {
        setError(String(e));
      }
    } else if (lower.endsWith(".ics")) {
      const res = await api.eventsImportIcs(content);
      setDone(`Imported ${res.imported} event(s).`);
      void refreshCounts();
    } else if (lower.endsWith(".vcf")) {
      const res = await api.contactsImportVcf(content);
      setDone(`Imported ${res.imported} contact(s).`);
      void refreshCounts();
    } else {
      setError("Unsupported file type — use .csv, .ics or .vcf.");
    }
  };

  return (
    <div class="overlay" onClick={close}>
      <div class="dialog import-wizard" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Import">
        <h3>Import</h3>
        {!preview && (
          <label class="settings-row">
            Choose a .csv, .ics or .vcf file (never modified)
            <input
              type="file"
              accept=".csv,.ics,.vcf"
              onChange={(e) => {
                const f = (e.target as HTMLInputElement).files?.[0];
                if (f) void pick(f);
              }}
            />
          </label>
        )}
        {preview && (
          <>
            {preset && (
              <div class="io-banner">
                Detected: {preset.label} — column mapping applied automatically.
                <button onClick={() => setMapping(preview.mapping)}>Use default mapping</button>
              </div>
            )}
            <p class="muted">
              {preview.total_rows} rows. Map columns to contact fields:
            </p>
            <div class="import-mapping">
              {preview.contact_fields.map((field) => (
                <label class="settings-row" key={field}>
                  {field}
                  <select
                    value={mapping[field] ?? -1}
                    onChange={(e) =>
                      setMapping((m) => ({
                        ...m,
                        [field]: Number((e.target as HTMLSelectElement).value),
                      }))
                    }
                  >
                    <option value={-1}>—</option>
                    {preview.headers.map((h, i) => (
                      <option key={i} value={i}>{h}</option>
                    ))}
                  </select>
                </label>
              ))}
            </div>
            {report && (
              <div class="import-report">
                <b>Validation report</b> — {report.total_rows} rows, {report.inserted} inserted,{" "}
                {report.updated} updated, {report.skipped} skipped.
                <ul>
                  {report.issues.map((iss, i) => (
                    <li key={i} class={iss.severity === "error" ? "lock-error" : "muted"}>
                      Row {iss.row + 1}: {iss.message}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {done && <div class="muted">{done}</div>}
            {error && <div class="lock-error">{error}</div>}
            <div class="dialog-actions">
              <button
                onClick={async () => {
                  try {
                    setReport(await api.importCsvValidate(text, mapping));
                  } catch (e) {
                    setError(String(e));
                  }
                }}
              >
                Validate (dry run)
              </button>
              <button
                class="primary"
                onClick={async () => {
                  try {
                    const rep = await api.importCsvCommit(text, mapping);
                    if (rep.issues.some((i) => i.severity === "error")) {
                      setReport(rep);
                      setError("Import aborted — nothing was written. Fix the errors and retry.");
                    } else {
                      setDone(`Imported ${rep.inserted} contact(s).`);
                      void refreshCounts();
                    }
                  } catch (e) {
                    setError(String(e));
                  }
                }}
              >
                Import
              </button>
              <button onClick={close}>Close</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
