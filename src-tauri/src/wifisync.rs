//! Wi-Fi device sync (docs/epim_clone_plan.md §D1): mDNS discovery, TLS
//! transport with a self-signed certificate, 6-digit pairing, and a
//! length-prefixed-JSON session protocol driving `pfcore::sync`.
//!
//! Wire contract (client-initiated; a desktop is the server for its own
//! store and the client when talking to a peer):
//!
//! - `hello`   C→S `{device_id, name, platform, token?|pair_code?}`
//! - `welcome` S→C `{ok, device_id (server id), token? (on fresh pairing)}`
//! - `pull`    C→S `{cursor}` → `batch` S→C `{payloads, cursor, stale}`
//! - `snap`    C→S (full re-snapshot after a stale pull) → `batch`
//! - `push`    C→S `{payloads}` → `report` S→C `{applied, skipped, conflicts}`
//! - `bye`     both directions.
//!
//! Auth: the first connection presents the pairing code; both sides derive a
//! token `sha256(secret:device_id)` that authenticates every later session.
//! The certificate is self-signed per server start, so clients must accept
//! it — confidentiality comes from TLS, authentication from the token. Sync
//! is refused while the database is locked (auto-lock keeps working).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use pfcore::store::Store;
use pfcore::sync as engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SERVICE_TYPE: &str = "_smartpim._tcp.local.";
/// Hard cap on one framed message (a full snapshot stays far below).
const MAX_FRAME: u32 = 256 * 1024 * 1024;
const PULL_BATCH: usize = 200;

pub const SELF_ID_KEY: &str = "sync.self_id";
pub const NAME_KEY: &str = "sync.name";
pub fn token_key(server_id: &str) -> String {
    format!("sync.token.{server_id}")
}
pub fn pull_cursor_key(server_id: &str) -> String {
    format!("sync.pull_cursor.{server_id}")
}
pub fn push_cursor_key(server_id: &str) -> String {
    format!("sync.push_cursor.{server_id}")
}

/// Shared server secrets handed to connection threads.
struct Shared {
    secret: String,
    code: String,
    server_id: String,
}

/// A running Wi-Fi sync endpoint: TLS listener + mDNS advertisement.
pub struct WifiServer {
    pub port: u16,
    /// 6-digit pairing code shown to the user (valid while running).
    pub code: String,
    pub device_id: String,
    stop: Arc<AtomicBool>,
    /// `None` when mDNS could not start (headless host); TCP still serves.
    daemon: Option<ServiceDaemon>,
    service_name: String,
}

impl WifiServer {
    /// Bind an ephemeral port, generate the pairing secret and advertise on
    /// the LAN. Connections are served on their own threads until [`stop`].
    ///
    /// # Errors
    /// Text of the transport error when the listener, TLS identity or mDNS
    /// registration fails.
    pub fn start(
        store: Arc<Mutex<Option<Store>>>,
        device_id: String,
        device_name: String,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let mut rng = rand::rng();
        use rand::RngCore;
        let mut rb = || {
            let mut b = [0u8; 1];
            rng.fill_bytes(&mut b);
            b[0]
        };
        let code: String = (0..6).map(|_| char::from(b'0' + rb() % 10)).collect();
        let secret: String = (0..32).map(|_| char::from(b'0' + rb() % 16)).collect();
        let shared = Arc::new(Shared {
            secret,
            code: code.clone(),
            server_id: device_id.clone(),
        });

        let ck = rcgen::generate_simple_self_signed(vec!["smartpim.local".into()])
            .map_err(|e| e.to_string())?;
        let identity = native_tls::Identity::from_pkcs8(
            ck.cert.pem().as_bytes(),
            ck.signing_key.serialize_pem().as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        let acceptor = Arc::new(native_tls::TlsAcceptor::new(identity).map_err(|e| e.to_string())?);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let store = Arc::clone(&store);
                let shared = Arc::clone(&shared);
                let acceptor = Arc::clone(&acceptor);
                std::thread::spawn(move || {
                    let _ = serve_connection(acceptor, stream, store, shared);
                });
            }
        });

        // mDNS is best-effort: headless hosts without multicast still sync
        // by direct host:port (pair/sync_with never touch the daemon).
        let instance = format!("smartpim-{device_id}");
        let service_name = format!("{instance}.{SERVICE_TYPE}");
        let daemon = ServiceDaemon::new().ok().and_then(|daemon| {
            let props: std::collections::HashMap<String, String> = [
                ("id".to_string(), device_id.clone()),
                ("name".to_string(), device_name),
                ("platform".to_string(), "desktop".to_string()),
            ]
            .into_iter()
            .collect();
            let service = ServiceInfo::new(
                SERVICE_TYPE,
                &instance,
                &format!("{instance}.local."),
                "",
                port,
                props,
            )
            .ok()?
            .enable_addr_auto();
            daemon.register(service).ok()?;
            Some(daemon)
        });

        Ok(Self {
            port,
            code,
            device_id,
            stop,
            daemon,
            service_name,
        })
    }

    /// Unadvertise and stop serving.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(daemon) = &self.daemon {
            let _ = daemon.unregister(&self.service_name);
        }
    }
}

/// Stable identity of this install (generated once, stored in the DB).
pub fn self_id(conn: &rusqlite::Connection) -> String {
    if let Some(id) = setting_get_pub(conn, SELF_ID_KEY) {
        return id;
    }
    let mut rng = rand::rng();
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    let id: String = bytes.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    });
    setting_set(conn, SELF_ID_KEY, &id);
    id
}

/// Read a sync-related setting row.
pub fn setting_get_pub(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

/// Write a sync-related setting row.
pub fn setting_set(conn: &rusqlite::Connection, key: &str, value: &str) {
    let _ = conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    );
}

/// Run `f` with the unlocked store, or fail the session while locked.
fn with_store<T>(
    store: &Mutex<Option<Store>>,
    f: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = store.lock().map_err(|_| "state poisoned")?;
    let Some(st) = guard.as_mut() else {
        return Err("database is locked".into());
    };
    f(st)
}

/// One server-side session over an accepted connection.
fn serve_connection(
    acceptor: Arc<native_tls::TlsAcceptor>,
    stream: TcpStream,
    store: Arc<Mutex<Option<Store>>>,
    shared: Arc<Shared>,
) -> Result<(), String> {
    let mut stream = acceptor.accept(stream).map_err(|e| e.to_string())?;
    let hello = read_msg(&mut stream)?;
    if hello["msg"].as_str() != Some("hello") {
        return Err("expected hello".into());
    }
    let client_id = hello["device_id"].as_str().unwrap_or_default().to_string();
    if client_id.is_empty() || client_id == shared.server_id {
        send_msg(
            &mut stream,
            &json!({"msg":"welcome","ok":false,"error":"bad device id"}),
        )?;
        return Err("bad device id".into());
    }
    let expected = derive_token(&shared.secret, &client_id);
    let paired = hello["token"].as_str() == Some(expected.as_str());
    let fresh_pair = !paired
        && hello["pair_code"]
            .as_str()
            .is_some_and(|c| constant_eq(c, &shared.code));

    if !paired && !fresh_pair {
        send_msg(
            &mut stream,
            &json!({"msg":"welcome","ok":false,"error":"authentication failed"}),
        )?;
        return Err("authentication failed".into());
    }
    let new_token = fresh_pair.then(|| expected.clone());
    send_msg(
        &mut stream,
        &json!({
            "msg": "welcome",
            "ok": true,
            "device_id": shared.server_id,
            "token": new_token,
        }),
    )?;
    if fresh_pair {
        let name = hello["name"].as_str().unwrap_or("device").to_string();
        let platform = hello["platform"].as_str().unwrap_or("unknown").to_string();
        with_store(&store, |st| {
            engine::device_register(st.conn(), &client_id, &name, &platform)
                .map_err(|e| e.to_string())
        })?;
    }

    loop {
        let msg = read_msg(&mut stream)?;
        match msg["msg"].as_str() {
            Some("pull") => {
                let cursor = msg["cursor"].as_i64().unwrap_or(0);
                let reply = with_store(&store, |st| {
                    let (entries, next) = engine::changes_since(st.conn(), cursor, PULL_BATCH)
                        .map_err(|e| e.to_string())?;
                    let stale = engine::cursor_stale(st.conn(), cursor);
                    Ok(json!({
                        "msg": "batch",
                        "payloads": entries.into_iter().map(|e| e.payload).collect::<Vec<_>>(),
                        "cursor": next,
                        "stale": stale,
                    }))
                })?;
                send_msg(&mut stream, &reply)?;
            }
            Some("snap") => {
                let reply = with_store(&store, |st| {
                    let (payloads, cursor) =
                        engine::snapshot_all(st.conn()).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "msg": "batch",
                        "payloads": payloads,
                        "cursor": cursor,
                        "stale": false,
                    }))
                })?;
                send_msg(&mut stream, &reply)?;
            }
            Some("push") => {
                let payloads: Vec<engine::ChangePayload> =
                    serde_json::from_value(msg["payloads"].clone()).map_err(|e| e.to_string())?;
                let reply = with_store(&store, |st| {
                    let report =
                        engine::apply_batch(st.conn_mut(), &payloads).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "msg": "report",
                        "applied": report.applied,
                        "skipped": report.skipped,
                        "conflicts": report.conflicts,
                    }))
                })?;
                send_msg(&mut stream, &reply)?;
            }
            Some("bye") => {
                let _ = with_store(&store, |st| {
                    engine::device_touch(st.conn(), &client_id).map_err(|e| e.to_string())
                });
                send_msg(&mut stream, &json!({"msg":"bye"}))?;
                return Ok(());
            }
            other => {
                send_msg(
                    &mut stream,
                    &json!({"msg":"error","error": format!("unexpected message {other:?}")}),
                )?;
            }
        }
    }
}

/// Outcome of a pairing handshake.
pub struct PairOutcome {
    pub server_id: String,
    pub token: String,
    pub port: u16,
}

/// Pair with a peer's server using its 6-digit code. Stores nothing; the
/// caller persists `token` under [`token_key`] with the peer's id.
///
/// # Errors
/// Text of any transport, TLS, protocol or authentication failure.
pub fn pair(
    host: &str,
    port: u16,
    code: &str,
    device_id: &str,
    name: &str,
) -> Result<PairOutcome, String> {
    let mut stream = connect(host, port)?;
    send_msg(
        &mut stream,
        &json!({"msg":"hello","device_id":device_id,"name":name,"platform":"desktop","pair_code":code}),
    )?;
    let welcome = read_msg(&mut stream)?;
    if welcome["ok"].as_bool() != Some(true) {
        return Err(welcome["error"]
            .as_str()
            .unwrap_or("pairing refused")
            .into());
    }
    let server_id = welcome["device_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let token = welcome["token"].as_str().unwrap_or_default().to_string();
    if server_id.is_empty() || token.is_empty() {
        return Err("peer returned an empty pairing".into());
    }
    Ok(PairOutcome {
        server_id,
        token,
        port,
    })
}

/// Bidirectional sync session with a paired peer. Pulls the peer's changes
/// (full snapshot when the local cursor went stale), pushes ours, persists
/// both cursors per peer.
///
/// # Errors
/// Text of any transport, TLS, protocol or engine failure.
pub fn sync_with(
    store: &mut Store,
    host: &str,
    port: u16,
    server_id: &str,
) -> Result<Summary, String> {
    let (token, self_guid) = {
        let conn = store.conn();
        let token =
            setting_get_pub(conn, &token_key(server_id)).ok_or("not paired with this device")?;
        let id = self_id(conn);
        (token, id)
    };
    let name = setting_get_pub(store.conn(), NAME_KEY).unwrap_or_else(hostname_fallback);
    let mut stream = connect(host, port)?;
    send_msg(
        &mut stream,
        &json!({"msg":"hello","device_id":self_guid,"name":name,"platform":"desktop","token":token}),
    )?;
    let welcome = read_msg(&mut stream)?;
    if welcome["ok"].as_bool() != Some(true) {
        return Err(welcome["error"]
            .as_str()
            .unwrap_or("session refused")
            .into());
    }

    let mut summary = Summary::default();

    // ---- pull phase
    let mut cursor: i64 = setting_get_pub(store.conn(), &pull_cursor_key(server_id))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    loop {
        send_msg(&mut stream, &json!({"msg":"pull","cursor":cursor}))?;
        let batch = read_msg(&mut stream)?;
        if batch["msg"].as_str() == Some("error") {
            return Err(batch["error"].as_str().unwrap_or("pull failed").into());
        }
        let payloads: Vec<engine::ChangePayload> =
            serde_json::from_value(batch["payloads"].clone()).map_err(|e| e.to_string())?;
        if batch["stale"].as_bool() == Some(true) {
            send_msg(&mut stream, &json!({"msg":"snap"}))?;
            let snap = read_msg(&mut stream)?;
            let payloads: Vec<engine::ChangePayload> =
                serde_json::from_value(snap["payloads"].clone()).map_err(|e| e.to_string())?;
            let report =
                engine::apply_batch(store.conn_mut(), &payloads).map_err(|e| e.to_string())?;
            summary.note_apply(&report);
            cursor = snap["cursor"].as_i64().unwrap_or(cursor);
            break; // a fresh snapshot needs no further pulls
        }
        let empty = payloads.is_empty();
        if !empty {
            let report =
                engine::apply_batch(store.conn_mut(), &payloads).map_err(|e| e.to_string())?;
            summary.note_apply(&report);
            cursor = batch["cursor"].as_i64().unwrap_or(cursor);
        }
        if empty {
            break;
        }
    }
    setting_set(
        store.conn(),
        &pull_cursor_key(server_id),
        &cursor.to_string(),
    );

    // ---- push phase
    let mut push_cursor: i64 = setting_get_pub(store.conn(), &push_cursor_key(server_id))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    loop {
        let (entries, next) = engine::changes_since(store.conn(), push_cursor, PULL_BATCH)
            .map_err(|e| e.to_string())?;
        if entries.is_empty() {
            break;
        }
        send_msg(
            &mut stream,
            &json!({"msg":"push","payloads": entries.into_iter().map(|e| e.payload).collect::<Vec<_>>()}),
        )?;
        let report = read_msg(&mut stream)?;
        if report["msg"].as_str() == Some("error") {
            return Err(report["error"].as_str().unwrap_or("push failed").into());
        }
        summary.pushed += report["applied"].as_u64().unwrap_or(0) as usize;
        summary.conflicts += report["conflicts"].as_array().map_or(0, Vec::len);
        push_cursor = next;
    }
    setting_set(
        store.conn(),
        &push_cursor_key(server_id),
        &push_cursor.to_string(),
    );

    send_msg(&mut stream, &json!({"msg":"bye"}))?;
    let _ = read_msg(&mut stream); // peer's bye / connection close
    let _ = engine::device_touch(store.conn(), server_id);
    Ok(summary)
}

/// Totals of one sync session.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Summary {
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
    pub skipped: usize,
}

impl Summary {
    fn note_apply(&mut self, report: &engine::ApplyReport) {
        self.pulled += report.applied;
        self.skipped += report.skipped;
        self.conflicts += report.conflicts.len();
    }
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "desktop".into())
}

/// Client connection: TLS with a self-signed peer certificate (the token is
/// the authenticator).
fn connect(host: &str, port: u16) -> Result<native_tls::TlsStream<TcpStream>, String> {
    let tcp = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| e.to_string())?;
    connector
        .connect("smartpim.local", tcp)
        .map_err(|e| e.to_string())
}

/// A peer found on the LAN.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Discovered {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub port: u16,
    pub host: String,
}

/// Browse the LAN for `smartpim` sync servers for up to `timeout_ms`.
///
/// # Errors
/// Text of the mDNS error when browsing cannot start.
pub fn discover(timeout_ms: u64) -> Result<Vec<Discovered>, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receiver = daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut out = Vec::new();
    while std::time::Instant::now() < deadline {
        match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let props = info.get_properties();
                let host = info
                    .get_addresses()
                    .iter()
                    .next()
                    .map(|ip| ip.to_string())
                    .unwrap_or_default();
                if host.is_empty() {
                    continue;
                }
                out.push(Discovered {
                    device_id: props
                        .get_property_val_str("id")
                        .unwrap_or_default()
                        .to_string(),
                    name: props
                        .get_property_val_str("name")
                        .map_or_else(|| info.get_fullname().to_string(), str::to_string),
                    platform: props
                        .get_property_val_str("platform")
                        .unwrap_or_default()
                        .to_string(),
                    port: info.get_port(),
                    host,
                });
            }
            Ok(ServiceEvent::SearchStarted(_)) | Ok(ServiceEvent::SearchStopped(_)) => {}
            Ok(_) => {}
            Err(mdns_sd::RecvTimeoutError::Timeout) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    Ok(out)
}

/// Constant-time string equality (pairing code comparison).
fn constant_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Session token for a device: `sha256(secret:device_id)` hex.
#[must_use]
pub fn derive_token(secret: &str, device_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(b":");
    h.update(device_id.as_bytes());
    h.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

// ---------------------------------------------------------------- framing

/// Write one length-prefixed JSON frame.
pub fn send_msg(w: &mut impl Write, v: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(v).map_err(|e| e.to_string())?;
    let len = u32::try_from(bytes.len()).map_err(|_| "frame too large".to_string())?;
    w.write_all(&len.to_be_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&bytes).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

/// Read one length-prefixed JSON frame.
pub fn read_msg(r: &mut impl Read) -> Result<Value, String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len);
    if len > MAX_FRAME {
        return Err(format!("frame of {len} bytes exceeds cap"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_roundtrip_and_reject_garbage() {
        let mut buf = Vec::new();
        send_msg(&mut buf, &json!({"msg":"pull","cursor":7})).unwrap();
        let mut slice: &[u8] = &buf;
        let v = read_msg(&mut slice).unwrap();
        assert_eq!(v["msg"], "pull");
        assert_eq!(v["cursor"], 7);
        assert!(slice.is_empty());

        // A claimed length far beyond the cap is rejected without allocating.
        let hostile: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF];
        assert!(read_msg(&mut &hostile[..]).is_err());
    }

    #[test]
    fn token_is_deterministic_and_input_sensitive() {
        assert_eq!(derive_token("s1", "dev"), derive_token("s1", "dev"));
        assert_ne!(derive_token("s1", "dev"), derive_token("s2", "dev"));
        assert_ne!(derive_token("s1", "dev"), derive_token("s1", "dev2"));
        assert_eq!(derive_token("s", "d").len(), 64);
    }

    #[test]
    fn constant_eq_is_exact() {
        assert!(constant_eq("123456", "123456"));
        assert!(!constant_eq("123456", "123457"));
        assert!(!constant_eq("123456", "12345"));
        assert!(!constant_eq("", "x"));
    }

    #[test]
    fn cursor_keys_are_namespaced() {
        assert_eq!(token_key("abc"), "sync.token.abc");
        assert_eq!(pull_cursor_key("abc"), "sync.pull_cursor.abc");
        assert_eq!(push_cursor_key("abc"), "sync.push_cursor.abc");
    }
}
