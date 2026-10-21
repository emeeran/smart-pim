//! End-to-end Wi-Fi sync over real TLS on loopback: server endpoint + CLI
//! client paths (pair → bidirectional sync → idempotent re-sync). The
//! Android companion app will implement the client role identically.
//!
//! Connections and verification each open the database files independently
//! (SQLCipher/WAL supports concurrent connections from one process), mirroring
//! how the CLI peer example runs against a live desktop.

use std::sync::{Arc, Mutex};

use pfcore::model::Kind;
use pfcore::store::Store;
use smart_pim_lib::wifisync;

fn store_at(dir: &std::path::Path, name: &str) -> Store {
    Store::open(&dir.join(name), "test-pass").expect("open store")
}

fn add_note(store: &Store, title: &str) {
    let conn = store.conn();
    let item = pfcore::repo::create_item(conn, Kind::Note, None, false).unwrap();
    conn.execute(
        "INSERT INTO note (item_id, title, content) VALUES (?1, ?2, 'body')",
        rusqlite::params![item, title],
    )
    .unwrap();
}

fn note_titled(store: &Store, title: &str) -> bool {
    store
        .conn()
        .query_row("SELECT 1 FROM note WHERE title = ?1", [title], |r| {
            r.get::<_, i64>(0)
        })
        .is_ok()
}

#[test]
fn pair_then_bidirectional_sync_over_tls() {
    let dir = tempfile::tempdir().unwrap();
    let server_store = store_at(dir.path(), "server.db");
    add_note(&server_store, "from-server");

    let shared: Arc<Mutex<Option<Store>>> = Arc::new(Mutex::new(Some(server_store)));
    let server =
        wifisync::WifiServer::start(Arc::clone(&shared), "server-dev-1".into(), "Server".into())
            .expect("server starts");

    // ---- pair with the 6-digit code
    let mut client = store_at(dir.path(), "client.db");
    add_note(&client, "from-client");
    let client_id = wifisync::self_id(client.conn());
    let outcome = wifisync::pair("127.0.0.1", server.port, &server.code, &client_id, "Client")
        .expect("pairing succeeds");
    assert_eq!(outcome.server_id, "server-dev-1");
    wifisync::setting_set(
        client.conn(),
        &wifisync::token_key("server-dev-1"),
        &outcome.token,
    );

    // A wrong code is refused.
    assert!(wifisync::pair("127.0.0.1", server.port, "000000", &client_id, "X").is_err());

    // ---- bidirectional sync: pull the server's note, push ours
    let summary = wifisync::sync_with(&mut client, "127.0.0.1", server.port, "server-dev-1")
        .expect("first sync");
    assert_eq!(summary.pulled, 1, "server note replicated: {summary:?}");
    assert_eq!(summary.pushed, 1, "client note replicated: {summary:?}");

    let client_check = store_at(dir.path(), "client.db");
    assert!(note_titled(&client_check, "from-server"), "pull landed");
    {
        let guard = shared.lock().unwrap();
        let server = guard.as_ref().unwrap();
        assert!(note_titled(server, "from-client"), "push landed");
        let devices = pfcore::sync::devices(server.conn()).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, client_id);
        assert!(devices[0].last_sync.is_some(), "device_touch after session");
    }

    // ---- re-sync is a no-op (cursors advanced on both sides)
    let mut again = store_at(dir.path(), "client.db");
    let summary2 =
        wifisync::sync_with(&mut again, "127.0.0.1", server.port, "server-dev-1").expect("re-sync");
    assert_eq!(summary2.pulled, 0);
    assert_eq!(summary2.pushed, 0);

    server.stop();
}

#[test]
fn sync_refused_with_bad_token() {
    let dir = tempfile::tempdir().unwrap();
    let server_store = store_at(dir.path(), "server.db");
    let shared: Arc<Mutex<Option<Store>>> = Arc::new(Mutex::new(Some(server_store)));
    let server = wifisync::WifiServer::start(shared, "srv".into(), "S".into()).unwrap();
    let mut client = store_at(dir.path(), "client.db");
    wifisync::setting_set(
        client.conn(),
        &wifisync::token_key("srv"),
        "not-a-real-token",
    );
    let err = wifisync::sync_with(&mut client, "127.0.0.1", server.port, "srv")
        .expect_err("bad token refused");
    assert!(err.contains("authentication"), "{err}");
    server.stop();
}
