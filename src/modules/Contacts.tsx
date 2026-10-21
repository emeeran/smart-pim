/**
 * Contacts module (S46–S49 UI slice): letter bar, list + detail card,
 * editor, VCF import/export. Gate 4: vertical A–Z rail on the right edge
 * and colored initials avatars. Email history + groups arrive with
 * T3.36–T3.37.
 */
import { useEffect, useMemo, useState } from "preact/hooks";
import "../design/panels.css";
import { api, type ContactRecord } from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { DetailTabs } from "../components/DetailTabs";
import { RelatedPanel } from "../components/RelatedPanel";
import { VirtualList } from "../components/VirtualList";
import { DupeDialog } from "../components/DupeDialog";
import { availableLetters, hueIndex, initials, letterOf } from "../lib/grouping";
import { composeTo } from "./Mail";
import { jumpTarget, jumpTo, refreshCounts, viewCount } from "../store";

/** A–Z (plus #) shown on the rail; only letters with contacts are enabled. */
const RAIL_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ#".split("");

const EMPTY: ContactRecord = {
  uid: null,
  id: 0,
  item_id: 0,
  first_name: "",
  last_name: "",
  display_name: "",
  organization: null,
  emails: [],
  phones: [],
  addresses: [],
  urls: [],
  birthday: null,
  notes: null,
  categories: [],
};

function ContactEditor({
  contact,
  onClose,
  onSaved,
}: {
  contact: ContactRecord;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [form, setForm] = useState(contact);
  const set = (p: Partial<ContactRecord>) => setForm((f) => ({ ...f, ...p }));

  return (
    <div class="overlay" onClick={onClose}>
      <div class="dialog" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Contact">
        <h3>{form.id === 0 ? "New Contact" : "Edit Contact"}</h3>
        <label class="settings-row">First name <input value={form.first_name} onInput={(e) => set({ first_name: (e.target as HTMLInputElement).value })} /></label>
        <label class="settings-row">Last name <input value={form.last_name} onInput={(e) => set({ last_name: (e.target as HTMLInputElement).value })} /></label>
        <label class="settings-row">Organization <input value={form.organization ?? ""} onInput={(e) => set({ organization: (e.target as HTMLInputElement).value || null })} /></label>
        <label class="settings-row">Emails (comma-separated) <input value={form.emails.join(", ")} onInput={(e) => set({ emails: (e.target as HTMLInputElement).value.split(/[,;\s]+/).filter(Boolean) })} /></label>
        <label class="settings-row">Phones (comma-separated) <input value={form.phones.join(", ")} onInput={(e) => set({ phones: (e.target as HTMLInputElement).value.split(/[,;\s]+/).filter(Boolean) })} /></label>
        <label class="settings-row">Birthday (YYYY-MM-DD) <input value={form.birthday ?? ""} onInput={(e) => set({ birthday: (e.target as HTMLInputElement).value || null })} /></label>
        <div class="dialog-actions">
          <button onClick={onClose}>Cancel</button>
          <button
            class="primary"
            onClick={async () => {
              if (!form.display_name.trim()) {
                set({ display_name: `${form.first_name} ${form.last_name}`.trim() });
              }
              await api.contactSave({
                ...form,
                display_name:
                  form.display_name.trim() || `${form.first_name} ${form.last_name}`.trim(),
              });
              onSaved();
              onClose();
            }}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

export function ContactsModule() {
  const [contacts, setContacts] = useState<ContactRecord[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [editor, setEditor] = useState<ContactRecord | null>(null);
  const [letter, setLetter] = useState("");
  const [dupes, setDupes] = useState(false);

  const reload = async () => {
    try {
      setContacts(await api.contactsList());
    } catch {
      /* locked */
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  // Related Items / Anniversaries click-through: select the jumped contact.
  useEffect(() => {
    const jt = jumpTarget.value;
    if (jt?.kind === "contact") {
      setSelected(jt.id);
      setLetter("");
      jumpTarget.value = null;
    }
  }, [jumpTarget.value]);

  useEffect(() => {
    viewCount.value = contacts.length;
  }, [contacts.length]);

  const present = useMemo(
    () => new Set(availableLetters(contacts.map((c) => c.display_name))),
    [contacts],
  );

  const visible = letter
    ? contacts.filter((c) => letterOf(c.display_name) === letter)
    : contacts;
  const current = contacts.find((c) => c.id === selected) ?? null;

  useToolbar(
    <div class="mail-toolbar">
      <button class="primary" onClick={() => setEditor({ ...EMPTY, display_name: "" })}>
        New Contact
      </button>
      <button
        title="Find likely duplicate contacts (preview before any merge)"
        onClick={() => setDupes(true)}
      >
        Find Duplicates
      </button>
    </div>,
    [],
  );

  return (
    <div class="contacts-module">
      <ThreePane
        module="contacts"
        tree={
          <nav class="smartlists" aria-label="Contacts">
            <button
              class="smartlist-entry"
              data-active={letter === ""}
              onClick={() => setLetter("")}
            >
              <span>All Contacts</span>
              <span class="smartlist-count">{contacts.length}</span>
            </button>
            {letter !== "" && (
              <button
                class="smartlist-entry"
                data-active={true}
                onClick={() => setLetter("")}
              >
                <span>Letter: {letter}</span>
                <span class="smartlist-count">{visible.length}</span>
              </button>
            )}
            <button class="smartlist-entry" onClick={() => setEditor({ ...EMPTY })}>+ New contact</button>
            <label class="smartlist-entry" style={{ cursor: "pointer" }}>
              Import VCF…
              <input
                type="file"
                accept=".vcf,text/vcard"
                style={{ display: "none" }}
                onChange={async (e) => {
                  const f = (e.target as HTMLInputElement).files?.[0];
                  if (!f) return;
                  const res = await api.contactsImportVcf(await f.text());
                  alert(`Imported ${res.imported} contact(s).`);
                  void reload();
                  void refreshCounts();
                }}
              />
            </label>
            <button
              class="smartlist-entry"
              onClick={async () => {
                const path = await api.contactsExportVcf();
                alert(`Exported to ${path}`);
              }}
            >
              Export all to VCF
            </button>
          </nav>
        }
        list={
          <div class="letter-rail-wrap">
            {visible.length === 0 ? (
              <p class="muted empty-pane">No contacts{letter ? ` under ${letter}` : ""}.</p>
            ) : (
              <VirtualList
                rowCount={visible.length}
                rowHeight={48}
                renderRow={(i) => {
                  const c = visible[i];
                  const name = c.display_name || "?";
                  return (
                    <div
                      key={c.id}
                      class="contact-row"
                      style={{ height: "48px" }}
                      data-selected={c.id === selected}
                      onClick={() => setSelected(c.id)}
                    >
                      <span class={`mail-avatar hue-${hueIndex(name)}`}>
                        {initials(name)}
                      </span>
                      <span class="contact-row-name">{c.display_name || "Unnamed"}</span>
                      <span class="muted contact-row-email">{c.emails[0] ?? ""}</span>
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
                  title={present.has(l) ? `Show ${l}` : `No contacts under ${l}`}
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
            <div class="muted empty-pane">Select a contact.</div>
          ) : (
            <DetailTabs
              itemId={current.item_id}
              related={<RelatedPanel kind="contact" id={current.id} />}
              notes={
                <div class="contact-detail">
                  <h3>{current.display_name}</h3>
              {current.organization && <div class="muted">{current.organization}</div>}
              <dl class="contact-fields">
                {current.emails.map((e) => (
                  <div key={e}><dt>email</dt><dd>{e}</dd></div>
                ))}
                {current.phones.map((p) => (
                  <div key={p}><dt>phone</dt><dd>{p}</dd></div>
                ))}
                {current.urls.map((u) => (
                  <div key={u}><dt>url</dt><dd>{u}</dd></div>
                ))}
                {current.birthday && (
                  <div><dt>birthday</dt><dd>{current.birthday}</dd></div>
                )}
                {current.notes && (
                  <div><dt>notes</dt><dd>{current.notes}</dd></div>
                )}
              </dl>
              <div class="mail-header-actions">
                <button
                  title="Compose a message to this contact"
                  disabled={current.emails.length === 0}
                  onClick={() => composeTo(current.emails[0] ?? "", "")}
                >
                  Send E-mail
                </button>
                <button
                  title="Plan a meeting — opens the calendar"
                  onClick={() => jumpTo("calendar", "contact", current.id)}
                >
                  Schedule Meeting
                </button>
                <button onClick={() => setEditor(current)}>Edit</button>
                <button
                  class="danger-text"
                  onClick={async () => {
                    if (!confirm("Move this contact to trash?")) return;
                    await api.contactDelete(current.id);
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
        <ContactEditor
          contact={editor}
          onClose={() => setEditor(null)}
          onSaved={() => void reload()}
        />
      )}
      <DupeDialog
        open={dupes}
        contacts={contacts}
        onClose={() => setDupes(false)}
        onMerged={() => {
          void reload();
          void refreshCounts();
        }}
      />
    </div>
  );
}
