//! Managed application state: the optional (locked-out) store handle, the
//! database path, and the idle clock used by the auto-lock timer (S7).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use pfcore::Store;

pub struct AppState {
    /// `None` while locked or before first unlock. Dropping the `Store`
    /// closes the SQLCipher connection — lock is genuine, not cosmetic.
    /// `Arc` so the Wi-Fi sync server threads can hold it.
    pub store: std::sync::Arc<Mutex<Option<Store>>>,
    pub db_path: Mutex<PathBuf>,
    pub last_activity: Mutex<Instant>,
    /// Cooperative cancellation flags for running AI jobs.
    pub ai_cancel: crate::aiworker::Cancel,
}

impl AppState {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            store: std::sync::Arc::new(Mutex::new(None)),
            db_path: Mutex::new(db_path),
            last_activity: Mutex::new(Instant::now()),
            ai_cancel: crate::aiworker::Cancel::default(),
        }
    }

    /// Database file location: `--portable` keeps everything beside the
    /// binary (USB mode, spec S9); otherwise the OS app-data dir.
    pub fn resolve_db_path(app: &tauri::AppHandle) -> PathBuf {
        if std::env::args().any(|a| a == "--portable") {
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    return dir.join("data").join("smartpim.db");
                }
            }
        }
        use tauri::Manager as _;
        app.path()
            .app_data_dir()
            .map(|d| d.join("smartpim.db"))
            .unwrap_or_else(|_| PathBuf::from("smartpim.db"))
    }
}

/// Serialize the idle deadline for tests and the auto-lock thread:
/// `auto_lock_minutes` of 0 means "never lock".
pub fn idle_exceeded(last: Instant, auto_lock_minutes: i64) -> bool {
    auto_lock_minutes > 0
        && last.elapsed() >= std::time::Duration::from_secs(auto_lock_minutes as u64 * 60)
}

pub const AUTO_LOCK_KEY: &str = "auto_lock_minutes";
pub const AUTO_LOCK_DEFAULT: i64 = 15;
