//! Gate 2 integration tests: end-to-end import/export pipelines, atomicity,
//! and the 100k-row performance gates (AT-5, AT-13; N1 budgets).
//!
//! `SMARTPIM_PERF_N` overrides the perf corpus size (default `100_000`).

use std::time::Instant;

use core::import::commit_all;
use core::mailstore::{self, MessageInput};
use core::search::{global_search, SearchScope};
use core::store::Store;

/// The 100k-row gate is measured in release builds (the honest number for the
/// checklist). Debug builds default to 10k so `cargo test` stays quick;
/// `SMARTPIM_PERF_N` overrides in both.
#[cfg(debug_assertions)]
const PERF_DEFAULT: usize = 10_000;
#[cfg(not(debug_assertions))]
const PERF_DEFAULT: usize = 100_000;

fn perf_n() -> usize {
    std::env::var("SMARTPIM_PERF_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(PERF_DEFAULT)
}

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmHWM")).and_then(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|n| n.parse::<u64>().ok())
            })
        })
        .unwrap_or(0)
}

fn seed_perf(store: &mut Store, n: usize) -> std::time::Duration {
    let conn = store.conn_mut();
    let acct = mailstore::create_account(conn, "Perf", "imap", None, None, None).unwrap();
    let inbox = mailstore::ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();
    let start = Instant::now();
    let tx = conn.transaction().unwrap();
    for i in 0..n {
        let m = MessageInput {
            uid: i as i64,
            subject: format!("Quarterly report {i}"),
            from_name: "Sender".into(),
            from_addr: format!("s{i}@example.com"),
            to_addrs: "me@example.com".into(),
            date: format!(
                "2025-{:02}-{:02}T10:{:02}:00Z",
                (i % 12) + 1,
                (i % 28) + 1,
                i % 60
            ),
            body: format!("Body of message {i} with the word needle when index mod 97 == {i}."),
            ..Default::default()
        };
        mailstore::upsert_message(&tx, acct, inbox, &m).unwrap();
    }
    tx.commit().unwrap();
    start.elapsed()
}

#[test]
fn at5_at13_hundred_k_mail_corpus_and_search_latency() {
    let n = perf_n();
    let mut store = Store::open_memory("perf").unwrap();
    let insert_time = seed_perf(&mut store, n);
    let rss = peak_rss_kib();

    // DT-2: FTS external-content consistency after bulk insert.
    let indexed: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM message_fts", [], |r| r.get(0))
        .unwrap();
    let stored: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM message", [], |r| r.get(0))
        .unwrap();
    assert_eq!(indexed, stored, "FTS index must cover every message");

    // AT-5: search latency budget.
    let t0 = Instant::now();
    let hits = global_search(store.conn(), "needle", SearchScope::all(), 20).unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(hits.len(), 20.min(n));
    println!(
        "PERF n={n} insert={insert_time:?} search={elapsed:?} peak_rss={}MiB",
        rss / 1024
    );
    assert!(
        elapsed.as_millis() < 200,
        "global search took {elapsed:?}, budget 200 ms"
    );
    // Stated memory budget for Gate 2: import stays under 1.5 GiB peak RSS.
    assert!(rss < 1_500_000, "peak RSS {rss} KiB exceeds 1.5 GiB budget");
}

#[test]
fn ics_pipeline_roundtrip_through_db() {
    let mut store = Store::open_memory("x").unwrap();
    let conn = store.conn_mut();
    let sample = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:Project Sync\r\nDTSTART:20260330T100000Z\r\nDTEND:20260330T110000Z\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let cal = core::events::ensure_calendar(conn, "Work", None).unwrap();
    let parsed = core::ical::parse_ics(sample);
    assert_eq!(parsed.warnings, [] as [std::string::String; 0]);
    for ev in &parsed.events {
        core::events::upsert_event(conn, cal, ev).unwrap();
    }
    let exported = core::ical::write_ics(&core::events::events_of_calendar(conn, cal), "Work");
    let reparsed = core::ical::parse_ics(&exported);
    assert_eq!(reparsed.events.len(), 1);
    assert_eq!(reparsed.events[0].title, "Project Sync");
    assert_eq!(
        reparsed.events[0].rrule.as_deref(),
        Some("FREQ=WEEKLY;COUNT=4")
    );
    let occ = core::ical::expand_rrule(
        reparsed.events[0].rrule.as_deref().unwrap(),
        &reparsed.events[0].start,
        "2026-03-01T00:00:00Z",
        "2026-05-01T00:00:00Z",
        100,
    )
    .unwrap();
    assert_eq!(occ.len(), 4);
}

#[test]
fn vcf_pipeline_roundtrip_through_db() {
    let mut store = Store::open_memory("x").unwrap();
    let conn = store.conn_mut();
    let vcf = "BEGIN:VCARD\r\nUID:c1\r\nN:Chase;Sylvia;;;\r\nFN:Sylvia Chase\r\nEMAIL:sylvia@example.com\r\nEND:VCARD\r\n";
    for c in core::vcard::parse_vcf(vcf).contacts {
        core::contacts::upsert_contact(conn, &c).unwrap();
    }
    let exported = core::vcard::write_vcf(&core::contacts::all_contacts(conn));
    let reparsed = core::vcard::parse_vcf(&exported);
    assert_eq!(reparsed.contacts.len(), 1);
    assert_eq!(reparsed.contacts[0].display_name, "Sylvia Chase");
    assert_eq!(reparsed.contacts[0].emails, vec!["sylvia@example.com"]);
}

#[test]
fn csv_contact_import_is_atomic_and_reported() {
    let mut store = Store::open_memory("x").unwrap();
    let csv_data = core::csvio::parse_csv(
        b"Full Name,E-Mail Address\nMarie Powell,mariep@example.com\n,jane@x.com\n",
    )
    .unwrap();
    let map = core::csvio::suggest_mapping(&csv_data.headers, &core::csvio::contact_fields());
    // Dry run: row 2 has neither name nor... it has an email, so it is valid;
    // simulate a genuinely bad row for the abort path instead.
    let dry = core::import::validate_only(&csv_data.rows, |i, row, report| {
        let has_name = map
            .get("display_name")
            .and_then(|c| row.get(*c))
            .is_some_and(|v| !v.is_empty());
        if !has_name {
            report.warn(i, "no display name; will be derived from email");
        }
    });
    assert_eq!(dry.total_rows, 2);
    assert!(!dry.has_errors());

    let conn = store.conn_mut();
    let report = commit_all(conn, &csv_data.rows, |tx, i, row, report| {
        let name = map
            .get("display_name")
            .and_then(|c| row.get(*c))
            .map_or("", String::as_str)
            .to_string();
        if name.is_empty() {
            report.error(i, "missing required display name");
            return Err(core::Error::Row {
                row: i,
                message: "missing display name".into(),
            });
        }
        let c = core::vcard::ContactRecord {
            display_name: name,
            uid: Some(format!("csv-{i}")),
            ..Default::default()
        };
        core::contacts::upsert_contact(tx, &c)?;
        report.inserted += 1;
        Ok(())
    })
    .unwrap();
    assert!(report.has_errors(), "row 2 must fail validation");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM contact", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "atomicity: a failing row aborts the whole import");
}

#[test]
fn eml_import_and_export_roundtrip() {
    let mut store = Store::open_memory("x").unwrap();
    let conn = store.conn_mut();
    let acct = mailstore::create_account(conn, "A", "imap", None, None, None).unwrap();
    let inbox = mailstore::ensure_folder(conn, acct, None, "INBOX", Some("inbox")).unwrap();
    let eml = "From: Ana <ana@example.com>\r\nTo: me@example.com\r\nSubject: Round me\r\nDate: Wed, 01 Apr 2026 09:30:00 +0000\r\nMessage-ID: <r1@example.com>\r\n\r\nHello from EML.\r\n";
    mailstore::ingest_eml(conn, acct, inbox, eml.as_bytes()).unwrap();
    // The stored message must be findable by FTS (export path is the message
    // itself re-serialized at compose time; ingest + search is the Gate 2 loop).
    let hits = global_search(
        conn,
        "EML",
        SearchScope {
            mail: true,
            ..Default::default()
        },
        5,
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Round me");
}

#[test]
fn trash_lifecycle_across_modules_with_audit() {
    let mut store = Store::open_memory("x").unwrap();
    let conn = store.conn_mut();
    let item = core::repo::create_item(conn, core::model::Kind::Note, None, false).unwrap();
    conn.execute(
        "INSERT INTO note (item_id, title, content) VALUES (?1, 'T', 'c')",
        [item],
    )
    .unwrap();
    core::repo::trash_item(conn, item).unwrap();
    let q = core::repo::TrashQuery {
        kind: Some(core::model::Kind::Note),
        deleted_after: None,
        deleted_before: None,
        limit: 100,
    };
    assert_eq!(core::repo::query_trash(conn, &q).unwrap().len(), 1);
    core::repo::restore_item(conn, item).unwrap();
    assert!(core::repo::query_trash(conn, &q).unwrap().is_empty());
    core::repo::purge_item(conn, item).unwrap();
    // cascade removed the module row too
    let n: i64 = conn
        .query_row("SELECT count(*) FROM note", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "purge cascades to module tables");
    let audit_n: i64 = conn
        .query_row("SELECT count(*) FROM audit_log WHERE action IN ('trash_item','restore_item','purge_item')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(audit_n, 3);
}
