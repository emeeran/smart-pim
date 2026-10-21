//! Google OAuth (browser + loopback redirect) and Calendar/Tasks REST client.
//!
//! OAuth: the caller binds a `TcpListener` on `127.0.0.1`, [`auth_url`] builds
//! the consent URL (offline access, so a refresh token comes back), the system
//! browser opens it, and [`loopback_wait`] catches the redirect, validates the
//! anti-forgery `state` and returns the one-time code. Tokens live only in the
//! SQLCipher settings table (managed by `googlecmd`); access tokens are never
//! persisted.
//!
//! REST: [`GoogleRest`] implements `pfcore::gsync::GoogleClient` over the
//! Calendar v3 and Tasks v1 APIs, mapping rows to/from JSON with serde. Like
//! `dav.rs`, I/O sits on a tiny transport ([`BearerHttp`] over the same
//! `dav::Http` trait) and all pure mapping is unit-tested.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use pfcore::gsync::{EventPage, GError, GTask, GEvent, GoogleClient, TaskPage};
use serde::Deserialize;

use crate::dav::Http;

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const EVENTS_URL: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const TASKS_URL: &str = "https://tasks.googleapis.com/tasks/v1/lists";
pub const SCOPE_CALENDAR: &str = "https://www.googleapis.com/auth/calendar.events";
pub const SCOPE_TASKS: &str = "https://www.googleapis.com/auth/tasks";
pub const SCOPE_EMAIL: &str = "openid email";

// ------------------------------------------------------------------- oauth

/// Percent-encode a query-component value (RFC 3986 unreserved set kept).
#[must_use]
pub fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Consent-screen URL: loopback redirect, offline access so a refresh token
/// is issued, `prompt=consent` to force one even on repeat connections.
#[must_use]
pub fn auth_url(client_id: &str, port: u16, state: &str) -> String {
    format!(
        "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&state={}",
        form_encode(client_id),
        form_encode(&format!("http://127.0.0.1:{port}")),
        form_encode(&format!("{SCOPE_CALENDAR} {SCOPE_TASKS} {SCOPE_EMAIL}")),
        form_encode(state),
    )
}

/// Parse `GET /?code=…&state=…` (or `?error=…`) from a redirect request line.
pub fn parse_redirect(line: &str) -> Result<(Option<String>, Option<String>), String> {
    let query = line.split_whitespace().nth(1).ok_or("malformed request line")?;
    let query = query.rsplit_once('?').map_or(query, |(_, q)| q);
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "code" => code = Some(v.to_string()),
            "state" => state = Some(v.to_string()),
            "error" => error = Some(v.to_string()),
            _ => {}
        }
    }
    if let Some(e) = error {
        return Err(format!("Google returned `{e}`"));
    }
    if code.is_none() {
        return Err("no authorization code in redirect".into());
    }
    Ok((code, state))
}

/// The consent page reply shown in the browser.
const LOOPBACK_PAGE: &str = "<html><body><h2>Smart PIM is connected.</h2>\
<p>You can close this tab and return to the app.</p></body></html>";

/// Accept loopback redirects until one carries `want_state` (or `deadline`).
/// Every connection gets an answer; mismatches get 400 and the wait
/// continues — a stray prefetch or second tab must not abort the flow.
///
/// # Errors
/// I/O problems, browser-reported errors, state mismatch at deadline.
pub fn loopback_wait(
    listener: &TcpListener,
    want_state: &str,
    deadline: Instant,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("loopback: {e}"))?;
    loop {
        if Instant::now() >= deadline {
            return Err("loopback: timed out waiting for Google's redirect".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .map_err(|e| format!("loopback: {e}"))?;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let line = req.lines().next().unwrap_or_default();
                let (code, state) = parse_redirect(line)?;
                let ok = state.as_deref() == Some(want_state);
                let body = if ok { LOOPBACK_PAGE } else { "<html><body>state mismatch</body></html>" };
                let reply = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    if ok { "200" } else { "400" },
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
                if ok {
                    return code.ok_or_else(|| "no authorization code".to_string());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("loopback: {e}")),
        }
    }
}

/// Open the system browser (`xdg-open` / `open` / `cmd /c start`).
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot open browser: {e}"))
}

/// Token response from `oauth2.googleapis.com/token`.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    /// Present on the first consent only; Google may omit it on refresh.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// POST form body for the token endpoint.
fn token_form(client_id: &str, client_secret: &str, grant: &str) -> String {
    format!(
        "client_id={}&client_secret={}&{grant}",
        form_encode(client_id),
        form_encode(client_secret)
    )
}

fn parse_token(body: &[u8]) -> Result<TokenSet, String> {
    serde_json::from_slice(body).map_err(|e| format!("token response: {e}: {}", String::from_utf8_lossy(body)))
}

/// Exchange the one-time code for tokens (`redirect_uri` must match).
///
/// # Errors
/// Transport or Google error payload.
pub fn exchange_code(
    http: &dyn crate::dav::Http,
    client_id: &str,
    client_secret: &str,
    code: &str,
    port: u16,
) -> Result<TokenSet, String> {
    let grant = format!(
        "code={}&redirect_uri={}&grant_type=authorization_code",
        form_encode(code),
        form_encode(&format!("http://127.0.0.1:{port}"))
    );
    let resp = http.request(
        "POST",
        TOKEN_ENDPOINT,
        token_form(client_id, client_secret, &grant).as_bytes(),
        &[("Content-Type".into(), "application/x-www-form-urlencoded".into())],
    )?;
    if resp.status != 200 {
        return Err(format!("token exchange failed ({}): {}", resp.status, String::from_utf8_lossy(&resp.body)));
    }
    parse_token(&resp.body)
}

/// Refresh an expired access token. `refresh_token` survives responses that
/// omit it, so callers persist `TokenSet.refresh_token.or(previous)`.
///
/// # Errors
/// Transport or Google error payload (e.g. `invalid_grant` after revocation).
pub fn refresh_token(
    http: &dyn crate::dav::Http,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenSet, String> {
    let grant = format!(
        "refresh_token={}&grant_type=refresh_token",
        form_encode(refresh_token)
    );
    let resp = http.request(
        "POST",
        TOKEN_ENDPOINT,
        token_form(client_id, client_secret, &grant).as_bytes(),
        &[("Content-Type".into(), "application/x-www-form-urlencoded".into())],
    )?;
    if resp.status != 200 {
        return Err(format!("token refresh failed ({}): {}", resp.status, String::from_utf8_lossy(&resp.body)));
    }
    parse_token(&resp.body)
}

/// Look up the Google account email of an access token.
///
/// # Errors
/// Transport or HTTP failure.
pub fn token_email(http: &dyn crate::dav::Http, access_token: &str) -> Result<String, String> {
    let resp = http.request(
        "GET",
        "https://openidconnect.googleapis.com/v1/userinfo",
        &[],
        &[("Authorization".into(), format!("Bearer {access_token}"))],
    )?;
    if resp.status != 200 {
        return Err(format!("userinfo failed ({})", resp.status));
    }
    #[derive(Deserialize)]
    struct UserInfo {
        email: String,
    }
    serde_json::from_slice::<UserInfo>(&resp.body)
        .map(|u| u.email)
        .map_err(|e| format!("userinfo: {e}"))
}

// ---------------------------------------------------------------- transport

/// `dav::Http` implementation that authenticates with a bearer token instead
/// of basic auth; built like `UreqHttp` (no status-as-error, 30 s timeout).
/// [`BearerHttp::anonymous`] sends no Authorization header — that is what the
/// token endpoint itself requires (it rejects bearer/basic auth headers).
pub struct BearerHttp {
    token: String,
    agent: ureq::Agent,
}

impl BearerHttp {
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { token: token.into(), agent }
    }

    #[must_use]
    pub fn anonymous() -> Self {
        Self::new(String::new())
    }
}

impl crate::dav::Http for BearerHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        headers: &[(String, String)],
    ) -> Result<crate::dav::Response, String> {
        let m = ureq::http::Method::from_bytes(method.as_bytes())
            .map_err(|e| format!("google: bad method {method}: {e}"))?;
        let mut builder = ureq::http::Request::builder().method(m).uri(url);
        if !self.token.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", self.token));
        }
        for (name, value) in headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let req = builder
            .body(body.to_vec())
            .map_err(|e| format!("google: building request for {url}: {e}"))?;
        let resp = self
            .agent
            .run(req)
            .map_err(|e| format!("google: {method} {url}: {e}"))?;
        let status = resp.status().as_u16();
        let resp_headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let mut body_out = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut body_out)
            .map_err(|e| format!("google: reading {url}: {e}"))?;
        Ok(crate::dav::Response { status, headers: resp_headers, body: body_out })
    }
}

// --------------------------------------------------------------------- JSON

#[derive(Debug, Deserialize)]
struct EventJson {
    // Request payloads (round-trip tests) carry no id; responses always do.
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    start: Option<WhenJson>,
    #[serde(default)]
    end: Option<WhenJson>,
    #[serde(default)]
    recurrence: Vec<String>,
    #[serde(default)]
    updated: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhenJson {
    #[serde(default)]
    date: Option<String>,
    #[serde(default, rename = "dateTime")]
    date_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventsPageJson {
    #[serde(default)]
    items: Vec<EventJson>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(default, rename = "nextSyncToken")]
    next_sync_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    due: Option<String>,
    #[serde(default)]
    deleted: Option<bool>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    updated: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TasksPageJson {
    #[serde(default)]
    items: Vec<TaskJson>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

/// First `RRULE:` line body from a Google `recurrence` array.
#[must_use]
pub fn rrule_from_recurrence(recurrence: &[String]) -> Option<String> {
    recurrence.iter().find_map(|line| line.strip_prefix("RRULE:").map(String::from))
}

/// Google `WhenJson` → local start/end strings (`YYYY-MM-DD` for all-day).
fn when_strings(when: &WhenJson) -> (String, bool) {
    match (&when.date_time, &when.date) {
        (Some(dt), _) => (dt.clone(), false),
        (None, Some(d)) => (d.clone(), true),
        (None, None) => (String::new(), false),
    }
}

impl TryFrom<&EventJson> for GEvent {
    type Error = String;

    fn try_from(j: &EventJson) -> Result<Self, Self::Error> {
        let (start, all_day) = j
            .start
            .as_ref()
            .map(when_strings)
            .ok_or_else(|| format!("event {}: no start", j.id))?;
        let end = j.end.as_ref().map(|w| when_strings(w).0);
        Ok(GEvent {
            id: (!j.id.is_empty()).then(|| j.id.clone()),
            title: j.summary.clone().unwrap_or_default(),
            start,
            end,
            all_day,
            location: j.location.clone(),
            description: j.description.clone(),
            rrule: rrule_from_recurrence(&j.recurrence),
            updated: j.updated.clone().unwrap_or_default(),
            deleted: j.status.as_deref() == Some("cancelled"),
        })
    }
}

/// Serialize a local row into an events.insert/update payload.
#[must_use]
pub fn event_payload(ev: &GEvent) -> serde_json::Value {
    use serde_json::json;
    let (start_v, end_v) = if ev.all_day {
        (json!({ "date": ev.start }), ev.end.as_ref().map(|e| json!({ "date": e })))
    } else {
        (json!({ "dateTime": ev.start }), ev.end.as_ref().map(|e| json!({ "dateTime": e })))
    };
    let mut payload = json!({
        "summary": ev.title,
        "start": start_v,
        "end": end_v.unwrap_or_else(|| json!({ "dateTime": ev.start })),
    });
    if let Some(loc) = &ev.location {
        payload["location"] = json!(loc);
    }
    if let Some(desc) = &ev.description {
        payload["description"] = json!(desc);
    }
    if let Some(rrule) = &ev.rrule {
        payload["recurrence"] = json!([format!("RRULE:{rrule}")]);
    }
    payload
}

impl From<&TaskJson> for GTask {
    fn from(j: &TaskJson) -> Self {
        GTask {
            id: (!j.id.is_empty()).then(|| j.id.clone()),
            title: j.title.clone().unwrap_or_default(),
            notes: j.notes.clone(),
            due: j.due.clone(),
            completed: j.status.as_deref() == Some("completed"),
            updated: j.updated.clone().unwrap_or_default(),
            deleted: j.deleted.unwrap_or(false),
        }
    }
}

/// Serialize a local row into a tasks.insert/update payload.
#[must_use]
pub fn task_payload(t: &GTask) -> serde_json::Value {
    use serde_json::json;
    let mut payload = json!({ "title": t.title });
    if let Some(notes) = &t.notes {
        payload["notes"] = json!(notes);
    }
    if let Some(due) = &t.due {
        payload["due"] = json!(due);
    }
    payload["status"] = json!(if t.completed { "completed" } else { "needsAction" });
    payload
}

// -------------------------------------------------------------------- client

/// Blocking REST client bound to one access token. One instance per sync run;
/// token refresh happens between runs (in `googlecmd`), not mid-run.
pub struct GoogleRest {
    http: BearerHttp,
}

impl GoogleRest {
    #[must_use]
    pub fn new(access_token: &str) -> Self {
        Self { http: BearerHttp::new(access_token) }
    }

    /// GET with rate-limit backoff (1 s / 4 s / 16 s on 403/429, honoring
    /// `Retry-After` when present).
    fn get(&self, url: &str) -> Result<crate::dav::Response, String> {
        let mut wait = 1u64;
        for attempt in 0..3 {
            let resp = self.http.request("GET", url, &[], &[])?;
            if resp.status != 429 && resp.status != 403 || attempt == 2 {
                return Ok(resp);
            }
            let delay = resp
                .header("Retry-After")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(wait);
            std::thread::sleep(Duration::from_secs(delay.min(30)));
            wait *= 4;
        }
        unreachable!("loop returns before attempt 3")
    }

    fn error(status: u16, body: &[u8]) -> GError {
        if status == 410 {
            GError::Gone
        } else {
            GError::Api(format!("Google API {status}: {}", String::from_utf8_lossy(body)))
        }
    }

    fn ensure2(resp: crate::dav::Response) -> Result<crate::dav::Response, GError> {
        if (200..300).contains(&resp.status) {
            Ok(resp)
        } else {
            Err(Self::error(resp.status, &resp.body))
        }
    }
}

impl GoogleClient for GoogleRest {
    fn list_events(&self, sync_token: Option<&str>) -> Result<EventPage, GError> {
        let mut events = Vec::new();
        let mut page_token: Option<String> = None;
        let mut sync_out = None;
        loop {
            let mut url = format!("{EVENTS_URL}?maxResults=250&showDeleted=true");
            if let Some(tok) = &sync_token {
                url.push_str("&syncToken=");
                url.push_str(&form_encode(tok));
            }
            if let Some(tok) = &page_token {
                url.push_str("&pageToken=");
                url.push_str(&form_encode(tok));
            }
            let resp = Self::ensure2(self.get(&url).map_err(GError::Api)?)?;
            let page: EventsPageJson =
                serde_json::from_slice(&resp.body).map_err(|e| GError::Api(e.to_string()))?;
            for item in &page.items {
                if let Ok(ev) = GEvent::try_from(item) {
                    events.push(ev);
                }
            }
            page_token = page.next_page_token.clone();
            // nextSyncToken arrives only on the final page.
            if page_token.is_none() {
                sync_out = page.next_sync_token.clone();
            }
            if page_token.is_none() {
                break;
            }
        }
        Ok(EventPage { events, next_sync_token: sync_out })
    }

    fn upsert_event(&self, ev: &GEvent) -> Result<GEvent, GError> {
        let payload = serde_json::to_vec(&event_payload(ev)).map_err(|e| GError::Api(e.to_string()))?;
        let (method, url) = match &ev.id {
            Some(id) => ("PUT", format!("{EVENTS_URL}/{}", form_encode(id))),
            None => ("POST", EVENTS_URL.to_string()),
        };
        let resp = self
            .http
            .request(
                method,
                &url,
                &payload,
                &[("Content-Type".into(), "application/json".into())],
            )
            .map_err(GError::Api)?;
        let resp = Self::ensure2(resp)?;
        let saved: EventJson =
            serde_json::from_slice(&resp.body).map_err(|e| GError::Api(e.to_string()))?;
        GEvent::try_from(&saved).map_err(GError::Api)
    }

    fn delete_event(&self, id: &str) -> Result<(), GError> {
        let url = format!("{EVENTS_URL}/{}", form_encode(id));
        let resp = self.http.request("DELETE", &url, &[], &[]).map_err(GError::Api)?;
        // 410/404: already gone remotely — that is success for a push.
        match resp.status {
            200..=299 | 404 | 410 => Ok(()),
            s => Err(Self::error(s, &resp.body)),
        }
    }

    fn list_tasks(&self, updated_min: Option<&str>) -> Result<TaskPage, GError> {
        let mut tasks = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = format!("{TASKS_URL}/%40default/tasks?maxResults=100&showCompleted=true&showHidden=true&showDeleted=true");
            if let Some(min) = updated_min {
                url.push_str("&updatedMin=");
                url.push_str(&form_encode(min));
            }
            if let Some(tok) = &page_token {
                url.push_str("&pageToken=");
                url.push_str(&form_encode(tok));
            }
            let resp = Self::ensure2(self.get(&url).map_err(GError::Api)?)?;
            let page: TasksPageJson =
                serde_json::from_slice(&resp.body).map_err(|e| GError::Api(e.to_string()))?;
            tasks.extend(page.items.iter().map(GTask::from));
            page_token = page.next_page_token.clone();
            if page_token.is_none() {
                break;
            }
        }
        Ok(TaskPage { tasks })
    }

    fn upsert_task(&self, task: &GTask) -> Result<GTask, GError> {
        let payload = serde_json::to_vec(&task_payload(task)).map_err(|e| GError::Api(e.to_string()))?;
        let (method, url) = match &task.id {
            Some(id) => ("PUT", format!("{TASKS_URL}/%40default/tasks/{}", form_encode(id))),
            None => ("POST", format!("{TASKS_URL}/%40default/tasks")),
        };
        let resp = self
            .http
            .request(
                method,
                &url,
                &payload,
                &[("Content-Type".into(), "application/json".into())],
            )
            .map_err(GError::Api)?;
        let resp = Self::ensure2(resp)?;
        let saved: TaskJson =
            serde_json::from_slice(&resp.body).map_err(|e| GError::Api(e.to_string()))?;
        Ok(GTask::from(&saved))
    }

    fn delete_task(&self, id: &str) -> Result<(), GError> {
        let url = format!("{TASKS_URL}/%40default/tasks/{}", form_encode(id));
        let resp = self.http.request("DELETE", &url, &[], &[]).map_err(GError::Api)?;
        match resp.status {
            200..=299 | 404 | 410 => Ok(()),
            s => Err(Self::error(s, &resp.body)),
        }
    }
}

// --------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_url_carries_consent_parameters() {
        let url = auth_url("cid-1", 8321, "st4te");
        assert!(url.starts_with(AUTH_ENDPOINT));
        assert!(url.contains("client_id=cid-1"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8321"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("state=st4te"));
        assert!(url.contains(&form_encode(SCOPE_CALENDAR)));
        assert!(url.contains(&form_encode(SCOPE_TASKS)));
    }

    #[test]
    fn redirect_lines_yield_code_and_state() {
        let line = "GET /?code=abc%2F1&state=s1&scope=x HTTP/1.1";
        let (code, state) = parse_redirect(line).unwrap();
        assert_eq!(code.as_deref(), Some("abc%2F1"));
        assert_eq!(state.as_deref(), Some("s1"));
        assert!(parse_redirect("GET /?state=s1&error=access_denied HTTP/1.1")
            .unwrap_err()
            .contains("access_denied"));
        assert!(parse_redirect("GET /?state=s1 HTTP/1.1").is_err());
    }

    #[test]
    fn event_payload_roundtrip_shape() {
        let ev = GEvent {
            id: Some(String::from("ev1")),
            title: String::from("Standup"),
            start: String::from("2026-10-21T10:00:00Z"),
            end: Some(String::from("2026-10-21T10:30:00Z")),
            all_day: false,
            location: Some(String::from("Room")),
            description: None,
            rrule: Some(String::from("FREQ=DAILY;COUNT=5")),
            updated: String::new(),
            deleted: false,
        };
        let payload = event_payload(&ev);
        assert_eq!(payload["summary"], "Standup");
        assert_eq!(payload["start"]["dateTime"], "2026-10-21T10:00:00Z");
        assert_eq!(payload["recurrence"][0], "RRULE:FREQ=DAILY;COUNT=5");
        let text = serde_json::to_string(&payload).unwrap();
        let parsed: EventJson = serde_json::from_str(&text).unwrap();
        let back = GEvent::try_from(&parsed).unwrap();
        assert_eq!(back.title, "Standup");
        assert_eq!(back.rrule.as_deref(), Some("FREQ=DAILY;COUNT=5"));
    }

    #[test]
    fn all_day_events_use_date_fields() {
        let ev = GEvent {
            title: String::from("Offsite"),
            start: String::from("2026-10-22"),
            end: Some(String::from("2026-10-23")),
            all_day: true,
            ..GEvent::default()
        };
        let payload = event_payload(&ev);
        assert!(payload["start"].get("date").is_some());
        assert!(payload["start"].get("dateTime").is_none());
        let text = serde_json::to_string(&payload).unwrap();
        let parsed: EventJson = serde_json::from_str(&text).unwrap();
        let back = GEvent::try_from(&parsed).unwrap();
        assert!(back.all_day);
        assert_eq!(back.start, "2026-10-22");
    }

    #[test]
    fn cancelled_events_map_to_deleted() {
        let json = r#"{"id":"e1","status":"cancelled","summary":"x",
            "start":{"dateTime":"2026-10-21T10:00:00Z"},"updated":"2026-10-21T09:00:00Z"}"#;
        let parsed: EventJson = serde_json::from_str(json).unwrap();
        let ev = GEvent::try_from(&parsed).unwrap();
        assert!(ev.deleted);
    }

    #[test]
    fn task_payload_and_mapping() {
        let t = GTask {
            id: None,
            title: String::from("Buy milk"),
            notes: Some(String::from("2L")),
            due: Some(String::from("2026-10-25T17:00:00.000Z")),
            completed: true,
            updated: String::new(),
            deleted: false,
        };
        let payload = task_payload(&t);
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["title"], "Buy milk");
        let text = serde_json::to_string(&payload).unwrap();
        let parsed: TaskJson = serde_json::from_str(&text).unwrap();
        let back = GTask::from(&parsed);
        assert!(back.completed);
        assert_eq!(back.title, "Buy milk");
        assert_eq!(back.id, None, "mapping a payload never invents an id");
    }

    #[test]
    fn rrule_extraction_ignores_other_lines() {
        let rec = vec![
            String::from("RDATE:20261022T000000Z"),
            String::from("RRULE:FREQ=WEEKLY;BYDAY=MO"),
        ];
        assert_eq!(rrule_from_recurrence(&rec).as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(rrule_from_recurrence(&[]), None);
    }
}
