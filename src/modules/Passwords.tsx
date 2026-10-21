/**
 * Passwords module (S50–S51 UI slice): masked entries, editor with generator,
 * strength indicator. Gate 4: vertical A–Z rail and a local letter-tile
 * favicon placeholder (no network fetch). Auto-Type is P1 (Wayland caveat,
 * A-4).
 */
import { useEffect, useMemo, useState } from "preact/hooks";
import "../design/panels.css";
import { api, type PwRow } from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { DetailTabs } from "../components/DetailTabs";
import { VirtualList } from "../components/VirtualList";
import { availableLetters, hueIndex, letterOf } from "../lib/grouping";
import { refreshCounts, viewCount } from "../store";

const ROW_H = 44;

/** A–Z (plus #) shown on the rail; only letters with entries are enabled. */
const RAIL_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ#".split("");

interface EditorPw {
  id: number | null;
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
}

const EMPTY: EditorPw = { id: null, title: "", username: "", password: "", url: "", notes: "" };

function strengthLabel(score: number | null): { label: string; cls: string } {
  const s = score ?? 0;
  if (s < 30) return { label: "weak", cls: "weak" };
  if (s < 60) return { label: "fair", cls: "fair" };
  if (s < 85) return { label: "good", cls: "good" };
  return { label: "strong", cls: "strong" };
}

export function PasswordsModule() {
  const [entries, setEntries] = useState<PwRow[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [revealed, setRevealed] = useState<number | null>(null);
  const [editor, setEditor] = useState<EditorPw | null>(null);
  const [letter, setLetter] = useState("");

  const reload = async () => {
    try {
      setEntries(await api.passwordsList());
    } catch {
      /* locked */
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  useEffect(() => {
    viewCount.value = entries.length;
  }, [entries.length]);

  const present = useMemo(
    () => new Set(availableLetters(entries.map((e) => e.title))),
    [entries],
  );

  const visible = letter
    ? entries.filter((e) => letterOf(e.title) === letter)
    : entries;
  const current = entries.find((e) => e.id === selected) ?? null;

  useToolbar(
    <div class="mail-toolbar">
      <button class="primary" onClick={() => setEditor({ ...EMPTY })}>
        New Password
      </button>
    </div>,
    [],
  );

  return (
    <div class="passwords-module">
      <ThreePane
        module="passwords"
        tree={
          <nav class="smartlists" aria-label="Passwords">
            <button class="smartlist-entry" onClick={() => setEditor({ ...EMPTY })}>
              + New entry
            </button>
            <button
              class="smartlist-entry"
              onClick={async () => {
                const pw = await api.passwordGenerate(20);
                setEditor({ ...EMPTY, password: pw });
              }}
            >
              + Generate & create
            </button>
          </nav>
        }
        list={
          <div class="letter-rail-wrap">
            {visible.length === 0 ? (
              <p class="muted empty-pane">
                No password entries{letter ? ` under ${letter}` : ""}.
              </p>
            ) : (
              <VirtualList
                rowCount={visible.length}
                rowHeight={ROW_H}
                renderRow={(i) => {
                  const p = visible[i];
                  const s = strengthLabel(p.strength);
                  // Favicon placeholder: letter tile from the site host when
                  // we have one, else the title (never fetched from the net).
                  let host = "";
                  if (p.url) {
                    try {
                      host = new URL(p.url).hostname.replace(/^www\./, "");
                    } catch {
                      host = "";
                    }
                  }
                  const tile = host || p.title || "?";
                  return (
                    <div
                      key={p.id}
                      class="pw-row"
                      style={{ height: `${ROW_H}px` }}
                      data-selected={p.id === selected}
                      onClick={() => setSelected(p.id)}
                    >
                      <span class={`letter-tile hue-${hueIndex(tile)}`} title={host || undefined}>
                        {tile.slice(0, 1).toUpperCase()}
                      </span>
                      <span class="pw-row-title">{p.title}</span>
                      <span class="muted">{p.username ?? ""}</span>
                      <span
                        class={`pw-strength ${s.cls}`}
                        title={`strength ${p.strength ?? "?"}/100`}
                      >
                        {s.label}
                      </span>
                    </div>
                  );
                }}
              />
            )}
            <div class="letter-rail" role="navigation" aria-label="Jump to letter">
              {RAIL_LETTERS.map((l) => (
                <button
                  key={l}
                  disabled={!present.has(l)}
                  class={letter === l ? "active" : ""}
                  title={present.has(l) ? `Show ${l}` : `No entries under ${l}`}
                  onClick={() => setLetter(letter === l ? "" : l)}
                >
                  {l}
                </button>
              ))}
            </div>
          </div>
        }
        detail={
          current === null ? (
            <div class="muted empty-pane">Select an entry.</div>
          ) : (
            <DetailTabs
              itemId={current.item_id}
              notes={
                <div class="contact-detail">
                  <h3>{current.title}</h3>
              <dl class="contact-fields">
                <div>
                  <dt>username</dt>
                  <dd>
                    {current.username ?? "—"}
                    {current.username && (
                      <button
                        title="Copy user name"
                        onClick={() => void navigator.clipboard?.writeText(current.username ?? "")}
                      >
                        Copy
                      </button>
                    )}
                  </dd>
                </div>
                <div>
                  <dt>password</dt>
                  <dd class="pw-masked">
                    {revealed === current.id ? (current.password ?? "—") : "••••••••••"}
                    <button
                      onClick={() => setRevealed(revealed === current.id ? null : current.id)}
                    >
                      {revealed === current.id ? "Hide" : "Show"}
                    </button>
                    <button
                      onClick={() => {
                        void navigator.clipboard?.writeText(current.password ?? "");
                      }}
                    >
                      Copy
                    </button>
                  </dd>
                </div>
                {current.url && <div><dt>url</dt><dd>{current.url}</dd></div>}
                {current.notes && <div><dt>notes</dt><dd>{current.notes}</dd></div>}
              </dl>
              <div class="mail-header-actions">
                <button
                  onClick={() =>
                    setEditor({
                      id: current.id,
                      title: current.title,
                      username: current.username ?? "",
                      password: current.password ?? "",
                      url: current.url ?? "",
                      notes: current.notes ?? "",
                    })
                  }
                >
                  Edit
                </button>
                <button
                  class="danger-text"
                  onClick={async () => {
                    if (!confirm("Move this entry to trash?")) return;
                    await api.passwordDelete(current.id);
                    setSelected(null);
                    void reload();
                    void refreshCounts();
                  }}
                >
                  Delete
                </button>
              </div>
                </div>
              }
            />
          )
        }
      />
      {editor && (
        <div class="overlay" onClick={() => setEditor(null)}>
          <div class="dialog" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Password entry">
            <h3>{editor.id === null ? "New Entry" : "Edit Entry"}</h3>
            <label class="settings-row">Title <input value={editor.title} onInput={(e) => setEditor({ ...editor, title: (e.target as HTMLInputElement).value })} /></label>
            <label class="settings-row">Username <input value={editor.username} onInput={(e) => setEditor({ ...editor, username: (e.target as HTMLInputElement).value })} /></label>
            <label class="settings-row">
              Password
              <input
                type="password"
                value={editor.password}
                onInput={(e) => setEditor({ ...editor, password: (e.target as HTMLInputElement).value })}
              />
            </label>
            <div class="settings-row">
              <button
                onClick={async () => {
                  const pw = await api.passwordGenerate(20);
                  setEditor({ ...editor, password: pw });
                }}
              >
                Generate (20 chars)
              </button>
            </div>
            <label class="settings-row">URL <input value={editor.url} onInput={(e) => setEditor({ ...editor, url: (e.target as HTMLInputElement).value })} /></label>
            <label class="settings-row">Notes <input value={editor.notes} onInput={(e) => setEditor({ ...editor, notes: (e.target as HTMLInputElement).value })} /></label>
            <div class="dialog-actions">
              <button onClick={() => setEditor(null)}>Cancel</button>
              <button
                class="primary"
                onClick={async () => {
                  if (!editor.title.trim()) return;
                  await api.passwordSave({
                    id: editor.id,
                    title: editor.title,
                    username: editor.username || null,
                    password: editor.password || null,
                    url: editor.url || null,
                    notes: editor.notes || null,
                  });
                  setEditor(null);
                  void reload();
                  void refreshCounts();
                }}
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
