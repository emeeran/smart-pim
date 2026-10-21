/**
 * Settings dialog (S6 / S57): a sectioned preferences pane — Appearance
 * (theme), Security (auto-lock minutes), Backup (create / list / restore),
 * Sync → Devices (Wi-Fi advertise + pairing + device list + sync reports),
 * Sync → Cloud (CalDAV / CardDAV / WebDAV profiles) and AI (provider,
 * endpoint, model, per-feature opt-ins, embedding reindex). Backend commands
 * that have not landed yet surface their error text inline instead of crashing.
 */
import { useEffect, useState } from "preact/hooks";
import {
  api,
  type AiSettings,
  type AiUsage,
  type BackupInfo,
  type DeviceInfo,
  type DavProfile,
  type SyncReport,
  type DavReport,
} from "../api";
import { applyTheme, settingsOpen, theme, type Theme } from "../store";
import { FormDialog } from "../components/FormDialog";
import { AI_FEATURES, aiStatus, errText, refreshAiStatus } from "../lib/ai";
import "../design/dialogs.css";
import "../design/ai.css";

const AUTO_LOCK_KEY = "auto_lock_minutes"; // src-tauri/src/state.rs AUTO_LOCK_KEY

type Section = "appearance" | "security" | "backup" | "devices" | "cloud" | "ai";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "security", label: "Security" },
  { id: "backup", label: "Backup" },
  { id: "devices", label: "Sync → Devices" },
  { id: "cloud", label: "Sync → Cloud" },
  { id: "ai", label: "AI" },
];

const FEATURE_LABELS: Record<string, string> = {
  summarize: "Note summaries",
  digest: "Mail thread digests",
  reply: "Reply drafting",
  capture: "Natural-language capture",
  action_items: "Action-item extraction",
  auto_tag: "Auto-tagging",
  diary: "Diary reflection",
  triage: "Mail triage suggestions",
  subtasks: "Task subtask splitting",
  contacts: "Signature → contact",
  semantic: "Semantic search",
};

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function fmtWhen(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/** Small state machine for a loadable pane: data | error text. */
function useAsync<T>(load: () => Promise<T>, deps: unknown[]): {
  data: T | null;
  error: string | null;
  reload: () => void;
} {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let alive = true;
    load()
      .then((d) => {
        if (alive) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => {
        if (alive) setError(String(e));
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);
  return { data, error, reload: () => setTick((t) => t + 1) };
}

function SecuritySection() {
  const [minutes, setMinutes] = useState("15");
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void api
      .settingsGet(AUTO_LOCK_KEY)
      .then((v) => setMinutes(v ?? "15"))
      .catch((e) => setError(String(e)));
  }, []);

  const save = async (value: string) => {
    setMinutes(value);
    const n = Math.max(0, Math.min(1440, Math.floor(Number(value) || 0)));
    try {
      await api.settingsSet(AUTO_LOCK_KEY, String(n));
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div class="settings-grid">
      <h4>Security</h4>
      <label class="settings-row">
        Auto-lock after (minutes, 0 = never)
        <input
          type="number"
          min={0}
          max={1440}
          value={minutes}
          onChange={(e) => void save((e.target as HTMLInputElement).value)}
        />
      </label>
      {saved && <div class="settings-ok">Auto-lock preference saved.</div>}
      {error && <div class="settings-error">{error}</div>}
    </div>
  );
}

function BackupSection() {
  const list = useAsync<BackupInfo[]>(() => api.backupList(), []);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<BackupInfo | null>(null);

  const backup = async () => {
    try {
      setNote(`Saved: ${await api.backupNow()}`);
      setError(null);
      list.reload();
    } catch (e) {
      setError(`Backup failed: ${e}`);
    }
  };

  const restore = async (b: BackupInfo) => {
    setConfirming(null);
    try {
      await api.backupRestore(b.name);
      setNote(`Restored ${b.name}.`);
      setError(null);
    } catch (e) {
      setError(`Restore failed: ${e}`);
    }
  };

  return (
    <div class="settings-grid">
      <h4>Backup</h4>
      <div class="settings-actions">
        <button class="primary" onClick={() => void backup()}>
          Back up database now
        </button>
        <button onClick={list.reload}>Refresh list</button>
      </div>
      {list.data && list.data.length > 0 && (
        <table class="settings-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Size</th>
              <th>Created</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {list.data.map((b) => (
              <tr key={b.name}>
                <td>{b.name}</td>
                <td>{fmtBytes(b.size)}</td>
                <td>{fmtWhen(b.created)}</td>
                <td>
                  <button onClick={() => setConfirming(b)}>Restore</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {list.data && list.data.length === 0 && (
        <div class="settings-note">No backups yet.</div>
      )}
      {note && <div class="settings-ok">{note}</div>}
      {(error ?? list.error) && <div class="settings-error">{error ?? list.error}</div>}

      <FormDialog
        open={confirming !== null}
        title="Restore backup"
        onClose={() => setConfirming(null)}
        onOk={() => void (confirming && restore(confirming))}
        okLabel="Restore"
      >
        <p>
          Replace the current database with <b>{confirming?.name}</b>? Unsaved
          changes will be lost.
        </p>
      </FormDialog>
    </div>
  );
}

function DevicesSection() {
  const status = useAsync(() => api.wifiStatus(), []);
  const code = useAsync(() => api.wifiPairCode(), []);
  const devices = useAsync<DeviceInfo[]>(() => api.devicesList(), []);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<SyncReport | null>(null);
  const [host, setHost] = useState("");
  const [port, setPort] = useState("5600");
  const [pairCode, setPairCode] = useState("");
  const [deviceName, setDeviceName] = useState("");

  const fail = (e: unknown) => {
    setError(String(e));
    setNote(null);
  };

  const toggleAdvertise = async () => {
    try {
      if (status.data?.advertising) {
        await api.wifiAdvertiseStop();
      } else {
        const r = await api.wifiAdvertiseStart();
        setNote(`Advertising on port ${r.port}.`);
      }
      status.reload();
      code.reload();
    } catch (e) {
      fail(e);
    }
  };

  const pair = async () => {
    try {
      const d = await api.devicePair(host.trim(), Number(port) || 0, pairCode.trim(), deviceName.trim() || host.trim());
      setNote(`Paired with ${d.name}.`);
      setError(null);
      devices.reload();
    } catch (e) {
      fail(e);
    }
  };

  const sync = async (d: DeviceInfo) => {
    try {
      const r = await api.deviceSync(d.id);
      setReport(r);
      setError(null);
      devices.reload();
    } catch (e) {
      fail(e);
    }
  };

  return (
    <div class="settings-grid">
      <h4>Devices (Wi-Fi sync)</h4>
      <div class="settings-actions">
        <button onClick={() => void toggleAdvertise()}>
          {status.data?.advertising ? "Stop advertising" : "Advertise on LAN"}
        </button>
        {status.data?.port && <span class="settings-note">port {status.data.port}</span>}
        {code.data?.code && <span class="settings-note">pair code: {code.data.code}</span>}
      </div>
      <table class="settings-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Platform</th>
            <th>Last sync</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {(devices.data ?? []).map((d) => (
            <tr key={d.id}>
              <td>{d.name}</td>
              <td>{d.platform}</td>
              <td>{fmtWhen(d.last_sync)}</td>
              <td>
                <span class="settings-actions">
                  <button onClick={() => void sync(d)}>Sync now</button>
                  <button
                    class="danger-text"
                    onClick={() => void api.deviceRemove(d.id).then(devices.reload).catch(fail)}
                  >
                    Remove
                  </button>
                </span>
              </td>
            </tr>
          ))}
          {devices.data && devices.data.length === 0 && (
            <tr>
              <td colSpan={4} class="settings-note">
                No paired devices.
              </td>
            </tr>
          )}
        </tbody>
      </table>
      <div class="settings-actions">
        <input
          placeholder="Host"
          style={{ width: 150 }}
          value={host}
          onInput={(e) => setHost((e.target as HTMLInputElement).value)}
        />
        <input
          placeholder="Port"
          style={{ width: 70 }}
          value={port}
          onInput={(e) => setPort((e.target as HTMLInputElement).value)}
        />
        <input
          placeholder="Pair code"
          style={{ width: 90 }}
          value={pairCode}
          onInput={(e) => setPairCode((e.target as HTMLInputElement).value)}
        />
        <input
          placeholder="Device name"
          style={{ width: 130 }}
          value={deviceName}
          onInput={(e) => setDeviceName((e.target as HTMLInputElement).value)}
        />
        <button onClick={() => void pair()}>Connect manually</button>
      </div>
      {report && (
        <div class="settings-ok">
          {report.device}: pushed {report.pushed}, pulled {report.pulled},{" "}
          {report.conflicts} conflict(s).
        </div>
      )}
      {note && <div class="settings-ok">{note}</div>}
      {(error ?? status.error ?? code.error ?? devices.error) && (
        <div class="settings-error">{error ?? status.error ?? code.error ?? devices.error}</div>
      )}
    </div>
  );
}

function CloudSection() {
  const profiles = useAsync<DavProfile[]>(() => api.davProfiles(), []);
  const [kind, setKind] = useState<"caldav" | "carddav" | "webdav">("caldav");
  const [url, setUrl] = useState("");
  const [user, setUser] = useState("");
  const [password, setPassword] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<DavReport | null>(null);

  const fail = (e: unknown) => {
    setError(String(e));
    setNote(null);
  };

  const save = async () => {
    if (!url.trim()) {
      fail("A server URL is required.");
      return;
    }
    try {
      await api.davSave({ id: null, kind, url: url.trim(), user: user.trim() || null, password: password || null });
      setNote("Profile saved.");
      setError(null);
      setPassword("");
      profiles.reload();
    } catch (e) {
      fail(e);
    }
  };

  const syncProfile = async (p: DavProfile) => {
    try {
      const r = await api.davSync(p.id);
      setReport(r);
      setError(null);
      profiles.reload();
    } catch (e) {
      fail(e);
    }
  };

  return (
    <div class="settings-grid">
      <h4>Cloud (CalDAV / CardDAV / WebDAV)</h4>
      <table class="settings-table">
        <thead>
          <tr>
            <th>Kind</th>
            <th>URL</th>
            <th>User</th>
            <th>Last sync</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {(profiles.data ?? []).map((p) => (
            <tr key={p.id}>
              <td>{p.kind}</td>
              <td class="settings-note">{p.url}</td>
              <td>{p.user ?? "—"}</td>
              <td>{fmtWhen(p.last_sync)}</td>
              <td>
                <span class="settings-actions">
                  <button onClick={() => void syncProfile(p)}>Sync now</button>
                  <button
                    class="danger-text"
                    onClick={() => void api.davRemove(p.id).then(profiles.reload).catch(fail)}
                  >
                    Remove
                  </button>
                </span>
              </td>
            </tr>
          ))}
          {profiles.data && profiles.data.length === 0 && (
            <tr>
              <td colSpan={5} class="settings-note">
                No cloud accounts.
              </td>
            </tr>
          )}
        </tbody>
      </table>
      <div class="settings-actions">
        <select value={kind} onChange={(e) => setKind((e.target as HTMLSelectElement).value as typeof kind)}>
          <option value="caldav">CalDAV</option>
          <option value="carddav">CardDAV</option>
          <option value="webdav">WebDAV</option>
        </select>
        <input
          placeholder="https://server/path"
          style={{ width: 200 }}
          value={url}
          onInput={(e) => setUrl((e.target as HTMLInputElement).value)}
        />
        <input
          placeholder="User"
          style={{ width: 110 }}
          value={user}
          onInput={(e) => setUser((e.target as HTMLInputElement).value)}
        />
        <input
          type="password"
          placeholder="Password"
          style={{ width: 110 }}
          value={password}
          onInput={(e) => setPassword((e.target as HTMLInputElement).value)}
        />
        <button onClick={() => void save()}>Add profile</button>
      </div>
      {report && (
        <div class="settings-ok">
          Synced: pulled {report.pulled}, pushed {report.pushed}
          {report.errors.length > 0 ? `, ${report.errors.length} error(s)` : ""}.
        </div>
      )}
      {note && <div class="settings-ok">{note}</div>}
      {(error ?? profiles.error) && <div class="settings-error">{error ?? profiles.error}</div>}
    </div>
  );
}

function AiSettingsSection() {
  const status = aiStatus.value;
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [endpoint, setEndpoint] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [reindexJob, setReindexJob] = useState<number | null>(null);
  const [reindexNote, setReindexNote] = useState<string | null>(null);

  const load = () => {
    void refreshAiStatus();
    void api
      .aiSettingsGet()
      .then((s) => {
        setSettings(s);
        setEndpoint(s.endpoint);
        setModel(s.model);
        setApiKey("");
      })
      .catch((e) => setError(errText(e)));
  };

  useEffect(load, []);

  // While a reindex job runs, follow its progress via ai://job and mirror
  // the embedded/embeddable counts from refreshed ai_status.
  useEffect(() => {
    if (reindexJob === null) return;
    setReindexNote("Reindexing embeddings…");
    const tick = () => void refreshAiStatus();
    const poller = window.setInterval(tick, 2_000);
    const un = api
      .listen("ai://job", (ev) => {
        const p = ev.payload as { id: number; state: string; error?: string | null } | undefined;
        if (!p || p.id !== reindexJob) return;
        if (p.state === "done" || p.state === "failed" || p.state === "cancelled") {
          window.clearInterval(poller);
          setReindexJob(null);
          if (p.state === "done") {
            setReindexNote("Embeddings rebuilt.");
            setNote(null);
          } else {
            setReindexNote(`Reindex ${p.state}${p.error ? `: ${p.error}` : ""}.`);
          }
          void refreshAiStatus();
        }
      })
      .catch(() => undefined);
    return () => {
      window.clearInterval(poller);
      void un.then((f) => f?.()).catch(() => undefined);
    };
  }, [reindexJob]);

  const apply = async (patch: Parameters<typeof api.aiSettingsSet>[0], ok: string) => {
    setError(null);
    setNote(null);
    try {
      await api.aiSettingsSet(patch);
      setNote(ok);
      void refreshAiStatus();
    } catch (e) {
      setError(errText(e));
    }
  };

  const reindex = async () => {
    setError(null);
    setReindexNote(null);
    try {
      const jobId = await api.aiReindex();
      setReindexJob(jobId);
    } catch (e) {
      setError(errText(e));
    }
  };

  const features = settings?.features ?? {};
  const reindexing = reindexJob !== null;

  return (
    <div class="settings-grid">
      <h4>AI (local-first, opt-in)</h4>
      <div class="ai-status-line">
        <span>
          provider <b>{status?.provider ?? settings?.provider ?? "—"}</b>
        </span>
        <span>
          endpoint <b>{status?.endpoint ?? settings?.endpoint ?? "—"}</b>
        </span>
        <span>
          model <b>{status?.model ?? settings?.model ?? "—"}</b>
        </span>
        <span class="ai-reachable" data-ok={status?.reachable ?? false}>
          {status === null
            ? "status unavailable"
            : status.reachable
              ? "reachable ✓"
              : "unreachable ✗"}
        </span>
        <span class="ai-note">
          {status ? `${status.embedded}/${status.embeddable} embedded` : ""}
        </span>
        <button onClick={load}>Refresh</button>
      </div>
      {status && status.error && <div class="settings-note">{status.error}</div>}

      <label class="settings-row">
        Provider
        <select
          value={settings?.provider ?? "local"}
          onChange={(e) => {
            const provider = (e.target as HTMLSelectElement).value as "local" | "cloud";
            void apply(
              { provider, cloud_enabled: provider === "cloud" ? true : undefined },
              `Provider set to ${provider}.`,
            );
          }}
        >
          <option value="local">Local (Ollama / OpenAI-compatible)</option>
          <option value="cloud">Cloud (BYO API key)</option>
        </select>
      </label>
      {settings?.provider === "cloud" && (
        <>
          <div class="ai-warn">
            Opt-in: with a cloud provider the content you run AI on (notes, mail,
            diary — never passwords) leaves this machine.
          </div>
          <label class="settings-row">
            API key {settings.api_key_set && <span class="ai-note">(a key is stored)</span>}
            <input
              type="password"
              placeholder={settings.api_key_set ? "Replace stored key" : "sk-…"}
              value={apiKey}
              onInput={(e) => setApiKey((e.target as HTMLInputElement).value)}
            />
          </label>
          <div class="settings-actions">
            <button
              disabled={!apiKey.trim()}
              onClick={() =>
                void apply({ api_key: apiKey.trim() }, "API key saved (encrypted at rest).").then(
                  () => setApiKey(""),
                )
              }
            >
              Save API key
            </button>
          </div>
        </>
      )}

      <label class="settings-row">
        Endpoint
        <input
          value={endpoint}
          placeholder="http://127.0.0.1:11434"
          onChange={(e) => setEndpoint((e.target as HTMLInputElement).value)}
          onBlur={() => {
            if (endpoint !== (settings?.endpoint ?? "")) {
              void apply({ endpoint }, "Endpoint saved.");
            }
          }}
        />
      </label>
      <label class="settings-row">
        Model
        <input
          value={model}
          list="ai-models"
          placeholder="llama3"
          onChange={(e) => setModel((e.target as HTMLInputElement).value)}
          onBlur={() => {
            if (model !== (settings?.model ?? "")) {
              void apply({ model }, "Model saved.");
            }
          }}
        />
      </label>
      <datalist id="ai-models">
        {(status?.models ?? []).map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>

      <h4>Features (each one is a separate opt-in)</h4>
      <div class="ai-feature-grid">
        {AI_FEATURES.map((name) => (
          <label key={name}>
            <input
              type="checkbox"
              checked={Boolean(features[name])}
              onChange={(e) => {
                const on = (e.target as HTMLInputElement).checked;
                void apply(
                  { features: { ...features, [name]: on } },
                  `${FEATURE_LABELS[name] ?? name} ${on ? "enabled" : "disabled"}.`,
                );
              }}
            />
            {FEATURE_LABELS[name] ?? name}
          </label>
        ))}
      </div>

      <h4>Embeddings</h4>
      <div class="settings-actions">
        <button disabled={reindexing} onClick={() => void reindex()}>
          {reindexing ? "Rebuilding…" : "Rebuild embeddings"}
        </button>
        {status && (
          <span class="ai-note">
            {status.embedded} of {status.embeddable} items embedded
          </span>
        )}
      </div>
      {reindexNote && <div class="settings-ok ai-reindex-note">{reindexNote}</div>}

      <AiUsageSection />
      {note && <div class="settings-ok">{note}</div>}
      {error && <div class="settings-error">{error}</div>}
    </div>
  );
}

/** Token/cost accounting (plan §4): cumulative counters, per-kind breakdown
 * and an estimated spend once per-1k rates are set. Counters only — the
 * section never sees any content that was sent to a model. */
function AiUsageSection() {
  const [usage, setUsage] = useState<AiUsage | null>(null);
  const [costIn, setCostIn] = useState("");
  const [costOut, setCostOut] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    void api
      .aiUsage()
      .then((u) => {
        setUsage(u);
        setCostIn(u.cost_per_1k_in === null ? "" : String(u.cost_per_1k_in));
        setCostOut(u.cost_per_1k_out === null ? "" : String(u.cost_per_1k_out));
      })
      .catch(() => setUsage(null));
  };
  useEffect(load, []);

  const fmt = (n: number) => n.toLocaleString();
  const saveRates = async () => {
    const num = (s: string) => {
      const t = s.trim();
      return t === "" ? null : Number(t);
    };
    setError(null);
    try {
      await api.aiSettingsSet({ cost_per_1k_in: num(costIn), cost_per_1k_out: num(costOut) });
      load();
    } catch (e) {
      setError(errText(e));
    }
  };

  return (
    <>
      <h4>Usage &amp; cost (tokens this database has spent)</h4>
      <div class="ai-status-line">
        <span>
          tokens in <b>{usage ? fmt(usage.tokens_in) : "—"}</b>
        </span>
        <span>
          tokens out <b>{usage ? fmt(usage.tokens_out) : "—"}</b>
        </span>
        <span>
          jobs <b>{usage ? fmt(usage.jobs) : "—"}</b>
        </span>
        <span>
          est. cost{" "}
          <b>
            {usage?.estimated_cost !== null && usage !== null
              ? `$${usage.estimated_cost.toFixed(4)}`
              : "set rates below"}
          </b>
        </span>
        <button onClick={load}>Refresh</button>
        <button
          disabled={usage === null || (usage.tokens_in === 0 && usage.tokens_out === 0)}
          onClick={() => void api.aiUsageReset().then(load)}
        >
          Reset counters
        </button>
      </div>
      {usage !== null && Object.keys(usage.by_kind).length > 0 && (
        <div class="ai-feature-grid">
          {Object.entries(usage.by_kind).map(([kind, k]) => (
            <span key={kind} title={`${k.jobs} jobs`}>
              {kind}: {fmt(k.tokens_in)}↓ / {fmt(k.tokens_out)}↑
            </span>
          ))}
        </div>
      )}
      <div class="settings-actions">
        <input
          style="width: 9em"
          type="number"
          min={0}
          step="0.0001"
          title="Cost per 1,000 input tokens (e.g. a cloud model's price)"
          placeholder="$/1k tokens in"
          value={costIn}
          onInput={(e) => setCostIn((e.target as HTMLInputElement).value)}
        />
        <input
          style="width: 9em"
          type="number"
          min={0}
          step="0.0001"
          title="Cost per 1,000 output tokens"
          placeholder="$/1k tokens out"
          value={costOut}
          onInput={(e) => setCostOut((e.target as HTMLInputElement).value)}
        />
        <button onClick={() => void saveRates()}>Save rates</button>
        <span class="ai-note">Leave blank for local models — no spend to estimate.</span>
      </div>
      {error && <div class="settings-error">{error}</div>}
    </>
  );
}

export function Settings() {
  // Hooks run regardless of visibility so the dialog can stay mounted.
  const [section, setSection] = useState<Section>("appearance");
  if (!settingsOpen.value) return null;

  return (
    <div class="overlay" onClick={() => (settingsOpen.value = false)}>
      <div
        class="dialog settings-dialog"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="Settings"
      >
        <h3>Settings</h3>
        <div class="settings-body">
          <nav class="settings-nav" aria-label="Settings sections">
            {SECTIONS.map((s) => (
              <button
                key={s.id}
                data-active={section === s.id}
                onClick={() => setSection(s.id)}
              >
                {s.label}
              </button>
            ))}
          </nav>
          <div class="settings-pane">
            {section === "appearance" && (
              <div class="settings-grid">
                <h4>Appearance</h4>
                <label class="settings-row">
                  Theme
                  <select
                    value={theme.value}
                    onChange={(e) => applyTheme((e.target as HTMLSelectElement).value as Theme)}
                  >
                    <option value="light">Light</option>
                    <option value="grey">Grey</option>
                    <option value="dark">Dark</option>
                    <option value="system">Match system</option>
                  </select>
                </label>
              </div>
            )}
            {section === "security" && <SecuritySection />}
            {section === "backup" && <BackupSection />}
            {section === "devices" && <DevicesSection />}
            {section === "cloud" && <CloudSection />}
            {section === "ai" && <AiSettingsSection />}
          </div>
        </div>
        <div class="dialog-actions">
          <button onClick={() => (settingsOpen.value = false)}>Close</button>
        </div>
      </div>
    </div>
  );
}
