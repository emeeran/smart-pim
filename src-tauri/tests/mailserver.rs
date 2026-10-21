//! AT-14: IMAP + POP3 sync and SMTP send against live fixtures
//! (Dovecot on 10143/10110, Mailpit SMTP sink on 1025 — `scripts/mailfixture/up.sh`).
//!
//! When the fixtures are not running, the tests log a SKIP note and pass —
//! they are gated, not faked; the sync logic itself is additionally covered
//! by unit tests that need no server.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Mutex, MutexGuard};

use pfcore::rules;
use pfcore::store::Store;
use smart_pim_lib::mailsync;

/// The tests share one Dovecot maildrop; the "how many did we fetch"
/// assertions are only meaningful when tests don't append concurrently.
static FIXTURE: Mutex<()> = Mutex::new(());

fn fixture() -> MutexGuard<'static, ()> {
    FIXTURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const IMAP: &str = "127.0.0.1:10143";
const POP3: &str = "127.0.0.1:10110";
const SMTP_HTTP: &str = "127.0.0.1:8025";
const USER: &str = "test@example.com";
const PASS: &str = "secret123";

fn fixtures_up() -> bool {
    let ok = [IMAP, POP3].iter().all(|addr| {
        TcpStream::connect(addr)
            .ok()
            .map(|mut s| s.read(&mut [0u8; 8]).is_ok())
            .unwrap_or(false)
    });
    if !ok {
        eprintln!("SKIP: mail fixtures not running (scripts/mailfixture/up.sh)");
    }
    ok
}

fn imap_session() -> Option<imap::Session<TcpStream>> {
    let client = imap::Client::new(TcpStream::connect(IMAP).ok()?);
    client.login(USER, PASS).ok()
}

fn append_to_inbox(raw: &str) {
    let mut s = imap_session().expect("fixture imap login");
    s.append("INBOX", raw.as_bytes()).expect("imap append");
}

fn imap_count() -> usize {
    let mut s = imap_session().expect("fixture imap login");
    let mb = s.examine("INBOX").expect("examine INBOX");
    mb.exists as usize
}

fn imap_account(conn: &rusqlite::Connection) -> i64 {
    let id = pfcore::mailstore::create_account(
        conn,
        "Fixture IMAP",
        "imap",
        Some("127.0.0.1"),
        Some(10143),
        Some("none"),
    )
    .unwrap();
    set_password(conn, id);
    id
}

fn set_password(conn: &rusqlite::Connection, account: i64) {
    conn.execute(
        "UPDATE mail_account SET security = 'none', user = ?2, \
         settings = json_set(settings, '$.password', ?3) WHERE id = ?1",
        rusqlite::params![account, USER, PASS],
    )
    .unwrap();
}

fn eml(subject: &str, from: &str, body: &str) -> String {
    format!(
        "From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\n\
         Date: Sat, 18 Oct 2025 10:00:00 +0000\r\nMessage-ID: <{subject}@fixture.test>\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
}

fn mailpit_total() -> u32 {
    let mut s = TcpStream::connect(SMTP_HTTP).expect("mailpit api");
    s.write_all(b"GET /api/v1/messages HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).unwrap();
    // Lenient: the first "total":N anywhere in the response body.
    let idx = buf.find("\"total\":").map(|i| i + "\"total\":".len());
    idx.and_then(|i| {
        buf[i..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    })
    .unwrap_or(0)
}

#[test]
fn at14_imap_sync_is_incremental_and_fts_indexed() {
    if !fixtures_up() {
        return;
    }
    let _fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("db"), "t").unwrap();
    let account = imap_account(store.conn_mut());

    let marker = format!("Sync probe {}", std::process::id());
    append_to_inbox(&eml(&marker, "sync@fixture.test", "hello from imap"));
    append_to_inbox(&eml("Second message", "two@fixture.test", "body two"));

    let reports = mailsync::sync_imap_account(store.conn_mut(), account).unwrap();
    let inbox = reports
        .iter()
        .find(|r| r.folder == "INBOX")
        .expect("INBOX report");
    assert!(
        inbox.total >= 2,
        "at least the two appended messages: {:?}",
        inbox
    );

    let hits = pfcore::search::global_search(
        store.conn(),
        &marker,
        pfcore::search::SearchScope {
            mail: true,
            ..Default::default()
        },
        10,
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "synced message is FTS-indexed");
    assert_eq!(hits[0].title, marker);

    // Incremental: nothing new → nothing fetched; one new message → 1.
    let again = mailsync::sync_imap_account(store.conn_mut(), account).unwrap();
    let inbox = again.iter().find(|r| r.folder == "INBOX").unwrap();
    assert_eq!(inbox.total, imap_count());

    let marker2 = format!("Delta probe {}", std::process::id());
    append_to_inbox(&eml(&marker2, "delta@fixture.test", "delta body"));
    let third = mailsync::sync_imap_account(store.conn_mut(), account).unwrap();
    let inbox = third.iter().find(|r| r.folder == "INBOX").unwrap();
    assert_eq!(inbox.fetched, 1, "only the new UID is fetched: {inbox:?}");
}

#[test]
fn at14_imap_rules_fire_on_incoming() {
    if !fixtures_up() {
        return;
    }
    let _fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("db"), "t").unwrap();
    let conn = store.conn_mut();
    let account = imap_account(conn);
    rules::create_rule(
        conn,
        Some(account),
        &[rules::Condition {
            field: "from".into(),
            op: "contains".into(),
            value: "ruleme@fixture.test".into(),
        }],
        &[
            rules::Action::Move {
                folder: "Handled".into(),
            },
            rules::Action::MarkRead,
        ],
        0,
    )
    .unwrap();

    append_to_inbox(&eml(
        "Rule target",
        "ruleme@fixture.test",
        "should be moved",
    ));
    mailsync::sync_imap_account(conn, account).unwrap();

    let folder: String = conn
        .query_row(
            "SELECT f.name FROM message m JOIN mail_folder f ON f.id = m.folder_id \
             WHERE m.subject LIKE 'Rule target%' ORDER BY m.id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(folder, "Handled", "rule moved the message on ingest");
    let seen: i64 = conn
        .query_row(
            "SELECT json_extract(flags, '$.seen') FROM message \
             WHERE subject LIKE 'Rule target%' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seen, 1, "rule marked it read");
}

#[test]
fn at14_pop3_sync_is_incremental_and_nondestructive() {
    if !fixtures_up() {
        return;
    }
    let _fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("db"), "t").unwrap();
    let conn = store.conn_mut();
    let id = pfcore::mailstore::create_account(
        conn,
        "Fixture POP3",
        "pop3",
        Some("127.0.0.1"),
        Some(10110),
        Some("none"),
    )
    .unwrap();
    set_password(conn, id);

    let marker = format!("Pop3 probe {}", std::process::id());
    append_to_inbox(&eml(&marker, "pop@fixture.test", "via pop3"));

    let first = mailsync::sync_pop3_account(conn, id).unwrap();
    assert!(first.total >= 1);
    assert_eq!(
        first.fetched, first.total,
        "first sync fetches the full maildrop"
    );

    let second = mailsync::sync_pop3_account(conn, id).unwrap();
    assert_eq!(second.fetched, 0, "UIDL dedupe: nothing re-fetched");

    let hits = pfcore::search::global_search(
        conn,
        &marker,
        pfcore::search::SearchScope {
            mail: true,
            ..Default::default()
        },
        10,
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
    // Non-destructive: the server maildrop still holds the message.
    assert!(imap_count() >= 1, "POP3 sync must not delete server mail");
}

#[test]
fn at14_smtp_send_later_and_outbox_pump() {
    if !fixtures_up() {
        return;
    }
    let _fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("db"), "t").unwrap();
    let conn = store.conn_mut();
    let account = imap_account(conn);
    conn.execute(
        "UPDATE mail_account SET settings = json_set(settings, \
         '$.smtp_host', '127.0.0.1', '$.smtp_port', 1025, '$.smtp_user', '', '$.smtp_pass', '') \
         WHERE id = ?1",
        [account],
    )
    .unwrap();
    let tag = format!("send-later {}", std::process::id());

    // Future-scheduled send stays queued.
    let future = rules::queue_outgoing(
        conn,
        &serde_json::to_string(&mailsync::Draft {
            account_id: account,
            from_name: "Smart PIM".into(),
            from_email: "me@example.com".into(),
            to: vec!["sink@example.test".into()],
            cc: vec![],
            bcc: vec![],
            subject: format!("future {tag}"),
            text_body: "later".into(),
            html_body: String::new(),
        })
        .unwrap(),
        Some("2099-01-01T00:00:00+00:00"),
    )
    .unwrap();

    let (sent, failed) = mailsync::process_outbox(conn, 10).unwrap();
    assert_eq!((sent, failed), (0, 0), "future send must stay queued");

    // Re-schedule into the past → due + flagged missed (S21 recovery).
    conn.execute(
        "UPDATE outbox SET send_at = '2020-01-01T00:00:00+00:00' WHERE id = ?1",
        [future],
    )
    .unwrap();
    let before = mailpit_total();
    let (sent, failed) = mailsync::process_outbox(conn, 10).unwrap();
    let smtp_error: Option<String> = conn
        .query_row("SELECT error FROM outbox WHERE id = ?1", [future], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!((sent, failed), (1, 0), "smtp error: {smtp_error:?}");
    assert!(mailpit_total() > before, "SMTP sink received the message");

    let (state, missed_at): (String, Option<String>) = conn
        .query_row(
            "SELECT state, send_at FROM outbox WHERE id = ?1",
            [future],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "sent");
    assert_eq!(missed_at.as_deref(), Some("2020-01-01T00:00:00+00:00"));
}
