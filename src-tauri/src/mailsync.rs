//! Mail protocol clients + sync worker (T1.10 / S15–S16, S20–S21):
//! IMAP with incremental UID fetch, a minimal POP3 client (`pop3` submodule),
//! SMTP send via lettre, and the outbox pump (immediate + Send Later, S21).
//!
//! Account credentials live in the (SQLCipher-encrypted) `mail_account`
//! settings JSON — the database file is the secret store; nothing sensitive
//! touches disk elsewhere.

pub mod pop3;

use std::net::TcpStream;

use pfcore::mailstore::{self};
use pfcore::rules;
use pfcore::Error;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct AccountCreds {
    pub id: i64,
    pub kind: String,
    pub host: String,
    pub port: i64,
    /// "none" | "starttls" | "tls"
    pub security: String,
    pub user: String,
    pub password: String,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_user: Option<String>,
    pub smtp_pass: Option<String>,
}

/// Account row: (kind, host, port, security, user, settings JSON).
type AccountRow = (
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    String,
);

/// Load credentials from the account row + its settings JSON.
pub fn load_creds(conn: &Connection, account_id: i64) -> pfcore::Result<AccountCreds> {
    let row: Option<AccountRow> = conn
        .query_row(
            "SELECT kind, host, port, security, user, settings FROM mail_account WHERE id = ?1",
            [account_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, host, port, security, user, settings)) = row else {
        return Err(Error::NotFound(account_id));
    };
    let settings: serde_json::Value = serde_json::from_str(&settings)?;
    let get = |k: &str| settings.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let get_num = |k: &str| {
        settings.get(k).and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
    };
    let default_port = i64::from(kind == "imap") * 993 + i64::from(kind == "pop3") * 995;
    Ok(AccountCreds {
        id: account_id,
        kind,
        host: host.unwrap_or_default(),
        port: port.unwrap_or(default_port),
        security: security.unwrap_or_else(|| "tls".into()),
        user: user.unwrap_or_default(),
        password: get("password").unwrap_or_default(),
        smtp_host: get("smtp_host"),
        smtp_port: get_num("smtp_port"),
        smtp_user: get("smtp_user"),
        smtp_pass: get("smtp_pass"),
    })
}

// ------------------------------------------------------------------ IMAP

#[derive(Debug, Default, Clone, Serialize)]
pub struct FolderSyncReport {
    pub folder: String,
    pub fetched: usize,
    pub total: usize,
}

/// Connected IMAP session, over plain TCP or TLS (native-tls).
enum ImapConn {
    Plain(imap::Session<TcpStream>),
    Tls(imap::Session<native_tls::TlsStream<TcpStream>>),
}

fn open_session(creds: &AccountCreds) -> Result<ImapConn, String> {
    let addr = (
        creds.host.as_str(),
        u16::try_from(creds.port).unwrap_or(993),
    );
    let login = |c: imap::Client<native_tls::TlsStream<TcpStream>>| {
        c.login(&creds.user, &creds.password)
            .map_err(|e| e.0.to_string())
    };
    match creds.security.as_str() {
        "none" => {
            let client = imap::Client::new(TcpStream::connect(addr).map_err(|e| e.to_string())?);
            client
                .login(&creds.user, &creds.password)
                .map_err(|e| e.0.to_string())
                .map(ImapConn::Plain)
        }
        "starttls" => {
            let client = imap::Client::new(TcpStream::connect(addr).map_err(|e| e.to_string())?);
            let tls = native_tls::TlsConnector::builder()
                .build()
                .map_err(|e| e.to_string())?;
            let secure = client
                .secure(&creds.host, &tls)
                .map_err(|e| e.to_string())?;
            login(secure).map(ImapConn::Tls)
        }
        _ => {
            // implicit TLS
            let tls = native_tls::TlsConnector::builder()
                .build()
                .map_err(|e| e.to_string())?;
            let client = imap::connect(addr, &creds.host, &tls).map_err(|e| e.to_string())?;
            login(client).map(ImapConn::Tls)
        }
    }
}

/// Sync one IMAP account: folders discovered via LIST, incremental fetch of
/// UIDs beyond `last_seen_uid`, rules applied to each new message.
///
/// Lock discipline: the store mutex is held only around database work
/// (folder bookkeeping, message writes), never across network I/O. A Gmail
/// first sync moves megabytes over TLS; holding the lock through that
/// starves every other command and freezes the UI.
pub fn sync_imap_account(
    state: &crate::state::AppState,
    account_id: i64,
) -> pfcore::Result<Vec<FolderSyncReport>> {
    let creds = {
        let guard = state.store.lock().expect("store mutex");
        let Some(store) = guard.as_ref() else {
            return Ok(Vec::new()); // locked DB: nothing to do
        };
        load_creds(store.conn(), account_id)?
    };
    let mut session = open_session(&creds).map_err(|e| Error::parse("imap", e))?;
    match &mut session {
        ImapConn::Plain(s) => sync_imap_inner(state, account_id, s),
        ImapConn::Tls(s) => sync_imap_inner(state, account_id, s),
    }
}

fn sync_imap_inner<S: std::io::Read + std::io::Write>(
    state: &crate::state::AppState,
    account_id: i64,
    session: &mut imap::Session<S>,
) -> pfcore::Result<Vec<FolderSyncReport>> {
    let listings = session
        .list(None, Some("*"))
        .map_err(|e| Error::parse("imap", e.to_string()))?;
    // `\Noselect` mailboxes (and anything the server lists but refuses to
    // open — Gmail labels can do both) are skipped, never fatal: one odd
    // folder must not kill the whole account sync.
    let folders: Vec<String> = listings
        .iter()
        .filter(|n| {
            !n.attributes()
                .contains(&imap::types::NameAttribute::NoSelect)
        })
        .map(|n| n.name().to_string())
        .collect();

    let mut reports = Vec::new();
    for name in folders {
        let Ok(mailbox) = session.examine(&name) else {
            continue;
        };
        let uidvalidity = mailbox.uid_validity.unwrap_or(0);

        // Phase 1 (locked): folder row + cached cursor.
        let (folder_id, mut max_uid) = {
            let guard = state.store.lock().expect("store mutex");
            let Some(store) = guard.as_ref() else {
                continue;
            };
            let conn = store.conn();
            let folder_id = mailstore::ensure_folder(conn, account_id, None, &name, None)?;
            let stored: (i64, i64) = conn.query_row(
                "SELECT COALESCE(uidvalidity, 0), last_seen_uid FROM mail_folder WHERE id = ?1",
                [folder_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            // UIDVALIDITY change invalidates cached UIDs — resync from scratch.
            let max_uid = if stored.0 != 0 && stored.0 != i64::from(uidvalidity) {
                0
            } else {
                u32::try_from(stored.1).unwrap_or(0)
            };
            (folder_id, max_uid)
        };

        let Ok(all) = session.uid_search("ALL") else {
            continue; // listed but unusable — skip without failing the account
        };
        let total = all.len();
        let mut fetched = 0usize;
        let fresh: Vec<u32> = all.into_iter().filter(|u| *u > max_uid).collect();

        for chunk in fresh.chunks(64) {
            let set = chunk
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            // Phase 2 (unlocked): fetch raw bodies over the wire.
            let batch = session
                .uid_fetch(set, "(UID FLAGS BODY.PEEK[])")
                .map_err(|e| Error::parse("imap", e.to_string()))?;
            // Phase 3 (locked): parse + persist the batch.
            let guard = state.store.lock().expect("store mutex");
            let Some(store) = guard.as_ref() else {
                break; // DB locked mid-sync: keep what we have
            };
            let conn = store.conn();
            for msg in batch.iter() {
                let uid = msg.uid.unwrap_or(0);
                let Some(body) = msg.body() else { continue };
                store_message(
                    conn,
                    account_id,
                    folder_id,
                    uid,
                    body,
                    msg.flags().contains(&imap::types::Flag::Seen),
                )?;
                max_uid = max_uid.max(uid);
                fetched += 1;
            }
            drop(guard);
        }

        // Phase 4 (locked): commit the folder cursor.
        {
            let guard = state.store.lock().expect("store mutex");
            if let Some(store) = guard.as_ref() {
                let _ = store.conn().execute(
                    "UPDATE mail_folder SET uidvalidity = ?2, last_seen_uid = ?3, \
                     sync_state = 'ok' WHERE id = ?1",
                    params![folder_id, i64::from(uidvalidity), i64::from(max_uid)],
                );
            }
        }
        reports.push(FolderSyncReport {
            folder: name,
            fetched,
            total,
        });
    }
    Ok(reports)
}

/// Parse raw RFC 822 bytes and upsert + rule-apply one message.
pub fn store_message(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    uid: u32,
    raw: &[u8],
    seen: bool,
) -> pfcore::Result<()> {
    let mut input = mailstore::parse_message(raw)?;
    input.uid = i64::from(uid);
    if seen {
        input.flags = r#"{"seen":true}"#.to_string();
    }
    let msg_id = mailstore::upsert_message(conn, account_id, folder_id, &input)?;
    // Rules (S22): evaluated and applied to the freshly stored message.
    let item_id: Option<i64> = conn
        .query_row("SELECT item_id FROM message WHERE id = ?1", [msg_id], |r| {
            r.get(0)
        })
        .ok();
    let outcome = rules::evaluate(
        conn,
        account_id,
        &input.subject,
        &input.from_addr,
        &input.to_addrs,
    )?;
    rules::apply_outcome(conn, msg_id, item_id, &outcome)?;
    Ok(())
}

// ------------------------------------------------------------------ POP3

/// Sync a POP3 account: fetch messages whose UIDL we haven't stored. The
/// UIDL string is fingerprinted (SHA-256 prefix) into `message.uid`.
pub fn sync_pop3_account(
    conn: &mut Connection,
    account_id: i64,
) -> pfcore::Result<FolderSyncReport> {
    let creds = load_creds(conn, account_id)?;
    let stream = TcpStream::connect((
        creds.host.as_str(),
        u16::try_from(creds.port).unwrap_or(995),
    ))
    .map_err(Error::Io)?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(15)))
        .ok();
    let mut client = pop3::Pop3::new(stream).map_err(Error::Io)?;
    client
        .read_greeting()
        .map_err(|e| Error::parse("pop3", e))?;
    client
        .login(&creds.user, &creds.password)
        .map_err(|e| Error::parse("pop3", e))?;
    let inbox = mailstore::ensure_folder(conn, account_id, None, "INBOX", Some("inbox"))?;

    let uidls = client.uidl().map_err(|e| Error::parse("pop3", e))?;
    let mut fetched = 0usize;
    for (idx, uidl) in &uidls {
        let fingerprint = uidl_fingerprint(uidl);
        let known: i64 = conn
            .query_row(
                "SELECT count(*) FROM message WHERE folder_id = ?1 AND uid = ?2",
                params![inbox, fingerprint],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if known > 0 {
            continue;
        }
        let raw = client.retr(*idx).map_err(|e| Error::parse("pop3", e))?;
        store_message(conn, account_id, inbox, fingerprint, &raw, false)?;
        fetched += 1;
    }
    let _ = client.quit();
    Ok(FolderSyncReport {
        folder: "INBOX".into(),
        fetched,
        total: uidls.len(),
    })
}

/// Stable per-maildrop identity for a UIDL string (stored in `message.uid`).
fn uidl_fingerprint(uidl: &str) -> u32 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(uidl.as_bytes());
    u32::from_be_bytes(digest[..4].try_into().unwrap_or([0; 4]))
}

// ------------------------------------------------------------------ SMTP

/// One outgoing message (the `outbox.draft_json` payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub account_id: i64,
    pub from_name: String,
    pub from_email: String,
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    #[serde(default)]
    pub text_body: String,
    #[serde(default)]
    pub html_body: String,
}

impl Draft {
    fn into_lettre(self) -> Result<lettre::message::Message, String> {
        use lettre::message::Mailbox;
        let from: Mailbox = Mailbox {
            name: Some(self.from_name.clone()).filter(|n| !n.is_empty()),
            email: self
                .from_email
                .parse()
                .map_err(|e| format!("bad from: {e}"))?,
        };
        let mut builder = lettre::message::Message::builder()
            .from(from)
            .subject(&self.subject);
        for rcpt in self.to.iter().chain(&self.cc) {
            let mbox: Mailbox = rcpt
                .trim()
                .parse()
                .map_err(|e| format!("bad recipient {rcpt}: {e}"))?;
            builder = builder.to(mbox);
        }
        for rcpt in &self.bcc {
            let mbox: Mailbox = rcpt
                .trim()
                .parse()
                .map_err(|e| format!("bad recipient {rcpt}: {e}"))?;
            builder = builder.bcc(mbox);
        }
        if self.html_body.is_empty() {
            builder.body(self.text_body).map_err(|e| e.to_string())
        } else {
            builder
                .multipart(lettre::message::MultiPart::alternative_plain_html(
                    self.text_body,
                    self.html_body,
                ))
                .map_err(|e| e.to_string())
        }
    }
}

/// Send a draft through the account's SMTP relay (S20).
pub fn send_draft(conn: &Connection, draft: &Draft) -> pfcore::Result<()> {
    use lettre::Transport;
    let creds = load_creds(conn, draft.account_id)?;
    // Empty identity fields are filled from the account (compose sends the
    // account's login address unless the user overrode it).
    let mut draft = draft.clone();
    if draft.from_email.is_empty() {
        draft.from_email = creds.user.clone();
    }
    let host = creds
        .smtp_host
        .clone()
        .ok_or_else(|| Error::parse("smtp", "account has no smtp_host"))?;
    let port = u16::try_from(creds.smtp_port.unwrap_or(587)).unwrap_or(587);
    let email = draft
        .clone()
        .into_lettre()
        .map_err(|e| Error::parse("smtp", e))?;

    let transport = match creds.smtp_user.as_deref().filter(|u| !u.is_empty()) {
        Some(user) => {
            let relay = if creds.security == "none" {
                lettre::transport::smtp::SmtpTransport::builder_dangerous(&host).port(port)
            } else {
                lettre::transport::smtp::SmtpTransport::relay(&host)
                    .map_err(|e| Error::parse("smtp", e.to_string()))?
                    .port(port)
            };
            relay
                .credentials(lettre::transport::smtp::authentication::Credentials::new(
                    user.to_string(),
                    creds.smtp_pass.unwrap_or_default(),
                ))
                .build()
        }
        None => lettre::transport::smtp::SmtpTransport::builder_dangerous(&host)
            .port(port)
            .build(),
    };
    transport
        .send(&email)
        .map_err(|e| Error::parse("smtp", e.to_string()))?;
    Ok(())
}

/// Pump the outbox: send everything due now (S21). Missed scheduled sends
/// were already flagged by `rules::take_due_outgoing`; callers surface that.
pub fn process_outbox(conn: &mut Connection, limit: i64) -> pfcore::Result<(usize, usize)> {
    let due = rules::take_due_outgoing(conn, &pfcore::model::now_iso(), limit)?;
    let mut sent = 0usize;
    let mut failed = 0usize;
    for entry in due {
        match serde_json::from_str::<Draft>(&entry.draft_json) {
            Ok(draft) => match send_draft(conn, &draft) {
                Ok(()) => {
                    rules::mark_sent(conn, entry.id)?;
                    sent += 1;
                }
                Err(e) => {
                    rules::mark_failed(conn, entry.id, &e.to_string())?;
                    failed += 1;
                }
            },
            Err(e) => {
                rules::mark_failed(conn, entry.id, &format!("bad draft json: {e}"))?;
                failed += 1;
            }
        }
    }
    Ok((sent, failed))
}
