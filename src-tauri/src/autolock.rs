//! Auto-lock timer (S7): polls idle time every 5 s; on expiry drops the store
//! (closing the SQLCipher connection) and emits `session:locked`.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands;
use crate::state::{idle_exceeded, AppState, AUTO_LOCK_DEFAULT};
use crate::EVT_LOCKED;

const POLL: Duration = Duration::from_secs(5);

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        let Ok(guard) = state.store.lock() else {
            continue;
        };
        if guard.is_none() {
            continue; // already locked
        }
        let minutes = guard
            .as_ref()
            .map(|s| commands::auto_lock_minutes(s.conn()))
            .unwrap_or(AUTO_LOCK_DEFAULT);
        let idle = *state.last_activity.lock().expect("idle mutex");
        if !idle_exceeded(idle, minutes) {
            continue;
        }
        drop(guard);
        if let Ok(mut slot) = state.store.lock() {
            *slot = None; // closes the encrypted connection
        }
        let _ = app.emit(EVT_LOCKED, ());
    });
}
