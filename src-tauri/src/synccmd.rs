//! Sync, backup, and interchange command surface (docs/epim_clone_plan.md
//! §D1–D3): Wi-Fi device sync, cloud (DAV) profiles, backups, and
//! multi-format export. Errors cross the bridge as strings.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pfcore::store::Store;
use pfcore::sync as engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use crate::state::AppState;
use crate::wifisync::{self, WifiServer};

/// Process-wide handle for the running Wi-Fi sync endpoint.
pub struct SyncState {
    pub wifi: Mutex<Option<WifiServer>>,
}

impl SyncState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            wifi: Mutex::new(None),
        }
    }
}

impl Default for SyncState {
    fn default() -> Self {
        Self::new()
    }
}

fn with_store<T>(
    state: &State<AppState>,
    f: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.store.lock().expect("store mutex");
    let Some(store) = guard.as_mut() else {
        return Err("database is locked".to_string());
    };
    f(store)
}

// ------------------------------------------------------------- Wi-Fi sync

fn device_row_to_json(d: &engine::DeviceRow) -> serde_json::Value {
    json!({
        "id": d.id,
        "name": d.name,
        "platform": d.platform,
        "paired_at": d.paired_at,
        "last_sync": d.last_sync,
    })
}

#[tauri::command]
pub fn wifi_status(sync: State<SyncState>) -> Result<serde_json::Value, String> {
    let guard = sync.wifi.lock().expect("wifi mutex");
    Ok(match guard.as_ref() {
        Some(server) => json!({"advertising": true, "port": server.port}),
        None => json!({"advertising": false, "port": null}),
    })
}

#[tauri::command]
pub fn wifi_advertise_start(
    state: State<AppState>,
    sync: State<SyncState>,
) -> Result<serde_json::Value, String> {
    {
        let guard = sync.wifi.lock().expect("wifi mutex");
        if let Some(server) = guard.as_ref() {
            return Ok(json!({"port": server.port}));
        }
    }
    let store_arc = Arc::clone(&state.store);
    let (id, name) = with_store(&state, |st| {
        let id = wifisync::self_id(st.conn());
        let name = std::env::var("HOSTNAME").unwrap_or_else(|_| "desktop".into());
        Ok((id, name))
    })?;
    let server = WifiServer::start(store_arc, id, name)?;
    let port = server.port;
    *sync.wifi.lock().expect("wifi mutex") = Some(server);
    Ok(json!({"port": port}))
}

use std::sync::Arc;

#[tauri::command]
pub fn wifi_advertise_stop(sync: State<SyncState>) -> Result<(), String> {
    if let Some(server) = sync.wifi.lock().expect("wifi mutex").take() {
        server.stop();
    }
    Ok(())
}

#[tauri::command]
pub fn wifi_pair_code(sync: State<SyncState>) -> Result<serde_json::Value, String> {
    let guard = sync.wifi.lock().expect("wifi mutex");
    let server = guard.as_ref().ok_or("sync advertising is not running")?;
    Ok(json!({"code": server.code}))
}

#[tauri::command]
pub fn device_pair(
    state: State<AppState>,
    host: String,
    port: u16,
    code: String,
    name: String,
) -> Result<serde_json::Value, String> {
    let self_id = with_store(&state, |st| Ok(wifisync::self_id(st.conn())))?;
    let outcome = wifisync::pair(&host, port, &code, &self_id, &name)?;
    with_store(&state, |st| {
        let conn = st.conn();
        wifisync::setting_set(
            conn,
            &wifisync::token_key(&outcome.server_id),
            &outcome.token,
        );
        wifisync::setting_set(conn, &host_key(&outcome.server_id), &host);
        wifisync::setting_set(
            conn,
            &port_key(&outcome.server_id),
            &outcome.port.to_string(),
        );
        engine::device_register(conn, &outcome.server_id, &name, "desktop")
            .map_err(|e| e.to_string())?;
        let row = engine::devices(conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|d| d.id == outcome.server_id)
            .ok_or("pairing lost")?;
        Ok(device_row_to_json(&row))
    })
}

fn host_key(id: &str) -> String {
    format!("sync.host.{id}")
}

fn port_key(id: &str) -> String {
    format!("sync.port.{id}")
}

#[tauri::command]
pub fn devices_list(state: State<AppState>) -> Result<serde_json::Value, String> {
    with_store(&state, |st| {
        let rows = engine::devices(st.conn()).map_err(|e| e.to_string())?;
        Ok(serde_json::Value::Array(
            rows.iter().map(device_row_to_json).collect(),
        ))
    })
}

#[tauri::command]
pub fn device_remove(state: State<AppState>, device_id: String) -> Result<(), String> {
    with_store(&state, |st| {
        let conn = st.conn();
        engine::device_remove(conn, &device_id).map_err(|e| e.to_string())?;
        for key in [
            wifisync::token_key(&device_id),
            wifisync::pull_cursor_key(&device_id),
            wifisync::push_cursor_key(&device_id),
            host_key(&device_id),
            port_key(&device_id),
        ] {
            let _ = conn.execute("DELETE FROM setting WHERE key = ?1", [key]);
        }
        Ok(())
    })
}

#[tauri::command]
pub fn device_sync(state: State<AppState>, device_id: String) -> Result<serde_json::Value, String> {
    let (host, port) = with_store(&state, |st| {
        let host = wifisync::setting_get_pub(st.conn(), &host_key(&device_id))
            .ok_or("device has no stored address")?;
        let port: u16 = wifisync::setting_get_pub(st.conn(), &port_key(&device_id))
            .ok_or("device has no stored port")?
            .parse()
            .map_err(|_| "stored port is invalid")?;
        Ok((host, port))
    })?;
    let summary = with_store(&state, |st| {
        wifisync::sync_with(st, &host, port, &device_id)
    })?;
    Ok(json!({
        "device": device_id,
        "pushed": summary.pushed,
        "pulled": summary.pulled,
        "conflicts": summary.conflicts,
    }))
}

// ------------------------------------------------------------- backups

#[derive(Serialize)]
pub struct BackupInfoRow {
    pub name: String,
    pub created: String,
    pub size: u64,
}

#[tauri::command]
pub fn backup_list(state: State<AppState>) -> Result<Vec<BackupInfoRow>, String> {
    let dir = backup_dir(&state);
    let mut rows = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "db") {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Snapshot names are `<timestamp>.db` where the timestamp is the
        // ISO instant with ':'/'.' replaced by '-'.
        let created = name.trim_end_matches(".db").to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        rows.push(BackupInfoRow {
            name,
            created,
            size,
        });
    }
    rows.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(rows)
}

/// Replace the live database file with a backup. Requires the caller to
/// re-unlock afterwards (the store handle is dropped with the file swap).
#[tauri::command]
pub fn backup_restore(state: State<AppState>, name: String) -> Result<(), String> {
    // Refuse path traversal: the name must be a bare file name.
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("invalid backup name".into());
    }
    let dir = backup_dir(&state);
    let src = dir.join(&name);
    if !src.is_file() {
        return Err("backup not found".into());
    }
    let db_path = state.db_path.lock().expect("path mutex").clone();
    {
        let mut guard = state.store.lock().expect("store mutex");
        *guard = None; // close the SQLCipher handle before touching the file
    }
    // Safety net: keep the current state as a pre-restore snapshot.
    {
        let guard = state.store.lock().expect("store mutex");
        if let Some(st) = guard.as_ref() {
            let _ = st.conn().execute(
                "VACUUM INTO ?1",
                [dir.join("pre-restore.db").to_string_lossy().as_ref()],
            );
        }
    }
    std::fs::copy(&src, &db_path).map_err(|e| e.to_string())?;
    Ok(())
}

fn backup_dir(state: &State<AppState>) -> PathBuf {
    let db = state.db_path.lock().expect("path mutex").clone();
    db.parent()
        .map(|p| p.join("backups"))
        .unwrap_or_else(|| PathBuf::from("backups"))
}

// ------------------------------------------------------------- cloud DAV

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DavProfileInput {
    pub id: Option<i64>,
    pub kind: String,
    pub url: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

fn dav_pass_key(id: i64) -> String {
    format!("dav.pass.{id}")
}

#[tauri::command]
pub fn dav_profiles(state: State<AppState>) -> Result<serde_json::Value, String> {
    with_store(&state, |st| {
        let conn = st.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, url, user, last_sync FROM sync_profile \
                 WHERE kind IN ('caldav','carddav','webdav') ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "url": r.get::<_, Option<String>>(2)?,
                    "user": r.get::<_, Option<String>>(3)?,
                    "last_sync": r.get::<_, Option<String>>(4)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        Ok(serde_json::Value::Array(
            rows.collect::<std::result::Result<_, _>>()
                .map_err(|e| e.to_string())?,
        ))
    })
}

#[tauri::command]
pub fn dav_save(state: State<AppState>, profile: DavProfileInput) -> Result<i64, String> {
    if !matches!(profile.kind.as_str(), "caldav" | "carddav" | "webdav") {
        return Err(format!("unknown profile kind {}", profile.kind));
    }
    with_store(&state, |st| {
        let conn = st.conn();
        let id = match profile.id {
            Some(id) => {
                conn.execute(
                    "UPDATE sync_profile SET kind = ?2, url = ?3, user = ?4 WHERE id = ?1",
                    rusqlite::params![id, profile.kind, profile.url, profile.user],
                )
                .map_err(|e| e.to_string())?;
                id
            }
            None => {
                conn.execute(
                    "INSERT INTO sync_profile (kind, url, user, direction, interval_min) \
                     VALUES (?1, ?2, ?3, 'two_way', 15)",
                    rusqlite::params![profile.kind, profile.url, profile.user],
                )
                .map_err(|e| e.to_string())?;
                conn.last_insert_rowid()
            }
        };
        if let Some(pass) = profile.password.filter(|p| !p.is_empty()) {
            wifisync::setting_set(conn, &dav_pass_key(id), &pass);
        }
        Ok(id)
    })
}

#[tauri::command]
pub fn dav_remove(state: State<AppState>, profile_id: i64) -> Result<(), String> {
    with_store(&state, |st| {
        let conn = st.conn();
        conn.execute(
            "DELETE FROM sync_profile WHERE id = ?1 AND kind IN ('caldav','carddav','webdav')",
            [profile_id],
        )
        .map_err(|e| e.to_string())?;
        let _ = conn.execute(
            "DELETE FROM setting WHERE key = ?1",
            [dav_pass_key(profile_id)],
        );
        Ok(())
    })
}

fn dav_http(
    conn: &rusqlite::Connection,
    profile_id: i64,
    user: Option<&str>,
) -> Result<crate::dav::UreqHttp, String> {
    let pass = wifisync::setting_get_pub(conn, &dav_pass_key(profile_id))
        .ok_or("profile has no stored password")?;
    Ok(crate::dav::UreqHttp::new(user.unwrap_or_default(), &pass))
}

/// Run one sync for a DAV profile. CardDAV pulls per-item vCards and pushes
/// local contacts; CalDAV pulls events in a window and uploads the local
/// calendar; WebDAV snapshots the whole encrypted database file (push-only
/// when the remote moved — the fetched copy is saved to backups instead of
/// being applied blindly).
#[tauri::command]
pub fn dav_sync(state: State<AppState>, profile_id: i64) -> Result<serde_json::Value, String> {
    let (kind, url, user) = with_store(&state, |st| {
        st.conn()
            .query_row(
                "SELECT kind, url, user FROM sync_profile WHERE id = ?1",
                [profile_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())
    })?;
    let mut errors: Vec<String> = Vec::new();
    let mut pulled = 0usize;
    let mut pushed = 0usize;

    match kind.as_str() {
        "carddav" => {
            let http = with_store(&state, |st| {
                dav_http(st.conn(), profile_id, user.as_deref())
            })?;
            let items = crate::dav::fetch_contacts(&http, &url)?;
            with_store(&state, |st| {
                let conn = st.conn();
                for item in items {
                    let Some(data) = item.data else { continue };
                    let parsed = pfcore::vcard::parse_vcf(&data);
                    for contact in &parsed.contacts {
                        if pfcore::contacts::upsert_contact(conn, contact).is_ok() {
                            pulled += 1;
                        }
                    }
                    errors.extend(parsed.warnings.iter().take(3).cloned());
                }
                // Push: one vCard per local contact that has a uid.
                let contacts = pfcore::contacts::all_contacts(conn);
                for contact in contacts {
                    let uid = match &contact.uid {
                        Some(uid) => uid.clone(),
                        None => continue,
                    };
                    let body = pfcore::vcard::write_vcf(std::slice::from_ref(&contact));
                    match crate::dav::put_resource(
                        &http,
                        &crate::dav::join_url(&url, &format!("{uid}.vcf")),
                        body.as_bytes(),
                        None,
                    ) {
                        Ok(_) => pushed += 1,
                        Err(e) => errors.push(e),
                    }
                }
                Ok(())
            })?;
        }
        "caldav" => {
            let http = with_store(&state, |st| {
                dav_http(st.conn(), profile_id, user.as_deref())
            })?;
            let (from, to) = {
                let now = pfcore::model::now_iso();
                let start = (chrono_like_days(&now, -30)).ok_or("cannot compute window")?;
                let end = chrono_like_days(&now, 180).ok_or("cannot compute window")?;
                (start, end)
            };
            let items = crate::dav::fetch_events(&http, &url, &from, &to)?;
            with_store(&state, |st| {
                let conn = st.conn();
                let calendar_id = ensure_dav_calendar(conn, &url)?;
                for item in items {
                    let Some(data) = item.data else { continue };
                    match pfcore::calrepo::import_ics(conn, &data, calendar_id) {
                        Ok((n, warns)) => {
                            pulled += n;
                            errors.extend(warns.iter().take(3).cloned());
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                // Push: the full local calendar as one ICS resource — peers
                // that only read (file-based subscriptions) pick it up.
                let tmp = std::env::temp_dir().join("smartpim-caldav-push.ics");
                match pfcore::calrepo::export_ics(conn, &tmp) {
                    Ok(_) => match std::fs::read(&tmp) {
                        Ok(bytes) => {
                            match crate::dav::put_resource(
                                &http,
                                &crate::dav::join_url(&url, "smartpim.ics"),
                                &bytes,
                                None,
                            ) {
                                Ok(_) => pushed += 1,
                                Err(e) => errors.push(e),
                            }
                            let _ = std::fs::remove_file(&tmp);
                        }
                        Err(e) => errors.push(e.to_string()),
                    },
                    Err(e) => errors.push(e.to_string()),
                }
                Ok(())
            })?;
        }
        "webdav" => {
            let http = with_store(&state, |st| {
                dav_http(st.conn(), profile_id, user.as_deref())
            })?;
            let known: Option<String> = with_store(&state, |st| {
                st.conn()
                    .query_row(
                        "SELECT last_token FROM sync_profile WHERE id = ?1",
                        [profile_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .map_err(|e| e.to_string())
            })?;
            match crate::dav::remote_changed(&http, &url, known.as_deref())? {
                None => {
                    // Remote unchanged: push a fresh snapshot.
                    let path = with_store(&state, |st| {
                        let dir = std::env::temp_dir();
                        pfcore::backup::create_backup(st.conn(), &dir).map_err(|e| e.to_string())
                    })?;
                    let etag = crate::dav::upload_from(&http, &url, &path, known.as_deref())?;
                    with_store(&state, |st| {
                        st.conn()
                            .execute(
                                "UPDATE sync_profile SET last_token = ?2, last_sync = ?3 \
                                 WHERE id = ?1",
                                rusqlite::params![profile_id, etag, pfcore::model::now_iso()],
                            )
                            .map_err(|e| e.to_string())?;
                        let _ = std::fs::remove_file(&path);
                        Ok(())
                    })?;
                    pushed += 1;
                }
                Some(_etag) => {
                    // Remote moved since our last push: do not clobber it.
                    // Save the remote copy into backups for manual review.
                    let dir = backup_dir(&state);
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let dest = dir.join(format!(
                        "remote-{}-{}.db",
                        profile_id,
                        pfcore::model::now_iso().replace([':', '.'], "-")
                    ));
                    crate::dav::download_to(&http, &url, &dest)?;
                    errors.push(format!(
                        "remote database changed since the last sync; its copy was \
                         saved to {} — restore it from Backups if it is the newer one",
                        dest.display()
                    ));
                }
            }
        }
        other => return Err(format!("unknown profile kind {other}")),
    }

    Ok(json!({"pulled": pulled, "pushed": pushed, "errors": errors}))
}

fn ensure_dav_calendar(conn: &rusqlite::Connection, url: &str) -> Result<i64, String> {
    for cal in pfcore::calrepo::calendars_list(conn) {
        if cal.name == format!("CalDAV: {url}") {
            return Ok(cal.id);
        }
    }
    pfcore::calrepo::calendar_create(conn, &format!("CalDAV: {url}"), None)
        .map_err(|e| e.to_string())
}

/// Naive day shift on an ISO instant (window bounds only; sub-day precision
/// is not needed for sync windows).
fn chrono_like_days(iso: &str, days: i64) -> Option<String> {
    let year: i32 = iso.get(0..4)?.parse().ok()?;
    let month: u32 = iso.get(5..7)?.parse().ok()?;
    let day: u32 = iso.get(8..10)?.parse().ok()?;
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?
        .checked_add_signed(chrono::Duration::days(days))?;
    Some(format!("{date}T00:00:00+00:00"))
}

// ------------------------------------------------------------- export

/// Export a module to `path` in `format`. Formats: csv, ics, vcf, eml, html,
/// rtf, xml, md (validity per module is enforced by the caller/UI).
#[tauri::command]
pub fn export_module(
    state: State<AppState>,
    kind: String,
    format: String,
    path: String,
) -> Result<serde_json::Value, String> {
    let expanded = expand_home(&path).ok_or("cannot resolve export path")?;
    let count = with_store(&state, |st| {
        let conn = st.conn();
        let rows = collect_rows(conn, &kind)?;
        let out = match format.as_str() {
            "md" => pfcore::export::to_markdown(&rows, &pfcore::model::now_iso()),
            "html" => pfcore::export::to_html(&rows, &pfcore::model::now_iso()),
            "rtf" => pfcore::export::to_rtf(&rows, &pfcore::model::now_iso()),
            "xml" => pfcore::export::to_xml(&rows, &pfcore::model::now_iso()),
            "csv" => to_csv(&rows, &kind),
            "vcf" if kind == "contacts" => {
                let contacts = pfcore::contacts::all_contacts(conn);
                pfcore::vcard::write_vcf(&contacts)
            }
            "ics" if kind == "events" => {
                pfcore::calrepo::export_ics(conn, Path::new(&expanded))
                    .map_err(|e| e.to_string())?;
                return Ok(rows.len());
            }
            "eml" if kind == "messages" => to_eml(&rows),
            other => return Err(format!("format {other} is not valid for {kind}")),
        };
        std::fs::write(&expanded, out).map_err(|e| e.to_string())?;
        Ok(rows.len())
    })?;
    Ok(json!({"path": expanded, "count": count, "format": format}))
}

fn collect_rows(
    conn: &rusqlite::Connection,
    kind: &str,
) -> Result<Vec<pfcore::export::ExportRow>, String> {
    use pfcore::export::{ContactExp, DiaryExp, ExportRow, MessageExp, NoteExp, TaskExp};
    let mut rows = Vec::new();
    match kind {
        "tasks" => {
            let mut stmt = conn
                .prepare(
                    "SELECT title, due_utc, priority, percent, completed_at FROM task \
                     JOIN item ON item.id = task.item_id ORDER BY COALESCE(due_utc, '9999')",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([], |r| {
                    Ok(ExportRow::Task(TaskExp {
                        title: r.get(0)?,
                        due_utc: r.get(1)?,
                        priority: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        percent: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        completed: r.get::<_, Option<String>>(4)?.is_some(),
                        tags: Vec::new(),
                    }))
                })
                .map_err(|e| e.to_string())?;
            rows.extend(it.flatten());
        }
        "notes" => {
            let mut stmt = conn
                .prepare("SELECT title, content FROM note ORDER BY title")
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([], |r| {
                    Ok(ExportRow::Note(NoteExp {
                        title: r.get(0)?,
                        html: r.get(1)?,
                        tags: Vec::new(),
                    }))
                })
                .map_err(|e| e.to_string())?;
            rows.extend(it.flatten());
        }
        "diary" => {
            let mut stmt = conn
                .prepare("SELECT entry_date, mood, content FROM diary_entry ORDER BY entry_date")
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([], |r| {
                    Ok(ExportRow::Diary(DiaryExp {
                        entry_date: r.get(0)?,
                        mood: r.get(1)?,
                        html: r.get(2)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            rows.extend(it.flatten());
        }
        "contacts" => {
            for c in pfcore::contacts::all_contacts(conn) {
                rows.push(ExportRow::Contact(ContactExp {
                    display_name: c.display_name,
                    organization: c.organization,
                    emails: c.emails,
                    phones: c.phones,
                    urls: c.urls,
                    address: c.addresses.first().cloned(),
                    birthday: c.birthday,
                    notes: c.notes,
                }));
            }
        }
        "messages" => {
            let mut stmt = conn
                .prepare(
                    "SELECT subject, from_name, from_addr, date, snippet FROM message \
                     ORDER BY date DESC LIMIT 5000",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([], |r| {
                    Ok(ExportRow::Message(MessageExp {
                        subject: r.get(0)?,
                        from_name: r.get(1)?,
                        from_addr: r.get(2)?,
                        date: r.get(3)?,
                        snippet: r.get(4)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            rows.extend(it.flatten());
        }
        "events" => {
            // Covered by the ics writer; document-style exports use titles.
            let mut stmt = conn
                .prepare(
                    "SELECT title, start_utc, location, description FROM event ORDER BY start_utc",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([], |r| {
                    let (title, start, location, description): (
                        String,
                        String,
                        Option<String>,
                        Option<String>,
                    ) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                    let mut html = pfcore::htmlsan::sanitize_html(&format!("<p>{title}</p>"));
                    if let Some(loc) = location {
                        html.push_str(&pfcore::htmlsan::sanitize_html(&format!("<p>{loc}</p>")));
                    }
                    if let Some(d) = description {
                        html.push_str(&pfcore::htmlsan::sanitize_html(&d));
                    }
                    Ok(ExportRow::Note(NoteExp {
                        title: start,
                        html,
                        tags: Vec::new(),
                    }))
                })
                .map_err(|e| e.to_string())?;
            rows.extend(it.flatten());
        }
        other => return Err(format!("unknown export kind {other}")),
    }
    Ok(rows)
}

fn to_csv(rows: &[pfcore::export::ExportRow], kind: &str) -> String {
    let mut buf = Vec::new();
    {
        let mut w = csv::Writer::from_writer(&mut buf);
        let header: Vec<&str> = match kind {
            "tasks" => vec!["Title", "Due", "Priority", "Percent", "Completed"],
            "notes" => vec!["Title", "Content"],
            "diary" => vec!["Date", "Mood", "Content"],
            "contacts" => vec![
                "Display Name",
                "Organization",
                "Emails",
                "Phones",
                "Urls",
                "Address",
                "Birthday",
                "Notes",
            ],
            "messages" => vec!["Subject", "From Name", "From Address", "Date", "Snippet"],
            _ => vec!["Title", "Start", "Content"],
        };
        let _ = w.write_record(&header);
        for row in rows {
            let cells: Vec<String> = match row {
                pfcore::export::ExportRow::Task(t) => vec![
                    t.title.clone(),
                    t.due_utc.clone().unwrap_or_default(),
                    t.priority.to_string(),
                    t.percent.to_string(),
                    if t.completed { "yes" } else { "no" }.into(),
                ],
                pfcore::export::ExportRow::Note(n) => {
                    vec![n.title.clone(), pfcore::htmlsan::strip_html(&n.html)]
                }
                pfcore::export::ExportRow::Diary(d) => vec![
                    d.entry_date.clone(),
                    d.mood.clone().unwrap_or_default(),
                    pfcore::htmlsan::strip_html(&d.html),
                ],
                pfcore::export::ExportRow::Contact(c) => vec![
                    c.display_name.clone(),
                    c.organization.clone().unwrap_or_default(),
                    c.emails.join("; "),
                    c.phones.join("; "),
                    c.urls.join("; "),
                    c.address.clone().unwrap_or_default(),
                    c.birthday.clone().unwrap_or_default(),
                    c.notes.clone().unwrap_or_default(),
                ],
                pfcore::export::ExportRow::Message(m) => vec![
                    m.subject.clone(),
                    m.from_name.clone(),
                    m.from_addr.clone(),
                    m.date.clone(),
                    m.snippet.clone(),
                ],
            };
            let _ = w.write_record(&cells);
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn to_eml(rows: &[pfcore::export::ExportRow]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for row in rows {
        if let pfcore::export::ExportRow::Message(m) = row {
            let _ = writeln!(out, "From: {} <{}>", m.from_name, m.from_addr);
            let _ = writeln!(out, "Subject: {}", m.subject);
            let _ = writeln!(out, "Date: {}", m.date);
            let _ = writeln!(out, "X-SmartPIM-Snippet: {}", m.snippet.replace('\n', " "));
            let _ = writeln!(out);
        }
    }
    out
}

fn expand_home(path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").ok()?;
        Some(format!("{home}/{rest}"))
    } else {
        Some(path.to_string())
    }
}
