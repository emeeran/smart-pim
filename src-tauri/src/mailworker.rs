//! Background mail worker: periodically syncs all IMAP/POP3 accounts and
//! pumps the outbox (S16/S21) while the database is unlocked. Poll cadence
//! comes from the `mail_poll_secs` setting (default 120; 0 disables polling
//! — manual F5 still works through the `mail_sync` command).

use std::time::Duration;

use tauri::Manager;

use crate::googlecmd;
use crate::mailsync;
use crate::state::AppState;

const POLL_KEY: &str = "mail_poll_secs";
const DEFAULT_POLL: u64 = 120;

fn poll_secs(conn: &rusqlite::Connection) -> u64 {
    pfcore_store_settings_get(conn, POLL_KEY)
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_POLL)
}

// Small shim so this module does not depend on command plumbing.
fn pfcore_store_settings_get(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
    .ok()
    .flatten()
}

fn sync_all(state: &AppState) -> usize {
    // Account list first (brief lock); the IMAP sync itself manages its own
    // locking per phase so the UI never starves behind network I/O.
    let accounts: Vec<(i64, String)> = {
        let Ok(guard) = state.store.lock() else {
            return 0;
        };
        let Some(store) = guard.as_ref() else {
            return 0;
        };
        let conn = store.conn();
        conn.prepare("SELECT id, kind FROM mail_account WHERE host IS NOT NULL")
            .and_then(|mut s| {
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    };
    let mut failures = 0usize;
    for &(account, ref kind) in &accounts {
        let result = if kind == "pop3" {
            // POP3 maildrops are small; lock-held sync is acceptable.
            let Ok(mut guard) = state.store.lock() else {
                continue;
            };
            match guard.as_mut() {
                Some(store) => mailsync::sync_pop3_account(store.conn_mut(), account).map(|_| 1),
                None => continue,
            }
        } else {
            mailsync::sync_imap_account(state, account).map(|r| r.len())
        };
        if result.is_err() {
            failures += 1;
        }
    }
    // Linked Google identities ride the same cadence: Calendar + Tasks
    // two-way sync (run_sync holds the store lock only around DB work).
    for &(account, _) in &accounts {
        if googlecmd::has_link(state, account) && googlecmd::run_sync(state, account).is_err() {
            failures += 1;
        }
    }
    // Outbox pump (brief lock).
    let Ok(mut guard) = state.store.lock() else {
        return failures;
    };
    if let Some(store) = guard.as_mut() {
        let _ = mailsync::process_outbox(store.conn_mut(), 20);
    }
    failures
}

pub fn spawn(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        let Ok(guard) = state.store.lock() else {
            continue;
        };
        let unlocked = guard.is_some();
        let poll = guard.as_ref().map(|s| poll_secs(s.conn())).unwrap_or(0);
        drop(guard);
        if !unlocked || poll == 0 {
            continue;
        }
        let _ = sync_all(&state);
        maybe_backup(&state);
        // Sleep the poll interval (checked again each 15 s tick via unlock).
        std::thread::sleep(Duration::from_secs(poll.saturating_sub(15)));
    });
}

const BACKUP_KEY: &str = "backup.interval_min";
const BACKUP_DEFAULT: u64 = 1440; // daily; 0 disables
const BACKUP_KEEP: usize = 14;

/// Automatic backup (Pro parity): snapshot on cadence while unlocked, with
/// retention pruning. The last-run stamp lives in `setting`.
fn maybe_backup(state: &tauri::State<'_, AppState>) {
    let Ok(mut guard) = state.store.lock() else {
        return;
    };
    let Some(store) = guard.as_mut() else {
        return;
    };
    let conn = store.conn();
    let interval = pfcore_store_settings_get(conn, BACKUP_KEY)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(BACKUP_DEFAULT);
    if interval == 0 {
        return;
    }
    let now = now_epoch_secs();
    let last = pfcore_store_settings_get(conn, "backup.last_at")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if now.saturating_sub(last) < interval * 60 {
        return;
    }
    let dir = {
        let db = state.db_path.lock().expect("path mutex").clone();
        db.parent()
            .map(|p| p.join("backups"))
            .unwrap_or_else(|| std::path::PathBuf::from("backups"))
    };
    if pfcore::backup::create_backup(conn, &dir).is_ok() {
        let _ = pfcore_store_settings_set(conn, "backup.last_at", &now.to_string());
        let _ = pfcore::backup::prune_backups(&dir, BACKUP_KEEP);
    }
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Setting writes through the same shim as reads.
fn pfcore_store_settings_set(
    conn: &rusqlite::Connection,
    key: &str,
    value: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
}
