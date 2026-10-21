//! Tauri commands for the Google Calendar/Tasks link on a mail account:
//! browser-based OAuth connect, manual sync, status, disconnect.
//!
//! Everything blocking (loopback wait, network) runs on OS threads — the
//! async runtime / UI thread is never blocked (same rule as `mail_sync`),
//! and the store lock is held only around database work, never across
//! network calls. Refresh tokens live only in the SQLCipher `setting`
//! table; access tokens are cached in memory until expiry, never persisted.

use std::time::{Duration, Instant};

use rand::RngCore;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::google::{
    auth_url, exchange_code, loopback_wait, open_browser, refresh_token, token_email,
    BearerHttp, GoogleRest,
};
use crate::state::AppState;

const CLIENT_ID_KEY: &str = "google.client_id";
const CLIENT_SECRET_KEY: &str = "google.client_secret";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);

fn refresh_key(account_id: i64) -> String {
    format!("google.refresh.{account_id}")
}

fn get_setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    crate::wifisync::setting_get_pub(conn, key)
}

/// A short read-only view of the link for the UI.
#[derive(serde::Serialize)]
pub struct GoogleStatus {
    pub linked: bool,
    pub google_email: Option<String>,
    pub status: Option<String>,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
    /// Whether the client id/secret are configured in Settings.
    pub configured: bool,
}

#[tauri::command(async)]
pub fn google_status(state: State<AppState>, account_id: i64) -> Result<GoogleStatus, String> {
    let guard = state.store.lock().map_err(|_| "state poisoned")?;
    let store = guard.as_ref().ok_or("locked")?;
    let conn = store.conn();
    let (google_email, status, last_sync) = match pfcore::gsync::GoogleLink::load(conn, account_id)
    {
        Ok(Some(link)) => (Some(link.google_email), Some(link.status), link.last_sync),
        _ => (None, None, None),
    };
    let last_error: Option<String> = conn
        .query_row(
            "SELECT last_error FROM google_link WHERE mail_account_id = ?1",
            [account_id],
            |r| r.get(0),
        )
        .ok();
    let configured = get_setting(conn, CLIENT_ID_KEY).is_some()
        && get_setting(conn, CLIENT_SECRET_KEY).is_some();
    Ok(GoogleStatus {
        linked: google_email.is_some(),
        google_email,
        status,
        last_sync,
        last_error,
        configured,
    })
}

/// Start the browser consent flow. Returns immediately; the result arrives
/// via `google://connected` or `google://auth_error`.
#[tauri::command(async)]
pub fn google_connect(
    app: AppHandle,
    state: State<AppState>,
    account_id: i64,
) -> Result<(), String> {
    let (client_id, client_secret, refresh_existing) = {
        let guard = state.store.lock().map_err(|_| "state poisoned")?;
        let store = guard.as_ref().ok_or("locked")?;
        let conn = store.conn();
        let missing = "Set the Google client ID and secret in Settings first";
        let cid = get_setting(conn, CLIENT_ID_KEY).ok_or(missing)?;
        let cs = get_setting(conn, CLIENT_SECRET_KEY).ok_or(missing)?;
        (cid, cs, get_setting(conn, &refresh_key(account_id)))
    };
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("loopback bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut secret = [0u8; 16];
    rand::rng().fill_bytes(&mut secret);
    let state_value: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    let url = auth_url(&client_id, port, &state_value);

    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            open_browser(&url)?;
            let deadline = Instant::now() + CONNECT_TIMEOUT;
            let code = loopback_wait(&listener, &state_value, deadline)?;
            let http = BearerHttp::anonymous();
            let tokens = exchange_code(&http, &client_id, &client_secret, &code, port)?;
            let email = token_email(&http, &tokens.access_token)?;

            let Some(app_state) = app.try_state::<AppState>() else {
                return Err("app state gone".into());
            };
            let mut guard = app_state.store.lock().map_err(|_| "state poisoned")?;
            let Some(store) = guard.as_mut() else {
                return Err("database is locked".into());
            };
            let conn = store.conn_mut();
            let calendar_id = pfcore::events::ensure_calendar(conn, "Google", Some("#1a73e8"))
                .map_err(|e| e.to_string())?;
            // Keep watermarks across reconnects; only identity/status change.
            conn.execute(
                "INSERT INTO google_link (mail_account_id, google_email, calendar_id, \
                 tasklist_id, status) VALUES (?1, ?2, ?3, '@default', 'connected') \
                 ON CONFLICT(mail_account_id) DO UPDATE SET \
                 google_email = excluded.google_email, calendar_id = excluded.calendar_id, \
                 tasklist_id = '@default', status = 'connected', last_error = NULL",
                rusqlite::params![account_id, email, calendar_id],
            )
            .map_err(|e| e.to_string())?;
            conn.execute("UPDATE mail_account SET oauth = 1 WHERE id = ?1", [account_id])
                .map_err(|e| e.to_string())?;
            if let Some(rt) = tokens.refresh_token.or(refresh_existing) {
                crate::wifisync::setting_set(conn, &refresh_key(account_id), &rt);
            }
            drop(guard);

            // First sync right away so the calendar populates while the user
            // watches.
            let sync_error = run_sync(&app_state, account_id).err().map(|e| e.to_string());
            let _ = app.emit(
                "google://connected",
                serde_json::json!({ "account_id": account_id, "email": email,
                    "error": sync_error }),
            );
            Ok(())
        })();
        if let Err(e) = result {
            let _ = app.emit(
                "google://auth_error",
                serde_json::json!({ "account_id": account_id, "error": e }),
            );
        }
    });
    Ok(())
}

/// Manual "Sync now". Returns at once; result via `google://synced`.
#[tauri::command(async)]
pub fn google_sync(app: AppHandle, account_id: i64) -> Result<(), String> {
    std::thread::spawn(move || {
        let error = app
            .try_state::<AppState>()
            .and_then(|s| run_sync(&s, account_id).err())
            .map(|e| e.to_string());
        let _ = app.emit(
            "google://synced",
            serde_json::json!({ "account_id": account_id, "error": error }),
        );
    });
    Ok(())
}

#[tauri::command]
pub fn google_disconnect(state: State<AppState>, account_id: i64) -> Result<(), String> {
    let mut guard = state.store.lock().map_err(|_| "state poisoned")?;
    let Some(store) = guard.as_mut() else {
        return Err("database is locked".into());
    };
    let conn = store.conn_mut();
    conn.execute("DELETE FROM google_link WHERE mail_account_id = ?1", [account_id])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE mail_account SET oauth = 0 WHERE id = ?1", [account_id])
        .map_err(|e| e.to_string())?;
    crate::wifisync::setting_set(conn, &refresh_key(account_id), "");
    Ok(())
}

/// Cheap poller probe: is this mail account Google-linked?
pub fn has_link(state: &AppState, account_id: i64) -> bool {
    let Ok(guard) = state.store.lock() else {
        return false;
    };
    guard
        .as_ref()
        .and_then(|store| pfcore::gsync::GoogleLink::load(store.conn(), account_id).ok())
        .flatten()
        .is_some()
}

/// Cached access token, if present and not near expiry.
fn cached_token(state: &AppState, account_id: i64) -> Option<String> {
    let cache = state.google_tokens.lock().ok()?;
    let (token, until) = cache.get(&account_id)?;
    (*until > Instant::now()).then(|| token.clone())
}

/// Everything needed for a run that can be read with one brief lock.
struct Auth {
    link: pfcore::gsync::GoogleLink,
    client_id: String,
    client_secret: String,
    refresh_stored: Option<String>,
}

fn load_auth(state: &AppState, account_id: i64) -> Result<Auth, String> {
    let guard = state.store.lock().map_err(|_| "state poisoned")?;
    let store = guard.as_ref().ok_or("locked")?;
    let conn = store.conn();
    let missing = "Google client ID/secret not configured (Settings)";
    Ok(Auth {
        link: pfcore::gsync::GoogleLink::load(conn, account_id)
            .map_err(|e| e.to_string())?
            .ok_or("Google account not linked")?,
        client_id: get_setting(conn, CLIENT_ID_KEY).ok_or(missing)?,
        client_secret: get_setting(conn, CLIENT_SECRET_KEY).ok_or(missing)?,
        refresh_stored: get_setting(conn, &refresh_key(account_id)).filter(|s| !s.is_empty()),
    })
}

/// Access token for a run: cache hit, else a network refresh (no locks held).
/// Returns the token plus a rotated refresh token to persist, if Google sent
/// one.
fn obtain_access(
    state: &AppState,
    account_id: i64,
    auth: &Auth,
) -> Result<(String, Option<String>), String> {
    if let Some(token) = cached_token(state, account_id) {
        return Ok((token, None));
    }
    let stored = auth
        .refresh_stored
        .clone()
        .ok_or("Google account not linked (no refresh token)")?;
    let http = BearerHttp::anonymous();
    let tokens = refresh_token(&http, &auth.client_id, &auth.client_secret, &stored)?;
    if let Ok(mut cache) = state.google_tokens.lock() {
        cache.insert(
            account_id,
            (
                tokens.access_token.clone(),
                Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60)),
            ),
        );
    }
    Ok((tokens.access_token, tokens.refresh_token))
}

/// One sync run for a linked account. `invalid_grant` (revoked access) flips
/// the link to `needs_reconnect` so the UI offers a reconnect button.
/// Called from `google_sync`, the connect flow, `mail_sync` and the poller.
///
/// # Errors
/// Lock/unlock failures, missing configuration, transport or engine errors.
pub fn run_sync(state: &AppState, account_id: i64) -> Result<pfcore::gsync::SyncReport, String> {
    let auth = load_auth(state, account_id)?;
    let (token, rotated) = match obtain_access(state, account_id, &auth) {
        Ok(t) => t,
        Err(e) => {
            if e.contains("invalid_grant") {
                if let Ok(mut guard) = state.store.lock() {
                    if let Some(store) = guard.as_mut() {
                        let _ = store.conn_mut().execute(
                            "UPDATE google_link SET status = 'needs_reconnect', last_error = ?2 \
                             WHERE mail_account_id = ?1",
                            rusqlite::params![account_id, e],
                        );
                    }
                }
            }
            return Err(e);
        }
    };

    let result = (|| {
        let mut guard = state.store.lock().map_err(|_| "state poisoned")?;
        let Some(store) = guard.as_mut() else {
            return Err("database is locked".into());
        };
        let conn = store.conn_mut();
        if let Some(rt) = &rotated {
            // Google rotates refresh tokens occasionally; keep the newest.
            crate::wifisync::setting_set(conn, &refresh_key(account_id), rt);
        }
        pfcore::gsync::sync_account(conn, &auth.link, &GoogleRest::new(&token))
            .map_err(|e| e.to_string())
    })();
    if let Err(e) = &result {
        // A rejected token must not be reused by the next run.
        if let Ok(mut cache) = state.google_tokens.lock() {
            cache.remove(&account_id);
        }
        if e.contains("invalid_grant") {
            if let Ok(mut guard) = state.store.lock() {
                if let Some(store) = guard.as_mut() {
                    let _ = store.conn_mut().execute(
                        "UPDATE google_link SET status = 'needs_reconnect', last_error = ?2 \
                         WHERE mail_account_id = ?1",
                        rusqlite::params![account_id, e],
                    );
                }
            }
        }
    }
    result
}
