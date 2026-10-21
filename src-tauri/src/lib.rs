//! Smart PIM Tauri shell: window, IPC commands, auto-lock timer.

mod ai;
mod aicmd;
mod aiworker;
mod autolock;
mod commands;
mod dav;
mod google;
mod googlecmd;
mod import_csv;
pub mod mailsync;
mod mailworker;
mod state;
mod synccmd;
pub mod wifisync;

use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::googlecmd::{google_connect, google_disconnect, google_status, google_sync};
use crate::state::AppState;

/// Sentinel event names emitted to the UI.
pub const EVT_LOCKED: &str = "session:locked";

fn with_conn<T>(
    state: &State<AppState>,
    f: impl FnOnce(&rusqlite::Connection) -> pfcore::Result<T>,
) -> Result<T, String> {
    let guard = state.store.lock().expect("store mutex");
    let Some(store) = guard.as_ref() else {
        return Err("locked".into());
    };
    f(store.conn()).map_err(|e| e.to_string())
}

fn touch(state: &State<AppState>) {
    *state.last_activity.lock().expect("idle mutex") = Instant::now();
}

// ------------------------------------------------------------------ commands

#[tauri::command(async)]
fn db_status(state: State<AppState>) -> Result<serde_json::Value, String> {
    let path = state
        .db_path
        .lock()
        .expect("path mutex")
        .display()
        .to_string();
    let locked = state.store.lock().expect("store mutex").is_none();
    Ok(serde_json::json!({
        "locked": locked,
        "db_path": path,
        "exists": std::path::Path::new(&path).exists(),
        "title": format!("{path} — Smart PIM"),
    }))
}

#[tauri::command(async)]
fn unlock(
    app: AppHandle,
    state: State<AppState>,
    passphrase: String,
) -> Result<commands::UnlockOutcome, String> {
    let path = state.db_path.lock().expect("path mutex").clone();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let result = commands::unlock_db(&path, &passphrase);
    let outcome = result.outcome();
    if let Some(store) = result.take_store() {
        *state.store.lock().expect("store mutex") = Some(store);
        touch(&state);
        if let Some(win) = app.get_webview_window("main") {
            let title = path.display().to_string();
            let _ = win.set_title(&format!("{title} — Smart PIM"));
        }
        let _ = app.emit("session:unlocked", ());
    }
    Ok(outcome)
}

#[tauri::command(async)]
fn lock(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    // Dropping the Store closes the encrypted connection.
    *state.store.lock().expect("store mutex") = None;
    let _ = app.emit(EVT_LOCKED, ());
    Ok(())
}

#[tauri::command(async)]
fn heartbeat(state: State<AppState>) -> Result<(), String> {
    touch(&state);
    Ok(())
}

#[tauri::command(async)]
fn counts(state: State<AppState>) -> Result<commands::Counts, String> {
    with_conn(&state, commands::shell_counts)
}

#[tauri::command(async)]
fn global_search(
    state: State<AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<pfcore::search::SearchHit>, String> {
    with_conn(&state, |c| {
        commands::do_global_search(c, &query, limit.unwrap_or(50))
    })
}

#[tauri::command(async)]
fn messages_page(
    state: State<AppState>,
    folder_id: i64,
    cursor_date: Option<String>,
    cursor_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::pagination::MessagePageRow>, String> {
    with_conn(&state, |c| {
        commands::do_messages_page(c, folder_id, cursor_date, cursor_id, limit.unwrap_or(100))
    })
}

#[tauri::command(async)]
fn tasks_page(
    state: State<AppState>,
    after_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::pagination::TaskPageRow>, String> {
    with_conn(&state, |c| {
        commands::do_tasks_page(c, after_id, limit.unwrap_or(100))
    })
}

#[tauri::command(async)]
fn trash(
    state: State<AppState>,
    kind: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::model::Item>, String> {
    with_conn(&state, |c| {
        commands::do_trash(c, kind, limit.unwrap_or(200))
    })
}

#[tauri::command(async)]
fn trash_restore(state: State<AppState>, item_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::repo::restore_item(c, item_id))
}

#[tauri::command(async)]
fn trash_purge(state: State<AppState>, item_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::repo::purge_item(c, item_id))
}

#[tauri::command(async)]
fn undo(state: State<AppState>) -> Result<Option<pfcore::repo::UndoOutcome>, String> {
    with_conn(&state, commands::do_undo)
}

#[tauri::command(async)]
fn item_links(state: State<AppState>, item_id: i64) -> Result<Vec<commands::LinkedItem>, String> {
    with_conn(&state, |c| commands::do_item_links(c, item_id))
}

/// Cross-module related items: for contacts this includes their mail,
/// assigned tasks and events; for everything else the explicit link graph.
/// `id` is the module-row id of `kind` (note id, task id, …).
#[tauri::command(async)]
fn related_items(
    state: State<AppState>,
    kind: String,
    id: i64,
) -> Result<Vec<pfcore::related::RelatedItem>, String> {
    with_conn(&state, |c| {
        if kind == "contact" {
            return Ok(pfcore::related::for_contact(c, id));
        }
        let table = match kind.as_str() {
            "note" => "note",
            "task" => "task",
            "diary" => "diary_entry",
            "event" => "event",
            _ => {
                return Err(pfcore::Error::Parse {
                    what: "related items",
                    message: format!("no related-items source for {kind}"),
                })
            }
        };
        let item_id: i64 = c
            .query_row(
                &format!("SELECT item_id FROM {table} WHERE id = ?1"),
                [id],
                |r| r.get(0),
            )
            .map_err(|_| pfcore::Error::NotFound(id))?;
        Ok(pfcore::related::for_item(c, item_id))
    })
}

/// Today landing snapshot: agenda, due/overdue tasks, per-account unread,
/// upcoming anniversaries. The shell passes its local date (YYYY-MM-DD).
#[tauri::command(async)]
fn today_dashboard(
    state: State<AppState>,
    today: String,
) -> Result<pfcore::related::TodayDashboard, String> {
    with_conn(&state, |c| pfcore::related::today_dashboard(c, &today))
}

#[tauri::command(async)]
fn item_link(state: State<AppState>, item_id: i64, other_id: i64) -> Result<(), String> {
    with_conn(&state, |c| commands::do_item_link(c, item_id, other_id))
}

#[tauri::command(async)]
fn item_unlink(state: State<AppState>, item_id: i64, other_id: i64) -> Result<(), String> {
    with_conn(&state, |c| commands::do_item_unlink(c, item_id, other_id))
}

#[tauri::command(async)]
fn settings_get(state: State<AppState>, key: String) -> Result<Option<String>, String> {
    with_conn(&state, |c| Ok(commands::settings_get(c, &key)))
}

#[tauri::command(async)]
fn settings_set(state: State<AppState>, key: String, value: String) -> Result<(), String> {
    with_conn(&state, |c| commands::settings_set(c, &key, &value))
}

/// Open the provider's app-password page in the system browser. The URL is
/// NOT taken from the caller — the command maps a fixed provider id to a
/// hardcoded URL, so nothing on the webview side can open an arbitrary
/// address.
#[tauri::command(async)]
fn open_provider_help(provider: String) -> Result<(), String> {
    const URLS: &[(&str, &str)] = &[
        ("gmail", "https://myaccount.google.com/apppasswords"),
        ("outlook", "https://account.microsoft.com/security"),
        ("yahoo", "https://login.yahoo.com/account/security"),
        ("icloud", "https://account.apple.com/account/manage"),
        ("fastmail", "https://app.fastmail.com/settings/security"),
        ("zoho", "https://accounts.zoho.com/home#security"),
    ];
    let url = URLS
        .iter()
        .find(|(id, _)| *id == provider)
        .map(|(_, url)| url)
        .ok_or_else(|| format!("unknown provider {provider}"))?;
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();
    result
        .map(|_| ())
        .map_err(|e| format!("could not open browser: {e}"))
}

#[tauri::command(async)]
fn backup_now(state: State<AppState>) -> Result<String, String> {
    with_conn(&state, |c| {
        let dir = backup_dir(&state);
        std::fs::create_dir_all(&dir).map_err(pfcore::Error::from)?;
        let path = pfcore::backup::create_backup(c, &dir)?;
        Ok(path.display().to_string())
    })
}

fn backup_dir(state: &State<AppState>) -> std::path::PathBuf {
    let db = state.db_path.lock().expect("path mutex").clone();
    db.parent()
        .map(|p| p.join("backups"))
        .unwrap_or_else(|| std::path::PathBuf::from("backups"))
}

/// Manual sync (Sync All button): returns at once and runs on a background
/// thread, reporting via `mail://synced`. Inline syncing blocked the UI for
/// the whole Gmail conversation.
#[tauri::command(async)]
fn mail_sync(app: tauri::AppHandle, state: State<AppState>, account_id: i64) -> Result<(), String> {
    let kind: String = {
        let guard = state.store.lock().expect("store mutex");
        let store = guard.as_ref().ok_or("locked")?;
        store
            .conn()
            .query_row(
                "SELECT kind FROM mail_account WHERE id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?
    };
    std::thread::spawn(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let result = if kind == "pop3" {
            let mut guard = state.store.lock().expect("store mutex");
            match guard.as_mut() {
                Some(store) => mailsync::sync_pop3_account(store.conn_mut(), account_id).map(|r| {
                    serde_json::json!([{ "folder": r.folder, "fetched": r.fetched, "total": r.total }])
                }),
                None => return,
            }
        } else {
            mailsync::sync_imap_account(&state, account_id)
                .map(|r| serde_json::to_value(r).unwrap_or_default())
        };
        let error = result.err().map(|e| e.to_string());
        let _ = app.emit(
            "mail://synced",
            serde_json::json!({ "account_id": account_id, "error": error }),
        );
        // A linked Google identity rides along: Calendar + Tasks two-way.
        if let Some(state) = app.try_state::<AppState>() {
            if let Err(gerr) = googlecmd::run_sync(&state, account_id) {
                let _ = app.emit(
                    "google://synced",
                    serde_json::json!({ "account_id": account_id, "error": gerr }),
                );
            }
        }
    });
    Ok(())
}

#[tauri::command(async)]
fn outbox_process(state: State<AppState>) -> Result<serde_json::Value, String> {
    let mut guard = state.store.lock().expect("store mutex");
    let store = guard.as_mut().ok_or("locked")?;
    let (sent, failed) =
        mailsync::process_outbox(store.conn_mut(), 20).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "sent": sent, "failed": failed }))
}

#[tauri::command(async)]
fn mail_accounts(state: State<AppState>) -> Result<Vec<pfcore::mailstore::Account>, String> {
    with_conn(&state, |c| Ok(pfcore::mailstore::accounts(c)))
}

#[derive(serde::Deserialize)]
struct NewAccount {
    name: String,
    kind: String, // imap | pop3
    host: String,
    port: i64,
    security: String, // none | starttls | tls
    user: String,
    password: String,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
}

/// Create an account with credentials, then sync it immediately so the
/// wizard can show the folder list (S15). Rolls the row back on failure.
/// Create the account row and return immediately; the initial sync runs in
/// the background (`mail://synced` event reports the outcome). Syncing
/// inline blocked the UI thread for the whole Gmail conversation — the
/// window went "Not Responding" — and the periodic worker would pick the
/// account up anyway.
#[tauri::command(async)]
fn mail_account_create(
    app: tauri::AppHandle,
    state: State<AppState>,
    account: NewAccount,
) -> Result<i64, String> {
    let id = {
        let mut guard = state.store.lock().expect("store mutex");
        let store = guard.as_mut().ok_or("locked")?;
        let conn = store.conn_mut();
        let id = pfcore::mailstore::create_account(
            conn,
            &account.name,
            &account.kind,
            Some(&account.host),
            Some(account.port),
            None,
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE mail_account SET security = ?2, user = ?3, \
             settings = json_set(settings, '$.password', ?4, '$.smtp_host', ?5, '$.smtp_port', ?6) \
             WHERE id = ?1",
            rusqlite::params![
                id,
                account.security,
                account.user,
                account.password,
                account.smtp_host,
                account.smtp_port
            ],
        )
        .map_err(|e| e.to_string())?;
        id
    };
    let kind = account.kind;
    // Blocking network work on its own OS thread — never on the async
    // runtime (a minutes-long Gmail sync would starve it) and never on the
    // UI/command thread.
    std::thread::spawn(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let result = if kind == "pop3" {
            // POP3 maildrops are small; the lock-holding path is fine there.
            let mut guard = state.store.lock().expect("store mutex");
            match guard.as_mut() {
                Some(store) => mailsync::sync_pop3_account(store.conn_mut(), id).map(|r| vec![r]),
                None => return,
            }
        } else {
            mailsync::sync_imap_account(&state, id)
        };
        let error = result.err().map(|e| e.to_string());
        let _ = app.emit(
            "mail://synced",
            serde_json::json!({ "account_id": id, "error": error }),
        );
    });
    Ok(id)
}

#[tauri::command(async)]
fn mail_folders(
    state: State<AppState>,
    account_id: i64,
) -> Result<Vec<pfcore::mailstore::FolderNode>, String> {
    with_conn(&state, |c| {
        Ok(pfcore::mailstore::folders_tree(c, account_id))
    })
}

#[tauri::command(async)]
fn message_full(
    state: State<AppState>,
    message_id: i64,
) -> Result<pfcore::mailstore::MessageDetail, String> {
    with_conn(&state, |c| pfcore::mailstore::message_detail(c, message_id))
}

#[tauri::command(async)]
fn message_mark_read(state: State<AppState>, message_id: i64, seen: bool) -> Result<(), String> {
    with_conn(&state, |c| pfcore::mailstore::set_seen(c, message_id, seen))
}

#[derive(serde::Deserialize)]
struct SendDraft {
    account_id: i64,
    from_name: String,
    from_email: String,
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: String,
    #[serde(default)]
    text_body: String,
    #[serde(default)]
    html_body: String,
    /// RFC 3339; None = send ASAP (queue only when SMTP not configured).
    send_at: Option<String>,
}

/// Queue a draft in the outbox (S20/S21). Returns the outbox row id.
#[tauri::command(async)]
fn mail_queue_draft(state: State<AppState>, draft: SendDraft) -> Result<i64, String> {
    with_conn(&state, |c| {
        let json = serde_json::to_string(&mailsync::Draft {
            account_id: draft.account_id,
            from_name: draft.from_name,
            from_email: draft.from_email,
            to: draft.to,
            cc: draft.cc,
            bcc: draft.bcc,
            subject: draft.subject,
            text_body: draft.text_body,
            html_body: draft.html_body,
        })
        .map_err(pfcore::Error::from)?;
        pfcore::rules::queue_outgoing(c, &json, draft.send_at.as_deref())
    })
}

#[tauri::command(async)]
fn notes_page(
    state: State<AppState>,
    cursor_date: Option<String>,
    cursor_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::notes::NoteRow>, String> {
    with_conn(&state, |c| {
        let after = cursor_date.as_deref().map(|d| (d, cursor_id.unwrap_or(0)));
        pfcore::notes::notes_page(c, after, limit.unwrap_or(200))
    })
}

#[tauri::command(async)]
fn note_get(state: State<AppState>, note_id: i64) -> Result<pfcore::notes::NoteDetail, String> {
    with_conn(&state, |c| pfcore::notes::note_get(c, note_id))
}

#[tauri::command(async)]
fn note_create(state: State<AppState>, title: String, content: String) -> Result<i64, String> {
    with_conn(&state, |c| pfcore::notes::note_create(c, &title, &content))
}

#[tauri::command(async)]
fn note_save(
    state: State<AppState>,
    note_id: i64,
    title: String,
    content: String,
) -> Result<(), String> {
    with_conn(&state, |c| {
        pfcore::notes::note_save(c, note_id, &title, &content)
    })
}

#[tauri::command(async)]
fn note_set_favorite(state: State<AppState>, note_id: i64, favorite: bool) -> Result<(), String> {
    with_conn(&state, |c| {
        pfcore::notes::note_set_favorite(c, note_id, favorite)
    })
}

#[tauri::command(async)]
fn tasks_tree(
    state: State<AppState>,
    include_completed: Option<bool>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::tasks::TaskTreeRow>, String> {
    with_conn(&state, |c| {
        pfcore::tasks::tasks_tree(c, include_completed.unwrap_or(false), limit.unwrap_or(500))
    })
}

#[tauri::command(async)]
fn task_set_parent(
    state: State<AppState>,
    task_id: i64,
    parent_task_id: Option<i64>,
) -> Result<(), String> {
    with_conn(&state, |c| {
        pfcore::tasks::set_task_parent(c, task_id, parent_task_id)
    })
}

#[tauri::command(async)]
fn note_delete(state: State<AppState>, note_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::notes::note_delete(c, note_id))
}

#[tauri::command(async)]
fn diary_entries(
    state: State<AppState>,
    from_date: String,
    to_date: String,
) -> Result<Vec<pfcore::diary::DiaryRow>, String> {
    with_conn(&state, |c| {
        pfcore::diary::diary_range(c, &from_date, &to_date)
    })
}

#[tauri::command(async)]
fn diary_get(
    state: State<AppState>,
    date: String,
) -> Result<Option<pfcore::diary::DiaryRow>, String> {
    with_conn(&state, |c| pfcore::diary::diary_get(c, &date))
}

#[tauri::command(async)]
fn diary_save(
    state: State<AppState>,
    date: String,
    content: String,
    mood: Option<String>,
) -> Result<i64, String> {
    with_conn(&state, |c| {
        pfcore::diary::diary_upsert(c, &date, &content, mood.as_deref())
    })
}

#[tauri::command(async)]
fn diary_delete(state: State<AppState>, date: String) -> Result<(), String> {
    with_conn(&state, |c| pfcore::diary::diary_delete(c, &date))
}

#[tauri::command(async)]
fn diary_months(
    state: State<AppState>,
    year: i32,
) -> Result<Vec<pfcore::diary::DiaryMonth>, String> {
    with_conn(&state, |c| pfcore::diary::diary_months(c, year))
}

#[tauri::command(async)]
fn diary_streak(state: State<AppState>, today: String) -> Result<i64, String> {
    with_conn(&state, |c| pfcore::diary::diary_streak(c, &today))
}

#[tauri::command(async)]
fn calendars_list(state: State<AppState>) -> Result<Vec<pfcore::calrepo::CalendarInfo>, String> {
    with_conn(&state, |c| Ok(pfcore::calrepo::calendars_list(c)))
}

#[tauri::command(async)]
fn calendar_create(
    state: State<AppState>,
    name: String,
    color: Option<String>,
) -> Result<i64, String> {
    with_conn(&state, |c| {
        pfcore::calrepo::calendar_create(c, &name, color.as_deref())
    })
}

#[tauri::command(async)]
fn events_window(
    state: State<AppState>,
    calendar_id: Option<i64>,
    from: String,
    to: String,
) -> Result<Vec<pfcore::calrepo::Occurrence>, String> {
    with_conn(&state, |c| {
        pfcore::calrepo::events_window(c, calendar_id, &from, &to)
    })
}

#[derive(serde::Deserialize)]
struct EventInputMsg {
    id: Option<i64>,
    calendar_id: i64,
    title: String,
    start: String,
    end: Option<String>,
    all_day: bool,
    location: Option<String>,
    description: Option<String>,
    rrule: Option<String>,
    reminder_minutes: Option<i64>,
    #[serde(default)]
    participants: Vec<ParticipantMsg>,
}

#[derive(serde::Deserialize)]
struct ParticipantMsg {
    contact_id: Option<i64>,
    email: String,
}

#[tauri::command(async)]
fn event_save(state: State<AppState>, event: EventInputMsg) -> Result<i64, String> {
    with_conn(&state, |c| {
        pfcore::calrepo::save_event(
            c,
            &pfcore::calrepo::EventInput {
                id: event.id,
                calendar_id: event.calendar_id,
                title: event.title,
                start: event.start,
                end: event.end,
                all_day: event.all_day,
                location: event.location,
                description: event.description,
                rrule: event.rrule,
                reminder_minutes: event.reminder_minutes,
                participants: event
                    .participants
                    .into_iter()
                    .map(|p| pfcore::calrepo::EventParticipantInput {
                        contact_id: p.contact_id,
                        email: p.email,
                    })
                    .collect(),
            },
        )
    })
}

#[tauri::command(async)]
fn event_delete(state: State<AppState>, event_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::calrepo::delete_event(c, event_id))
}

/// Stored attendees of an event (editor loads these).
#[tauri::command(async)]
fn event_participants(
    state: State<AppState>,
    event_id: i64,
) -> Result<Vec<pfcore::calrepo::EventParticipant>, String> {
    with_conn(&state, |c| pfcore::calrepo::event_participants(c, event_id))
}

#[tauri::command(async)]
fn events_import_ics(state: State<AppState>, text: String) -> Result<serde_json::Value, String> {
    with_conn(&state, |c| {
        let cal = pfcore::events::ensure_calendar(c, "Imported", None)?;
        let (imported, warnings) = pfcore::calrepo::import_ics(c, &text, cal)?;
        Ok(serde_json::json!({ "imported": imported, "warnings": warnings }))
    })
}

#[tauri::command(async)]
fn events_export_ics(state: State<AppState>) -> Result<String, String> {
    let path = {
        let db = state.db_path.lock().expect("path mutex").clone();
        let dir = db
            .parent()
            .map(|p| p.join("exports"))
            .unwrap_or_else(|| std::path::PathBuf::from("exports"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        dir.join(format!("calendar-{}.ics", chrono_stamp()))
    };
    with_conn(&state, |c| {
        pfcore::calrepo::export_ics(c, &path)?;
        Ok(path.display().to_string())
    })
}

fn chrono_stamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

#[tauri::command(async)]
fn tasks_open(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::tasks::TaskRow>, String> {
    with_conn(&state, |c| {
        pfcore::tasks::tasks_open(c, limit.unwrap_or(500))
    })
}

#[tauri::command(async)]
fn tasks_completed(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::tasks::TaskRow>, String> {
    with_conn(&state, |c| {
        pfcore::tasks::tasks_completed(c, limit.unwrap_or(100))
    })
}

#[derive(serde::Deserialize)]
struct TaskInputMsg {
    id: Option<i64>,
    title: String,
    notes: Option<String>,
    due_utc: Option<String>,
    priority: i64,
    percent: i64,
}

#[tauri::command(async)]
fn task_save(state: State<AppState>, task: TaskInputMsg) -> Result<i64, String> {
    with_conn(&state, |c| {
        pfcore::tasks::save_task(
            c,
            &pfcore::tasks::TaskInput {
                id: task.id,
                title: task.title,
                notes: task.notes,
                due_utc: task.due_utc,
                priority: task.priority,
                percent: task.percent,
            },
        )
    })
}

#[tauri::command(async)]
fn task_delete(state: State<AppState>, task_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::tasks::delete_task(c, task_id))
}

#[tauri::command(async)]
fn contacts_list(state: State<AppState>) -> Result<Vec<pfcore::vcard::ContactRecord>, String> {
    with_conn(&state, |c| Ok(pfcore::contacts::all_contacts(c)))
}

#[tauri::command(async)]
fn contact_save(
    state: State<AppState>,
    contact: pfcore::vcard::ContactRecord,
) -> Result<i64, String> {
    with_conn(&state, |c| pfcore::contacts::upsert_contact(c, &contact))
}

#[tauri::command(async)]
fn contact_delete(state: State<AppState>, contact_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::contacts::delete_contact(c, contact_id))
}

#[tauri::command(async)]
fn contacts_import_vcf(state: State<AppState>, text: String) -> Result<serde_json::Value, String> {
    with_conn(&state, |c| {
        let parsed = pfcore::vcard::parse_vcf(&text);
        let mut n = 0;
        for contact in &parsed.contacts {
            pfcore::contacts::upsert_contact(c, contact)?;
            n += 1;
        }
        Ok(serde_json::json!({ "imported": n, "warnings": parsed.warnings }))
    })
}

#[tauri::command(async)]
fn contacts_export_vcf(state: State<AppState>) -> Result<String, String> {
    let path = {
        let db = state.db_path.lock().expect("path mutex").clone();
        let dir = db
            .parent()
            .map(|p| p.join("exports"))
            .unwrap_or_else(|| std::path::PathBuf::from("exports"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        dir.join(format!("contacts-{}.vcf", chrono_stamp()))
    };
    with_conn(&state, |c| {
        std::fs::write(
            &path,
            pfcore::vcard::write_vcf(&pfcore::contacts::all_contacts(c)),
        )
        .map_err(pfcore::Error::from)?;
        Ok(path.display().to_string())
    })
}

#[tauri::command(async)]
fn passwords_list(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::pwrepo::PwRow>, String> {
    with_conn(&state, |c| {
        pfcore::pwrepo::list_entries(c, limit.unwrap_or(500))
    })
}

#[derive(serde::Deserialize)]
struct PwInputMsg {
    id: Option<i64>,
    title: String,
    username: Option<String>,
    password: Option<String>,
    url: Option<String>,
    notes: Option<String>,
}

#[tauri::command(async)]
fn password_save(state: State<AppState>, entry: PwInputMsg) -> Result<i64, String> {
    with_conn(&state, |c| {
        pfcore::pwrepo::save_entry(
            c,
            entry.id,
            &entry.title,
            entry.username.as_deref(),
            entry.password.as_deref(),
            entry.url.as_deref(),
            entry.notes.as_deref(),
        )
    })
}

#[tauri::command(async)]
fn password_delete(state: State<AppState>, entry_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::pwrepo::delete_entry(c, entry_id))
}

#[tauri::command(async)]
fn password_generate(length: Option<usize>) -> String {
    pfcore::pwrepo::generate(length.unwrap_or(20))
}

/// Pop due reminders (fires each exactly once; `missed` flags recovered
/// fires from while the app was closed). The UI polls this and notifies.
#[tauri::command(async)]
fn reminders_take(state: State<AppState>) -> Result<Vec<pfcore::ops::DueReminder>, String> {
    with_conn(&state, |c| {
        pfcore::ops::take_due_reminders(c, &pfcore::model::now_iso(), 3600, 10)
    })
}

// ------------------------------------------------------- mail actions (T3.6/T3.9/T3.10/T3.11)

#[tauri::command(async)]
fn message_set_flag(
    state: State<AppState>,
    message_id: i64,
    flag: String,
    value: bool,
) -> Result<(), String> {
    with_conn(&state, |c| {
        pfcore::mailstore::set_flag(c, message_id, &flag, value)
    })
}

#[tauri::command(async)]
fn message_delete(state: State<AppState>, message_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::mailstore::delete_message(c, message_id))
}

#[tauri::command(async)]
fn messages_unified(
    state: State<AppState>,
    cursor_date: Option<String>,
    cursor_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::pagination::MessagePageRow>, String> {
    with_conn(&state, |c| {
        let cursor =
            cursor_date
                .as_deref()
                .zip(cursor_id)
                .map(|(date, id)| pfcore::pagination::Cursor {
                    date: date.to_string(),
                    id,
                });
        pfcore::mailstore::unified_page(c, cursor.as_ref(), limit.unwrap_or(100))
    })
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)] // flat filter surface mirrors S26; a struct would wrap the same 8 IPC fields
fn mail_search(
    state: State<AppState>,
    query: String,
    folder_id: Option<i64>,
    account_id: Option<i64>,
    unread_only: bool,
    flagged_only: bool,
    from: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<pfcore::pagination::MessagePageRow>, String> {
    with_conn(&state, |c| {
        pfcore::mailstore::mail_search(
            c,
            &query,
            &pfcore::mailstore::SearchFilter {
                folder_id,
                account_id,
                unread_only,
                flagged_only,
                from: from.as_deref().unwrap_or(""),
            },
            limit.unwrap_or(50),
        )
    })
}

#[tauri::command(async)]
fn thread_messages(
    state: State<AppState>,
    thread_id: i64,
) -> Result<Vec<pfcore::pagination::MessagePageRow>, String> {
    with_conn(&state, |c| pfcore::mailstore::thread_messages(c, thread_id))
}

#[tauri::command(async)]
fn message_convert(state: State<AppState>, message_id: i64, to: String) -> Result<i64, String> {
    let target = match to.as_str() {
        "task" => pfcore::mailstore::ConvertTarget::Task,
        "note" => pfcore::mailstore::ConvertTarget::Note,
        "event" => pfcore::mailstore::ConvertTarget::Event,
        other => return Err(format!("unknown convert target {other}")),
    };
    with_conn(&state, |c| {
        pfcore::mailstore::convert_message(c, message_id, target).map(|(id, _)| id)
    })
}

// ------------------------------------------------------- CSV import (S13/T3.44)

#[tauri::command(async)]
fn import_csv_preview(
    state: State<AppState>,
    text: String,
) -> Result<import_csv::CsvPreview, String> {
    let _guard = state.store.lock().expect("store mutex");
    import_csv::preview(&text).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn import_csv_validate(
    state: State<AppState>,
    text: String,
    mapping: std::collections::HashMap<String, usize>,
) -> Result<pfcore::import::ImportReport, String> {
    let _guard = state.store.lock().expect("store mutex");
    import_csv::validate(&text, &mapping).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn import_csv_commit(
    state: State<AppState>,
    text: String,
    mapping: std::collections::HashMap<String, usize>,
) -> Result<pfcore::import::ImportReport, String> {
    let mut guard = state.store.lock().expect("store mutex");
    let store = guard.as_mut().ok_or("locked")?;
    import_csv::commit(store.conn_mut(), &text, &mapping).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn rules_list(state: State<AppState>) -> Result<Vec<pfcore::rules::Rule>, String> {
    with_conn(&state, |c| pfcore::rules::rules_of_account(c, None))
}

#[derive(serde::Deserialize)]
struct NewRule {
    account_id: Option<i64>,
    field: String,
    op: String,
    value: String,
    action: String, // move | mark_read | tag
    action_value: Option<String>,
}

#[tauri::command(async)]
fn rule_create(state: State<AppState>, rule: NewRule) -> Result<i64, String> {
    with_conn(&state, |c| {
        let action = match rule.action.as_str() {
            "move" => pfcore::rules::Action::Move {
                folder: rule
                    .action_value
                    .clone()
                    .unwrap_or_else(|| "Archive".into()),
            },
            "mark_read" => pfcore::rules::Action::MarkRead,
            "tag" => pfcore::rules::Action::Tag {
                value: rule.action_value.clone().unwrap_or_default(),
            },
            other => {
                return Err(pfcore::Error::Row {
                    row: 0,
                    message: format!("unknown rule action {other}"),
                })
            }
        };
        pfcore::rules::create_rule(
            c,
            rule.account_id,
            &[pfcore::rules::Condition {
                field: rule.field,
                op: rule.op,
                value: rule.value,
            }],
            &[action],
            0,
        )
    })
}

#[tauri::command(async)]
fn rule_set_enabled(state: State<AppState>, rule_id: i64, enabled: bool) -> Result<(), String> {
    with_conn(&state, |c| {
        pfcore::rules::set_rule_enabled(c, rule_id, enabled)
    })
}

#[tauri::command(async)]
fn rule_delete(state: State<AppState>, rule_id: i64) -> Result<(), String> {
    with_conn(&state, |c| pfcore::rules::delete_rule(c, rule_id))
}

#[tauri::command(async)]
fn set_window_title(app: AppHandle, title: String) -> Result<(), String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    win.set_title(&title).map_err(|e| e.to_string())
}

// --------------------------------------------------------------------- setup

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let db_path = AppState::resolve_db_path(&handle);
            app.manage(AppState::new(db_path));
            app.manage(synccmd::SyncState::new());
            autolock::spawn(handle.clone());
            aiworker::spawn(handle.clone());
            mailworker::spawn(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db_status,
            unlock,
            lock,
            heartbeat,
            counts,
            global_search,
            messages_page,
            tasks_page,
            tasks_tree,
            task_set_parent,
            trash,
            trash_restore,
            trash_purge,
            undo,
            item_links,
            item_link,
            item_unlink,
            settings_get,
            settings_set,
            backup_now,
            mail_sync,
            google_connect,
            google_sync,
            google_status,
            google_disconnect,
            outbox_process,
            mail_accounts,
            mail_account_create,
            mail_folders,
            message_full,
            message_mark_read,
            mail_queue_draft,
            notes_page,
            diary_entries,
            diary_get,
            diary_save,
            diary_delete,
            diary_months,
            diary_streak,
            note_get,
            note_create,
            note_save,
            note_set_favorite,
            note_delete,
            calendars_list,
            calendar_create,
            events_window,
            event_save,
            event_delete,
            events_import_ics,
            events_export_ics,
            tasks_open,
            tasks_completed,
            task_save,
            task_delete,
            contacts_list,
            contact_save,
            contact_delete,
            contacts_import_vcf,
            contacts_export_vcf,
            passwords_list,
            password_save,
            password_delete,
            password_generate,
            reminders_take,
            message_set_flag,
            message_delete,
            messages_unified,
            mail_search,
            thread_messages,
            message_convert,
            import_csv_preview,
            import_csv_validate,
            import_csv_commit,
            rules_list,
            rule_create,
            rule_set_enabled,
            rule_delete,
            set_window_title,
            synccmd::wifi_status,
            synccmd::wifi_advertise_start,
            synccmd::wifi_advertise_stop,
            synccmd::wifi_pair_code,
            synccmd::device_pair,
            synccmd::devices_list,
            synccmd::device_remove,
            synccmd::device_sync,
            synccmd::backup_list,
            synccmd::backup_restore,
            synccmd::dav_profiles,
            synccmd::dav_save,
            synccmd::dav_remove,
            synccmd::dav_sync,
            synccmd::export_module,
            aicmd::ai_status,
            aicmd::ai_settings_get,
            aicmd::ai_settings_set,
            aicmd::ai_run,
            aicmd::ai_job,
            aicmd::ai_cancel,
            aicmd::ai_reindex,
            aicmd::ai_search_semantic,
            aicmd::free_slots,
            aicmd::dupe_candidates,
            aicmd::merge_contacts,
            aicmd::item_tag_add,
            aicmd::item_tag_remove,
            aicmd::ai_usage,
            aicmd::ai_usage_reset,
            related_items,
            today_dashboard,
            event_participants,
            open_provider_help,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Smart PIM");
}
