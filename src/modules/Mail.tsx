/**
 * Mail module (S15–S21 UI slice): folder tree | message list | reading pane,
 * account wizard, compose with Send Later, remote-content blocking (S19).
 * Gate 4: EPIM-style date-grouped cards, initials avatars, account accent
 * bars and a pinned quick-reply box under the reading pane.
 */
import { useEffect, useRef, useState } from "preact/hooks";
import { signal } from "@preact/signals";
import "../design/panels.css";
import {
  api,
  type Account,
  type FolderNode,
  type GoogleStatus,
  type MessageDetail,
  type MessagePageRow,
} from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { VirtualList } from "../components/VirtualList";
import { AiPreview } from "../components/AiPreview";
import {
  errText,
  featureEnabled,
  parseReplyDraft,
  parseSignatureContact,
  parseSuggestedRules,
  resultText,
  rulePrefill,
  runAiJob,
  useAiStatus,
  type ParsedContact,
  type SuggestedRule,
} from "../lib/ai";
import { appendPage, nextCursor } from "../lib/list-model";
import { hueIndex, initials, withMailGroupHeaders } from "../lib/grouping";
import { openRules } from "./RulesDialog";
import { activeModule, jumpTarget, refreshCounts, viewCount } from "../store";

const ROW_H = 64;

/** Compose dialog state lives at module level so drafts survive closing. */
const compose = {
  open: false,
  to: "",
  cc: "",
  subject: "",
  body: "",
};

/** Cross-module hand-off (Contacts → "Send E-mail"): pre-fill a compose and
 * flip the exported signal; the Mail module opens the dialog on arrival. */
export const composeOpen = signal(false);

export function composeTo(address: string, subject = ""): void {
  compose.to = address;
  compose.cc = "";
  compose.subject = subject;
  compose.body = "";
  composeOpen.value = true;
  activeModule.value = "mail";
}

interface DigestState {
  open: boolean;
  body: string;
  busy: boolean;
  error: string | null;
}

interface AiReplyState {
  open: boolean;
  bullets: string;
  busy: boolean;
  error: string | null;
}

interface SuggestState {
  open: boolean;
  rows: SuggestedRule[];
  busy: boolean;
  applying: boolean;
  error: string | null;
  note: string | null;
}

const DIGEST_IDLE: DigestState = { open: false, body: "", busy: false, error: null };
const REPLY_IDLE: AiReplyState = { open: false, bullets: "", busy: false, error: null };

/** Signature → contact proposal (Contacts feature, ai_integ_plan §3). */
interface SigState {
  open: boolean;
  busy: boolean;
  error: string | null;
  contact: ParsedContact | null;
  /** Chosen merge target, or null = create a new contact. */
  mergeInto: number | null;
  applying: boolean;
  applied: string | null;
}
const SIG_IDLE: SigState = {
  open: false,
  busy: false,
  error: null,
  contact: null,
  mergeInto: null,
  applying: false,
  applied: null,
};
const SUGGEST_IDLE: SuggestState = {
  open: false,
  rows: [],
  busy: false,
  applying: false,
  error: null,
  note: null,
};

// ------------------------------------------------------------------- tree

function FolderTree({
  onPickFolder,
  onPickUnified,
  onAddAccount,
  selectedFolder,
  reloadKey,
}: {
  onPickFolder: (folder: FolderNode) => void;
  onPickUnified: () => void;
  onAddAccount: () => void;
  selectedFolder: number;
  reloadKey: number;
}) {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [folders, setFolders] = useState<Record<number, FolderNode[]>>({});
  const [open, setOpen] = useState<Record<number, boolean>>({});

  useEffect(() => {
    void (async () => {
      try {
        const accs = await api.mailAccounts();
        setAccounts(accs);
        const map: Record<number, FolderNode[]> = {};
        for (const a of accs) {
          map[a.id] = await api.mailFolders(a.id);
          setOpen((o) => (a.id in o ? o : { ...o, [a.id]: true }));
        }
        setFolders(map);
      } catch {
        /* locked */
      }
    })();
  }, [reloadKey]);

  return (
    <div class="folder-tree">
      <div class="tree-toolbar">
        <button class="primary" onClick={onAddAccount}>Add Account…</button>
      </div>
      <button
        class="tree-folder"
        data-selected={selectedFolder === -1}
        onClick={() => onPickUnified()}
      >
        <span class="tree-folder-name">Unified Inbox</span>
      </button>
      {accounts.map((a) => (
        <div key={a.id} class="tree-account">
          <button
            class="tree-account-row"
            onClick={() => setOpen((o) => ({ ...o, [a.id]: !o[a.id] }))}
          >
            <span class="tree-caret">{open[a.id] ? "▾" : "▸"}</span>
            <span class="tree-account-name">{a.name}</span>
            <span class="tree-account-kind">{a.kind}</span>
          </button>
          {open[a.id] &&
            (folders[a.id] ?? []).map((f) => (
              <button
                key={f.id}
                class="tree-folder"
                data-selected={f.id === selectedFolder}
                onClick={() => onPickFolder(f)}
              >
                <span class="tree-folder-name">{f.name}</span>
                {f.unread > 0 && <span class="tree-unread">{f.unread}</span>}
              </button>
            ))}
        </div>
      ))}
      <button
        class="tree-sync"
        title="Sync all accounts"
        onClick={async () => {
          for (const a of accounts) {
            try {
              await api.mailSync(a.id);
            } catch (e) {
              alert(`Sync failed for ${a.name}: ${e}`);
            }
          }
          setFolders({ ...folders });
        }}
      >
        Sync All
      </button>
    </div>
  );
}

// ------------------------------------------------------------ reading pane

/** Remote-content policy (S19): blocked per sender until allowed. */
function remoteAllowed(sender: string): boolean {
  try {
    const allow: string[] = JSON.parse(localStorage.getItem("pf.remote-ok") ?? "[]");
    return allow.includes(sender);
  } catch {
    return false;
  }
}

function allowRemote(sender: string): void {
  const allow: string[] = JSON.parse(localStorage.getItem("pf.remote-ok") ?? "[]");
  if (!allow.includes(sender)) allow.push(sender);
  localStorage.setItem("pf.remote-ok", JSON.stringify(allow));
}

/** Strip remote <img> src unless the sender is allowed (default block). */
function blockRemote(html: string, sender: string): string {
  if (remoteAllowed(sender)) return html;
  return html.replace(
    /<img([^>]*?)src="(https?:[^"]*)"([^>]*)>/gi,
    (_m, pre: string, _src: string, post: string) =>
      `<img${pre}data-remote-blocked="1" alt="[remote image blocked]"${post}>`,
  );
}

function ReadingPane({
  messageId,
  accounts,
  defaultAccountId,
  onCompose,
  onAiDraft,
  onChanged,
}: {
  messageId: number;
  accounts: Account[];
  defaultAccountId: number | null;
  onCompose: (kind: "reply" | "reply-all" | "forward", msg: MessageDetail) => void;
  /** AI reply draft → pre-fills the existing compose dialog. */
  onAiDraft: (draft: { to: string; cc: string; subject: string; body: string }) => void;
  onChanged: () => void;
}) {
  const [msg, setMsg] = useState<MessageDetail | null>(null);
  const [showHtml, setShowHtml] = useState(true);
  const [remoteVersion, setRemoteVersion] = useState(0);
  const [quick, setQuick] = useState("");
  const [identity, setIdentity] = useState<number | null>(defaultAccountId);
  const [sending, setSending] = useState(false);
  const status = useAiStatus();
  const canDigest = featureEnabled(status, "digest");
  const canDraft = featureEnabled(status, "reply");
  const canContact = featureEnabled(status, "contacts");
  const [digest, setDigest] = useState<DigestState>(DIGEST_IDLE);
  const [aiReply, setAiReply] = useState<AiReplyState>(REPLY_IDLE);
  const [sig, setSig] = useState<SigState>(SIG_IDLE);

  const runSignature = async () => {
    if (msg === null) return;
    setSig({ ...SIG_IDLE, open: true, busy: true });
    try {
      const contact = parseSignatureContact(
        await runAiJob("signature_contact", { message_id: msg.id }),
      );
      setSig({
        ...SIG_IDLE,
        open: true,
        contact,
        mergeInto: contact?.candidates[0]?.contact_id ?? null,
      });
    } catch (e) {
      setSig({ ...SIG_IDLE, open: true, error: errText(e) });
    }
  };

  const setSigContact = (patch: Partial<ParsedContact>) =>
    setSig((s) => (s.contact ? { ...s, contact: { ...s.contact, ...patch } } : s));

  /** Apply: merge into the chosen contact, or create a new one. */
  const applySignature = async () => {
    const c = sig.contact;
    if (c === null || sig.applying) return;
    setSig((s) => ({ ...s, applying: true, error: null }));
    try {
      let name = "contact";
      if (sig.mergeInto !== null) {
        const all = await api.contactsList();
        const existing = all.find((r) => r.id === sig.mergeInto);
        if (existing === undefined) throw new Error("the matching contact no longer exists");
        const uniq = (list: string[], have: string[]) =>
          Array.from(new Set([...have, ...list.map((x) => x.trim())].filter(Boolean)));
        await api.contactSave({
          ...existing,
          first_name: existing.first_name || c.first_name,
          last_name: existing.last_name || c.last_name,
          organization: existing.organization || c.organization || null,
          emails: uniq(c.emails, existing.emails),
          phones: uniq(c.phones, existing.phones),
          urls: uniq(c.urls, existing.urls),
        });
        name = existing.display_name || existing.first_name || "contact";
      } else {
        const display =
          [c.first_name, c.last_name].filter(Boolean).join(" ") || c.emails[0] || msg?.from_addr || "contact";
        await api.contactSave({
          uid: null,
          id: 0,
          item_id: 0,
          first_name: c.first_name,
          last_name: c.last_name,
          display_name: display,
          organization: c.organization || null,
          emails: c.emails,
          phones: c.phones,
          addresses: [],
          urls: c.urls,
          birthday: null,
          notes: null,
          categories: [],
        });
        name = display;
      }
      setSig({ ...SIG_IDLE, open: true, applied: `Saved to contacts: ${name}` });
    } catch (e) {
      setSig((s) => ({ ...s, applying: false, error: errText(e) }));
    }
  };

  const runDigest = async () => {
    if (msg?.thread_id === null || msg === null) return;
    setDigest({ open: true, body: "", busy: true, error: null });
    try {
      const text = resultText(await runAiJob("thread_digest", { thread_id: msg.thread_id }));
      setDigest({ open: true, body: text, busy: false, error: null });
    } catch (e) {
      setDigest({ open: true, body: "", busy: false, error: errText(e) });
    }
  };

  const runDraftReply = async () => {
    if (msg === null) return;
    setAiReply((r) => ({ ...r, busy: true, error: null }));
    try {
      const bullets = aiReply.bullets.trim();
      const draft = parseReplyDraft(
        await runAiJob("draft_reply", bullets ? { message_id: msg.id, bullets } : { message_id: msg.id }),
      );
      if (draft === null) {
        setAiReply((r) => ({ ...r, busy: false, error: "The model returned an empty draft." }));
        return;
      }
      setAiReply(REPLY_IDLE);
      onAiDraft({
        to: msg.from_addr,
        cc: "",
        subject: draft.subject || (/^re:/i.test(msg.subject) ? msg.subject : `Re: ${msg.subject}`),
        body: draft.body,
      });
    } catch (e) {
      setAiReply((r) => ({ ...r, busy: false, error: errText(e) }));
    }
  };

  useEffect(() => {
    if (!messageId) {
      setMsg(null);
      return;
    }
    void api.messageFull(messageId).then((m) => {
      setMsg(m);
      if (!m.seen) {
        void api.messageMarkRead(m.id, true).then(() => void refreshCounts());
      }
    });
  }, [messageId]);

  // Follow the folder's account when it changes; keep the user's pick otherwise.
  useEffect(() => {
    setIdentity((cur) => cur ?? defaultAccountId);
  }, [defaultAccountId]);

  if (!msg) {
    return <div class="muted empty-pane">Select a message to read it.</div>;
  }
  const html = showHtml && msg.body_html ? blockRemote(msg.body_html, msg.from_addr) : null;
  void remoteVersion;

  const sendQuick = async () => {
    const accountId = identity ?? accounts[0]?.id;
    const body = quick.trim();
    if (accountId === undefined) {
      alert("Add an account first — quick reply needs a sending identity.");
      return;
    }
    if (!body || sending) return;
    setSending(true);
    try {
      await api.mailQueueDraft({
        account_id: accountId,
        from_name: "",
        // Empty = filled from the account identity at send time (mailsync).
        from_email: "",
        to: [msg.from_addr],
        cc: [],
        bcc: [],
        subject: /^re:/i.test(msg.subject) ? msg.subject : `Re: ${msg.subject}`,
        text_body: body,
        html_body: "",
        send_at: null,
      });
      const res = await api.outboxProcess();
      if (res.failed > 0) alert(`${res.sent} sent, ${res.failed} failed (kept in outbox).`);
      setQuick("");
      onChanged();
    } catch (e) {
      alert(`Queue failed: ${e}`);
    } finally {
      setSending(false);
    }
  };

  return (
    <div class="reading-pane">
      <div class="reading-scroll">
        <div class="mail-headers">
          <h3>{msg.subject}</h3>
          <div class="mail-header-row">
            <b>{msg.from_name || msg.from_addr}</b>
            <span class="muted"> &lt;{msg.from_addr}&gt;</span>
            <span class="mail-header-date">{new Date(msg.date).toLocaleString()}</span>
          </div>
          <div class="mail-header-row muted">to {msg.to_addrs}</div>
          <div class="mail-header-actions">
            {msg.body_html && (
              <button onClick={() => setShowHtml((v) => !v)}>
                {showHtml ? "Plain text" : "HTML"}
              </button>
            )}
            {html !== null && /data-remote-blocked/.test(html) && (
              <button
                onClick={() => {
                  allowRemote(msg.from_addr);
                  setRemoteVersion((v) => v + 1);
                }}
              >
                Load remote images from {msg.from_addr}
              </button>
            )}
            <button onClick={() => onCompose("reply", msg)}>Reply</button>
            <button onClick={() => onCompose("reply-all", msg)}>Reply all</button>
            <button onClick={() => onCompose("forward", msg)}>Forward</button>
            {canDigest && (
              <button
                disabled={msg.thread_id === null}
                title={
                  msg.thread_id === null
                    ? "This message is not part of a thread"
                    : "Summarize this thread with AI"
                }
                onClick={() => void runDigest()}
              >
                AI Digest
              </button>
            )}
            {canDraft && (
              <button
                title="Draft a reply with AI (opens the compose box)"
                onClick={() => setAiReply({ ...REPLY_IDLE, open: true })}
              >
                AI Reply…
              </button>
            )}
            {canContact && (
              <button
                title="Extract the sender's contact details (preview before saving)"
                onClick={() => void runSignature()}
              >
                AI Contact…
              </button>
            )}
            <button
              onClick={async () => {
                await api.messageSetFlag(msg.id, "seen", false);
                onChanged();
              }}
            >
              Mark unread
            </button>
            <button
              onClick={async () => {
                const currentlyFlagged = /"flagged":1/.test(
                  JSON.stringify((msg as unknown as { flags?: string }).flags ?? ""),
                );
                await api.messageSetFlag(msg.id, "flagged", !currentlyFlagged);
                onChanged();
              }}
            >
              Flag / Unflag
            </button>
            <button onClick={() => void api.messageDelete(msg.id).then(onChanged)}>Delete</button>
            <button onClick={() => void api.messageConvert(msg.id, "task").then(onChanged)}>→ Task</button>
            <button onClick={() => void api.messageConvert(msg.id, "note").then(onChanged)}>→ Note</button>
            <button onClick={() => void api.messageConvert(msg.id, "event").then(onChanged)}>→ Event</button>
          </div>
        </div>
        {html !== null ? (
          <div class="mail-body-html" dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <pre class="mail-body-text">{msg.body_text}</pre>
        )}
      </div>
      <div class="quick-reply">
        <textarea
          placeholder={`Quick reply to ${msg.from_addr}… (Enter to send, Shift+Enter for newline)`}
          value={quick}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void sendQuick();
            }
          }}
          onInput={(e) => setQuick((e.target as HTMLTextAreaElement).value)}
        />
        <div class="quick-reply-row">
          <select
            title="Sending identity"
            value={identity ?? ""}
            onChange={(e) => setIdentity(Number((e.target as HTMLSelectElement).value))}
          >
            {accounts.length === 0 && <option value="">No accounts</option>}
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name} ({a.user ?? a.kind})
              </option>
            ))}
          </select>
          <span class="muted quick-reply-who">to {msg.from_addr}</span>
          <button class="primary" disabled={sending || !quick.trim()} onClick={() => void sendQuick()}>
            {sending ? "Sending…" : "Send"}
          </button>
        </div>
      </div>
      <AiPreview
        open={digest.open}
        title="Thread digest"
        onClose={() => setDigest(DIGEST_IDLE)}
        body={digest.body}
        busy={digest.busy}
        busyLabel="Summarizing thread…"
        error={digest.error}
        actions={[{ label: "Copy", onClick: () => void navigator.clipboard?.writeText(digest.body), disabled: !digest.body }]}
      />
      <AiPreview
        open={aiReply.open}
        title="Draft a reply with AI"
        onClose={() => setAiReply(REPLY_IDLE)}
        busy={aiReply.busy}
        busyLabel="Drafting…"
        error={aiReply.error}
        actions={[
          {
            label: "Draft reply",
            primary: true,
            disabled: aiReply.busy,
            onClick: () => void runDraftReply(),
          },
        ]}
      >
        <label class="settings-row">
          Points to include (optional, one per line)
          <textarea
            class="compose-body"
            rows={4}
            placeholder="e.g. confirm the meeting time; ask for the updated invoice"
            value={aiReply.bullets}
            onInput={(e) =>
              setAiReply((r) => ({
                ...r,
                bullets: (e.target as HTMLTextAreaElement).value,
              }))
            }
          />
        </label>
        <div class="ai-note">
          The draft opens in the compose box for editing — nothing is sent from here.
        </div>
      </AiPreview>
      <AiPreview
        open={sig.open}
        title="Contact from signature"
        onClose={() => setSig(SIG_IDLE)}
        busy={sig.busy}
        busyLabel="Reading the signature…"
        error={sig.error}
        actions={
          sig.applied
            ? [{ label: "Close", onClick: () => setSig(SIG_IDLE) }]
            : [
                {
                  label: sig.mergeInto !== null ? "Merge & save" : "Save contact",
                  primary: true,
                  disabled:
                    sig.busy ||
                    sig.applying ||
                    sig.contact === null ||
                    (!sig.contact.first_name &&
                      !sig.contact.last_name &&
                      !sig.contact.organization &&
                      !sig.contact.emails.length &&
                      !sig.contact.phones.length &&
                      !sig.contact.urls.length),
                  onClick: () => void applySignature(),
                },
              ]
        }
      >
        {sig.applied !== null ? (
          <p>{sig.applied}</p>
        ) : sig.contact !== null ? (
          <>
            <label class="settings-row">
              Merge target
              <select
                value={sig.mergeInto ?? ""}
                onChange={(e) => {
                  const v = (e.target as HTMLSelectElement).value;
                  setSig((s) => ({ ...s, mergeInto: v === "" ? null : Number(v) }));
                }}
              >
                <option value="">— create a new contact —</option>
                {sig.contact.candidates.map((c) => (
                  <option key={c.contact_id} value={c.contact_id}>
                    {c.display_name}
                    {c.organization ? ` (${c.organization})` : ""}
                  </option>
                ))}
              </select>
            </label>
            <div class="settings-grid">
              <label class="settings-row">
                First name
                <input
                  value={sig.contact.first_name}
                  onInput={(e) => setSigContact({ first_name: (e.target as HTMLInputElement).value })}
                />
              </label>
              <label class="settings-row">
                Last name
                <input
                  value={sig.contact.last_name}
                  onInput={(e) => setSigContact({ last_name: (e.target as HTMLInputElement).value })}
                />
              </label>
              <label class="settings-row">
                Organization
                <input
                  value={sig.contact.organization}
                  onInput={(e) => setSigContact({ organization: (e.target as HTMLInputElement).value })}
                />
              </label>
              <label class="settings-row">
                Emails (one per line)
                <textarea
                  class="compose-body"
                  rows={2}
                  value={sig.contact.emails.join("\n")}
                  onInput={(e) =>
                    setSigContact({
                      emails: (e.target as HTMLTextAreaElement).value.split("\n"),
                    })
                  }
                />
              </label>
              <label class="settings-row">
                Phones (one per line)
                <textarea
                  class="compose-body"
                  rows={2}
                  value={sig.contact.phones.join("\n")}
                  onInput={(e) =>
                    setSigContact({
                      phones: (e.target as HTMLTextAreaElement).value.split("\n"),
                    })
                  }
                />
              </label>
              <label class="settings-row">
                URLs (one per line)
                <textarea
                  class="compose-body"
                  rows={2}
                  value={sig.contact.urls.join("\n")}
                  onInput={(e) =>
                    setSigContact({
                      urls: (e.target as HTMLTextAreaElement).value.split("\n"),
                    })
                  }
                />
              </label>
            </div>
            <div class="ai-note">
              {sig.mergeInto !== null
                ? "Merging only fills empty fields and adds new addresses — existing values are kept."
                : "Everything is editable; nothing is saved until you apply."}
            </div>
          </>
        ) : (
          <p class="muted">No contact details found.</p>
        )}
      </AiPreview>
    </div>
  );
}

// ------------------------------------------------------- google link card

/** Connect / status / sync controls for the Google Calendar+Tasks link on
 *  the selected mail account. Sync results arrive via google:// events and
 *  surface in the shared sync note. */
function GoogleLinkCard({ accountId, note }: { accountId: number | null; note: (m: string) => void }) {
  const [status, setStatus] = useState<GoogleStatus | null>(null);

  const reload = () => {
    if (accountId === null) {
      setStatus(null);
      return;
    }
    void api.googleStatus(accountId).then(setStatus).catch(() => setStatus(null));
  };
  useEffect(reload, [accountId]);

  if (accountId === null) return null;
  const s = status;
  return (
    <div class="google-link">
      {s === null ? null : !s.configured ? (
        <span class="muted">
          Google Calendar &amp; Tasks: set the OAuth client ID/secret in Settings first.
        </span>
      ) : s.linked && s.status === "needs_reconnect" ? (
        <>
          <span class="muted">
            Google access was revoked for {s.google_email} — reconnect to resume syncing.
          </span>
          <button onClick={() => void api.googleConnect(accountId)}>Reconnect</button>
        </>
      ) : s.linked ? (
        <>
          <span class="muted">
            Google: {s.google_email}
            {s.last_sync ? ` · last sync ${new Date(s.last_sync).toLocaleTimeString()}` : ""}
            {s.last_error ? ` · ${s.last_error}` : ""}
          </span>
          <button onClick={() => void api.googleSync(accountId)}>Sync Google</button>
          <button
            class="link-button"
            title="Unlink Google Calendar & Tasks from this account"
            onClick={() => {
              if (confirm(`Disconnect Google (${s.google_email}) from this account?`)) {
                void api.googleDisconnect(accountId).then(reload);
              }
            }}
          >
            Disconnect
          </button>
        </>
      ) : (
        <>
          <span class="muted">Google Calendar &amp; Tasks: not linked.</span>
          <button
            onClick={() => {
              note("Google sign-in opened in your browser — finish it there.");
              void api.googleConnect(accountId);
            }}
          >
            Connect Google Calendar &amp; Tasks
          </button>
        </>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ compose

function ComposeDialog({
  account,
  onClose,
}: {
  account: { id: number; email: string } | null;
  onClose: () => void;
}) {
  const [to, setTo] = useState(compose.to);
  const [cc, setCc] = useState(compose.cc);
  const [subject, setSubject] = useState(compose.subject);
  const [body, setBody] = useState(compose.body);
  useEffect(() => {
    setTo(compose.to);
    setCc(compose.cc);
    setSubject(compose.subject);
    setBody(compose.body);
  }, []);

  const send = async (sendAt: string | null) => {
    if (account === null) {
      alert("Select a folder of an account first.");
      return;
    }
    try {
      await api.mailQueueDraft({
        account_id: account.id,
        from_name: "",
        // Empty = filled from the account identity at send time (mailsync).
        from_email: "",
        to: to.split(/[,;\s]+/).filter(Boolean),
        cc: cc.split(/[,;\s]+/).filter(Boolean),
        bcc: [],
        subject,
        text_body: body,
        html_body: "",
        send_at: sendAt,
      });
      if (sendAt === null) {
        const res = await api.outboxProcess();
        if (res.failed > 0) alert(`${res.sent} sent, ${res.failed} failed (kept in outbox).`);
      }
      onClose();
    } catch (e) {
      alert(`Queue failed: ${e}`);
    }
  };

  return (
    <div class="overlay" onClick={onClose}>
      <div class="dialog compose" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Compose">
        <h3>New Message</h3>
        <input
          class="compose-row"
          placeholder="To"
          value={to}
          onInput={(e) => setTo((e.target as HTMLInputElement).value)}
        />
        <input
          class="compose-row"
          placeholder="Cc"
          value={cc}
          onInput={(e) => setCc((e.target as HTMLInputElement).value)}
        />
        <input
          class="compose-row"
          placeholder="Subject"
          value={subject}
          onInput={(e) => setSubject((e.target as HTMLInputElement).value)}
        />
        <textarea
          class="compose-body"
          placeholder="Message (plain text — the shared rich-text editor lands with Notes)"
          value={body}
          onInput={(e) => setBody((e.target as HTMLTextAreaElement).value)}
        />
        <div class="dialog-actions">
          <label class="muted send-later">
            Send at
            <input
              type="datetime-local"
              onChange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                if (v) void send(new Date(v).toISOString());
              }}
            />
          </label>
          <button class="primary" onClick={() => void send(null)}>Send</button>
          <button onClick={onClose}>Discard</button>
        </div>
      </div>
    </div>
  );
}

// ----------------------------------------------------------------- wizard

/** Well-known provider IMAP/SMTP defaults (manual sync; Gmail and friends
 * need an app password unless OAuth lands). Name is editable afterwards. */
const PROVIDER_PRESETS: {
  /** Stable id — also the key for the app-password help link. */
  id: string;
  label: string;
  name: string;
  host: string;
  port: number;
  security: "none" | "starttls" | "tls";
  smtp_host: string;
  smtp_port: number;
}[] = [
  {
    id: "gmail",
    label: "Gmail",
    name: "Gmail",
    host: "imap.gmail.com",
    port: 993,
    security: "tls",
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
  },
  {
    id: "outlook",
    label: "Outlook / M365",
    name: "Outlook",
    host: "outlook.office365.com",
    port: 993,
    security: "tls",
    smtp_host: "smtp.office365.com",
    smtp_port: 587,
  },
  {
    id: "yahoo",
    label: "Yahoo",
    name: "Yahoo",
    host: "imap.mail.yahoo.com",
    port: 993,
    security: "tls",
    smtp_host: "smtp.mail.yahoo.com",
    smtp_port: 587,
  },
  {
    id: "icloud",
    label: "iCloud",
    name: "iCloud",
    host: "imap.mail.me.com",
    port: 993,
    security: "tls",
    smtp_host: "smtp.mail.me.com",
    smtp_port: 587,
  },
  {
    id: "fastmail",
    label: "Fastmail",
    name: "Fastmail",
    host: "imap.fastmail.com",
    port: 993,
    security: "tls",
    smtp_host: "smtp.fastmail.com",
    smtp_port: 465,
  },
  {
    id: "zoho",
    label: "Zoho",
    name: "Zoho",
    host: "imap.zoho.com",
    port: 993,
    security: "tls",
    smtp_host: "smtp.zoho.com",
    smtp_port: 587,
  },
];

function AccountWizard({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const [form, setForm] = useState({
    name: "",
    kind: "imap" as "imap" | "pop3",
    host: "",
    port: 993,
    security: "tls" as "none" | "starttls" | "tls",
    user: "",
    password: "",
    smtp_host: "",
    smtp_port: 587,
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const set = (patch: Partial<typeof form>) => setForm((f) => ({ ...f, ...patch }));

  const applyPreset = (p: (typeof PROVIDER_PRESETS)[number]) =>
    set({
      name: form.name.trim() || p.name,
      host: p.host,
      port: p.port,
      security: p.security,
      smtp_host: p.smtp_host,
      smtp_port: p.smtp_port,
    });

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      // Returns immediately; the initial sync runs in the background and
      // reports via mail://synced (the wizard would freeze until Gmail
      // finished otherwise).
      await api.mailAccountCreate({
        ...form,
        smtp_host: form.smtp_host || null,
        smtp_port: form.smtp_port || null,
      });
      onCreated();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const activePreset = PROVIDER_PRESETS.find((p) => p.host === form.host);

  const field = (
    label: string,
    key: "name" | "host" | "user" | "password" | "smtp_host",
    type = "text",
  ) => (
    <label class="settings-row">
      {label}
      <span class="field-inline">
        <input
          type={type}
          value={form[key]}
          onInput={(e) => set({ [key]: (e.target as HTMLInputElement).value })}
        />
        {key === "password" && activePreset && (
          <button
            class="link-button"
            title={`Open ${activePreset.label}'s app-password page in your browser`}
            onClick={() => void api.openProviderHelp(activePreset.id)}
          >
            Get an app password ↗
          </button>
        )}
      </span>
    </label>
  );

  return (
    <div class="overlay" onClick={onClose}>
      <div class="dialog account-wizard" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Add account">
        <h3>Add Account</h3>
        <div class="account-wizard-body">
          <div class="wizard-presets">
            {PROVIDER_PRESETS.map((p) => (
              <button
                key={p.label}
                data-active={form.host === p.host}
                title={`imap.${p.host} → fill the server fields`}
                onClick={() => applyPreset(p)}
              >
                {p.label}
              </button>
            ))}
          </div>
          <div class="form-subtle">
            Presets fill the server fields (IMAP 993/TLS). Big providers need an
            app password — regular passwords are rejected there.
          </div>
          {field("Name", "name")}
          <label class="settings-row">
            Type
            <select
              value={form.kind}
              onChange={(e) => set({ kind: (e.target as HTMLSelectElement).value as "imap" | "pop3" })}
            >
              <option value="imap">IMAP</option>
              <option value="pop3">POP3</option>
            </select>
          </label>
          {field("Host", "host")}
          <label class="settings-row">
            Port
            <input
              type="number"
              value={form.port}
              onInput={(e) => set({ port: Number((e.target as HTMLInputElement).value) })}
            />
          </label>
          <label class="settings-row">
            Security
            <select
              value={form.security}
              onChange={(e) =>
                set({ security: (e.target as HTMLSelectElement).value as "none" | "starttls" | "tls" })
              }
            >
              <option value="tls">TLS</option>
              <option value="starttls">STARTTLS</option>
              <option value="none">None</option>
            </select>
          </label>
          {field("User", "user")}
          {field("Password", "password", "password")}
          {field("SMTP host", "smtp_host")}
          <label class="settings-row">
            SMTP port
            <input
              type="number"
              value={form.smtp_port}
              onInput={(e) => set({ smtp_port: Number((e.target as HTMLInputElement).value) })}
            />
          </label>
          {error && <div class="lock-error">{error}</div>}
        </div>
        <div class="dialog-actions">
          <button onClick={onClose}>Cancel</button>
          <button
            class="primary"
            disabled={busy || !form.host || !form.user}
            onClick={() => void create()}
          >
            {busy ? "Testing & syncing…" : "Create & Sync"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ module

function MailRow({
  m,
  selected,
  unread,
  flagged,
  indent,
  accent,
  threadCount,
  onToggleThread,
  onPick,
}: {
  m: MessagePageRow;
  selected: boolean;
  unread?: boolean;
  flagged?: boolean;
  indent?: boolean;
  /** Account color for the left accent bar (unavailable in unified view). */
  accent?: string | null;
  threadCount?: number;
  onToggleThread?: () => void;
  onPick: () => void;
}) {
  const showUnread = unread ?? !/"seen":\s*true/.test(m.flags);
  const sender = m.from_name || m.from_addr || "?";
  return (
    <div
      class="mail-card"
      data-selected={selected}
      data-unread={showUnread}
      data-indent={indent}
      data-accented={Boolean(accent)}
      style={{
        height: `${ROW_H}px`,
        ...(accent ? { "--card-accent": accent } : {}),
      }}
      onClick={onPick}
    >
      {threadCount ? (
        <button
          class="thread-toggle inline"
          title="Expand thread"
          onClick={(e) => {
            e.stopPropagation();
            onToggleThread?.();
          }}
        >
          ▾ {threadCount}
        </button>
      ) : (
        <span class={`mail-avatar hue-${hueIndex(sender)}`} aria-hidden="true">
          {initials(sender)}
        </span>
      )}
      <span class="mail-card-main">
        <span class="mail-card-top">
          <span class="mail-from">
            {flagged && <span class="mail-flag" title="Flagged">⚑</span>}
            {m.from_name || m.from_addr}
          </span>
          <span class="mail-time">
            {new Date(m.date).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
          </span>
        </span>
        <span class="mail-subject">{m.subject}</span>
        <span class="mail-snippet">{m.snippet}</span>
      </span>
    </div>
  );
}

export function MailModule() {
  const [folderId, setFolderId] = useState(-1); // -1 = unified inbox (S16)
  const [accountId, setAccountId] = useState<number | null>(null);
  const [accountEmail, setAccountEmail] = useState("");
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [rows, setRows] = useState<MessagePageRow[]>([]);
  const [exhausted, setExhausted] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  /** Failure from the background initial account sync (mail://synced). */
  const [syncNote, setSyncNote] = useState<string | null>(null);

  // Related Items click-through: a message jump selects it directly (the
  // reading pane loads by id, independent of the visible folder list).
  useEffect(() => {
    const jt = jumpTarget.value;
    if (activeModule.value === "mail" && jt?.kind === "message") {
      setSelected(jt.id);
      jumpTarget.value = null;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jumpTarget.value, activeModule.value]);
  const [wizard, setWizard] = useState(false);
  const [composeShown, setComposeShown] = useState(false);

  // Cross-module hand-off: Contacts "Send E-mail" pre-fills `compose` and
  // flips the signal; open the dialog here and consume the request.
  if (composeOpen.value) {
    composeOpen.value = false;
    setComposeShown(true);
  }
  const [treeReload, setTreeReload] = useState(0);
  const [query, setQuery] = useState("");
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [flaggedOnly, setFlaggedOnly] = useState(false);
  const [threadOpen, setThreadOpen] = useState<Record<number, boolean>>({});
  const [threadChildren, setThreadChildren] = useState<Record<number, MessagePageRow[]>>({});
  const busy = useRef(false);
  const done = useRef(false);
  done.current = exhausted;

  const pickFolder = (folder: FolderNode) => {
    setFolderId(folder.id);
    setAccountId(folder.account_id);
    setRows([]);
    setExhausted(false);
    done.current = false;
    setSelected(null);
    void api.mailAccounts().then((accs) => {
      const a = accs.find((x) => x.id === folder.account_id);
      setAccountEmail(a?.user ?? "");
    });
  };

  const loadMore = async () => {
    if (busy.current || done.current) return;
    busy.current = true;
    try {
      const page =
        query.trim() || unreadOnly || flaggedOnly
          ? await api.mailSearch(query, {
              folder_id: folderId > 0 ? folderId : null,
              account_id: null,
              unread_only: unreadOnly,
              flagged_only: flaggedOnly,
              from: null,
            })
          : folderId === -1
            ? await api.messagesUnified(nextCursor(rows), 100)
            : await api.messagesPage(folderId, nextCursor(rows), 100);
      if (page.length === 0) setExhausted(true);
      else setRows((r) => appendPage(r, page).rows);
    } catch {
      setExhausted(true);
    } finally {
      busy.current = false;
    }
  };

  // Reload when the folder or filters change (debounced for the search box).
  useEffect(() => {
    const t = window.setTimeout(() => {
      setRows([]);
      setExhausted(false);
      done.current = false;
      void loadMore();
    }, query ? 200 : 0);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [folderId, query, unreadOnly, flaggedOnly]);

  useEffect(() => {
    void loadMore();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [folderId, rows.length]);

  useEffect(() => {
    viewCount.value = rows.length;
  }, [rows.length]);

  useEffect(() => {
    void api.mailAccounts().then(setAccounts).catch(() => setAccounts([]));
  }, [treeReload]);

  // Background sync finished (account creation or Sync All) — refresh
  // folders, list and counts; surface failures without blocking anything.
  useEffect(() => {
    const un = api
      .listen("google://connected", (ev) => {
        const p = ev.payload as { email?: string; error?: string | null } | undefined;
        if (p?.error) {
          setSyncNote(`Connected as ${p.email ?? "Google"}, but the first sync failed: ${p.error}`);
        } else {
          setSyncNote(`Google Calendar & Tasks connected${p?.email ? ` as ${p.email}` : ""}.`);
        }
      })
      .catch(() => undefined);
    const unErr = api
      .listen("google://auth_error", (ev) => {
        const p = ev.payload as { error?: string } | undefined;
        setSyncNote(`Google connect failed: ${p?.error ?? "unknown error"}`);
      })
      .catch(() => undefined);
    const unSync = api
      .listen("google://synced", (ev) => {
        const p = ev.payload as { error?: string | null } | undefined;
        if (p?.error) setSyncNote(`Google sync failed: ${p.error}`);
      })
      .catch(() => undefined);
    const unMail = api
      .listen("mail://synced", (ev) => {
        const p = ev.payload as { account_id?: number; error?: string | null } | undefined;
        setFolderId(0);
        setTreeReload((k) => k + 1);
        setRows([]);
        setExhausted(false);
        done.current = false;
        void loadMore();
        void refreshCounts();
        if (p?.error) {
          setSyncNote(
            `Sync failed for an account: ${p.error} — check the account settings or re-add it.`,
          );
        } else {
          setSyncNote(null);
        }
      })
      .catch(() => undefined);
    return () => {
      for (const u of [un, unErr, unSync, unMail]) {
        void u.then((f) => f?.()).catch(() => undefined);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Account color for the accent bar — per-row only in a folder view. */
  const accent = accounts.find((a) => a.id === accountId)?.color ?? null;

  const listRows = withMailGroupHeaders(rows);

  const startCompose = (
    kind: "reply" | "reply-all" | "forward",
    msg: MessageDetail,
  ) => {
    const quoted = `\n\n> ${msg.body_text.split("\n").join("\n> ")}`;
    if (kind === "forward") {
      compose.to = "";
      compose.cc = "";
      compose.subject = /^fwd:/i.test(msg.subject) ? msg.subject : `Fwd: ${msg.subject}`;
      compose.body = `\n\n---------- Forwarded message ----------\nFrom: ${msg.from_name || msg.from_addr} <${msg.from_addr}>\nDate: ${new Date(msg.date).toLocaleString()}\nSubject: ${msg.subject}\n\n${msg.body_text}`;
    } else {
      compose.to = msg.from_addr;
      compose.cc = kind === "reply-all" ? msg.cc_addrs : "";
      compose.subject = /^re:/i.test(msg.subject) ? msg.subject : `Re: ${msg.subject}`;
      compose.body = quoted;
    }
    compose.open = true;
    setComposeShown(true);
  };

  /** AI-drafted reply → the SAME compose dialog, pre-filled for editing. */
  const applyAiDraft = (draft: { to: string; cc: string; subject: string; body: string }) => {
    compose.to = draft.to;
    compose.cc = draft.cc;
    compose.subject = draft.subject;
    compose.body = draft.body;
    compose.open = true;
    setComposeShown(true);
  };

  // -------------------------------------------------------- AI triage rules
  const status = useAiStatus();
  const [suggested, setSuggested] = useState<SuggestState>(SUGGEST_IDLE);

  const suggestRules = async () => {
    setSuggested({ ...SUGGEST_IDLE, open: true, busy: true });
    try {
      const rows = parseSuggestedRules(
        await runAiJob("triage_suggest", { account_id: accountId }),
      );
      setSuggested({
        ...SUGGEST_IDLE,
        open: true,
        rows,
        note:
          rows.length > 0
            ? "Edit the rows below, then create the ones you want."
            : "No rule suggestions.",
      });
    } catch (e) {
      setSuggested({ ...SUGGEST_IDLE, open: true, error: errText(e) });
    }
  };

  const createSuggestedRules = async () => {
    setSuggested((s) => ({ ...s, applying: true, error: null }));
    let created = 0;
    try {
      for (const r of suggested.rows) {
        if (!r.value.trim()) continue;
        await api.ruleCreate({
          account_id: r.account_id ?? accountId ?? null,
          field: r.field,
          op: r.op,
          value: r.value,
          action: r.action,
          action_value: r.action === "mark_read" ? null : r.action_value || null,
        });
        created++;
      }
      setSuggested((s) => ({
        ...s,
        applying: false,
        rows: [],
        note: `Created ${created} rule(s). Manage them under Tools → Mail Rules.`,
      }));
    } catch (e) {
      setSuggested((s) => ({ ...s, applying: false, error: errText(e) }));
    }
  };

  const editRuleRow = (i: number, patch: Partial<SuggestedRule>) =>
    setSuggested((s) => ({
      ...s,
      rows: s.rows.map((r, j) => (j === i ? { ...r, ...patch } : r)),
    }));

  useToolbar(
    <div class="mail-toolbar">
      <button class="primary" onClick={() => setComposeShown(true)}>
        Compose
      </button>
      <button
        onClick={async () => {
          if (accountId !== null) {
            try {
              await api.mailSync(accountId);
              setRows([]);
              setExhausted(false);
              done.current = false;
              setTreeReload((k) => k + 1);
              void refreshCounts();
            } catch (e) {
              alert(`Sync failed: ${e}`);
            }
          }
        }}
      >
        Sync
      </button>
      {featureEnabled(status, "triage") && (
        <button
          title="Ask AI to propose inbox rules (previewed first)"
          onClick={() => void suggestRules()}
        >
          Suggest Rules
        </button>
      )}
      <input
        class="mail-search"
        placeholder="Search mail… (subject, body, from)"
        value={query}
        onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
      />
      <label class="muted mail-filter">
        <input type="checkbox" checked={unreadOnly} onChange={(e) => setUnreadOnly((e.target as HTMLInputElement).checked)} /> unread
      </label>
      <label class="muted mail-filter">
        <input type="checkbox" checked={flaggedOnly} onChange={(e) => setFlaggedOnly((e.target as HTMLInputElement).checked)} /> flagged
      </label>
    </div>,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [accountId, query, unreadOnly, flaggedOnly, status],
  );

  return (
    <div class="mail-module">
      <GoogleLinkCard accountId={accountId} note={setSyncNote} />
      {syncNote && (
        <div class="sync-note">
          {syncNote}
          <button class="link-button" onClick={() => setSyncNote(null)}>
            dismiss
          </button>
        </div>
      )}
      <ThreePane
        module="mail"
        tree={
          <FolderTree
            selectedFolder={folderId}
            onPickFolder={pickFolder}
            onPickUnified={() => {
              setFolderId(-1);
              setAccountId(null);
              setRows([]);
              setExhausted(false);
              done.current = false;
              setSelected(null);
            }}
            onAddAccount={() => setWizard(true)}
            reloadKey={treeReload}
          />
        }
        list={
          <div class="mail-list">
            {rows.length === 0 ? (
              <p class="muted empty-pane">
                {exhausted ? "No messages." : "Loading…"}
              </p>
            ) : (
              <VirtualList
                rowCount={listRows.length}
                rowHeight={ROW_H}
                onNearEnd={() => void loadMore()}
                renderRow={(i) => {
                  const row = listRows[i];
                  if (row.type === "header") {
                    return (
                      <div key={`h${i}`} class="list-date-header" style={{ height: `${ROW_H}px` }}>
                        {row.label}
                      </div>
                    );
                  }
                  const m = row.item;
                  const unread = !/"seen":\s*true/.test(m.flags);
                  const flagged = /"flagged":1/.test(m.flags);
                  const isGrouped =
                    m.thread_id !== null && !query && !unreadOnly && !flaggedOnly;
                  const expanded = m.thread_id !== null && threadOpen[m.thread_id];
                  const childCount = m.thread_id !== null ? (threadChildren[m.thread_id]?.length ?? 0) : 0;
                  if (expanded && m.thread_id !== null) {
                    // This row renders the expanded thread's messages instead.
                    const msgs = threadChildren[m.thread_id] ?? [];
                    return (
                      <div key={`t${m.thread_id}`}>
                        {msgs.map((t) => (
                          <MailRow
                            key={t.id}
                            m={t}
                            selected={t.id === selected}
                            indent
                            accent={accent}
                            onPick={() => setSelected(t.id)}
                          />
                        ))}
                        <button
                          class="thread-toggle"
                          style={{ height: `${ROW_H / 2}px` }}
                          onClick={() => setThreadOpen((o) => ({ ...o, [m.thread_id!]: false }))}
                        >
                          ▴ collapse thread
                        </button>
                      </div>
                    );
                  }
                  return (
                    <div key={m.id} style={{ height: isGrouped ? undefined : `${ROW_H}px` }}>
                      <MailRow
                        m={m}
                        selected={m.id === selected}
                        flagged={flagged}
                        unread={unread}
                        accent={accent}
                        threadCount={isGrouped ? Math.max(childCount, 1) : 0}
                        onToggleThread={
                          isGrouped
                            ? async () => {
                                const tid = m.thread_id!;
                                if (!threadOpen[tid]) {
                                  const msgs = await api.threadMessages(tid);
                                  setThreadChildren((c) => ({ ...c, [tid]: msgs }));
                                }
                                setThreadOpen((o) => ({ ...o, [tid]: !o[tid] }));
                              }
                            : undefined
                        }
                        onPick={() => setSelected(m.id)}
                      />
                    </div>
                  );
                }}
              />
            )}
          </div>
        }
        detail={
          selected !== null ? (
            <ReadingPane
              messageId={selected}
              accounts={accounts}
              defaultAccountId={accountId}
              onCompose={startCompose}
              onAiDraft={applyAiDraft}
              onChanged={() => {
                setRows([]);
                setExhausted(false);
                done.current = false;
                void loadMore();
                void refreshCounts();
              }}
            />
          ) : (
            <div class="muted empty-pane">Select a message to read it.</div>
          )
        }
      />
      {wizard && (
        <AccountWizard
          onClose={() => setWizard(false)}
          onCreated={() => {
            setFolderId(0);
            setTreeReload((k) => k + 1);
            void refreshCounts();
          }}
        />
      )}
      {composeShown && (
        <ComposeDialog
          account={accountId !== null ? { id: accountId, email: accountEmail } : null}
          onClose={() => {
            setComposeShown(false);
            compose.open = false;
          }}
        />
      )}
      <AiPreview
        open={suggested.open}
        title="Suggested rules"
        onClose={() => setSuggested(SUGGEST_IDLE)}
        busy={suggested.busy}
        busyLabel="Analysing your inbox…"
        error={suggested.error}
        actions={[
          {
            label: "Edit in Rules…",
            disabled: suggested.busy || suggested.rows.length === 0,
            onClick: () => {
              rulePrefill.value = suggested.rows[0] ?? null;
              openRules();
              setSuggested(SUGGEST_IDLE);
            },
          },
          {
            label: "Create rules",
            primary: true,
            disabled: suggested.applying || suggested.rows.length === 0,
            onClick: () => void createSuggestedRules(),
          },
        ]}
      >
        {suggested.rows.map((r, i) => (
          <div class="ai-rule-row" key={i}>
            <select
              value={r.field}
              aria-label="Field"
              onChange={(e) => editRuleRow(i, { field: (e.target as HTMLSelectElement).value })}
            >
              <option value="from">from</option>
              <option value="to">to</option>
              <option value="subject">subject</option>
            </select>
            <select
              value={r.op}
              aria-label="Operator"
              onChange={(e) => editRuleRow(i, { op: (e.target as HTMLSelectElement).value })}
            >
              <option value="contains">contains</option>
              <option value="equals">equals</option>
            </select>
            <input
              value={r.value}
              aria-label="Value"
              onInput={(e) => editRuleRow(i, { value: (e.target as HTMLInputElement).value })}
            />
            <select
              value={r.action}
              aria-label="Action"
              onChange={(e) => editRuleRow(i, { action: (e.target as HTMLSelectElement).value })}
            >
              <option value="move">move to folder</option>
              <option value="mark_read">mark read</option>
              <option value="tag">tag</option>
            </select>
            <input
              placeholder={r.action === "tag" ? "Tag" : "Folder"}
              value={r.action_value ?? ""}
              disabled={r.action === "mark_read"}
              aria-label="Action target"
              onInput={(e) =>
                editRuleRow(i, { action_value: (e.target as HTMLInputElement).value })
              }
            />
          </div>
        ))}
        {suggested.rows.length === 0 && !suggested.busy && (
          <p class="muted">No rule suggestions.</p>
        )}
        {suggested.note && <div class="ai-note">{suggested.note}</div>}
      </AiPreview>
    </div>
  );
}
