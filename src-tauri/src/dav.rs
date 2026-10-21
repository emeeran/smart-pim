//! WebDAV / CalDAV / CardDAV client for cloud sync (Gate 4).
//!
//! Two calendar/card models are supported, mirroring how the modules work:
//!
//! * **Item model** — calendars/address books stay the source of truth and the
//!   server holds individual VEVENT/VCARD resources. `fetch_events` /
//!   `fetch_contacts` / `put_resource` / `get_resource` cover that.
//! * **Whole-file model** — the entire SQLCipher database is one encrypted
//!   blob pushed to a WebDAV folder (`download_to` / `upload_from` /
//!   `remote_changed`). ETags give optimistic concurrency so two devices
//!   cannot silently clobber each other.
//!
//! All XML building and parsing lives in pure functions that are unit-tested
//! here; network I/O is behind the tiny [`Http`] trait so tests drive the
//! client with a recording fake instead of a server. XML is handled with a
//! small hand-rolled scanner rather than a new dependency — DAV multistatus
//! responses are regular enough (namespace-prefixed `<D:response>` vs
//! `<d:response>` vs bare `<response>`) that local-name matching is
//! sufficient and avoids a full XML stack in the Tauri shell.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

// ------------------------------------------------------------------ transport

/// A completed HTTP exchange. Non-2xx statuses are *not* errors here: the
/// transport returns the response and lets callers judge (DAV servers use
/// 207 Multi-Status where a plain HTTP client would look successful).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// Case-insensitive header lookup (first match wins).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    /// `ETag` header value, if the server sent one.
    #[must_use]
    pub fn etag(&self) -> Option<String> {
        self.header("ETag")
    }
}

/// Minimal blocking HTTP transport. Implementors send `body` (empty for
/// GET/HEAD/OPTIONS) plus the given extra headers (`Depth`, `If-Match`, …)
/// and must never panic on transport errors — those become `Err`.
pub trait Http {
    fn request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        headers: &[(String, String)],
    ) -> Result<Response, String>;
}

/// `ureq`-backed transport with HTTP basic auth and a 30 s global timeout.
///
/// Status codes are *not* turned into errors (`http_status_as_error(false)`):
/// DAV leans on 207 Multi-Status, 404 and 412, so the raw response is what
/// callers need. The agent is kept on the struct so connections are pooled
/// across the requests of one sync run.
///
/// Credentials live only in the SQLCipher-encrypted settings database, like
/// the mail accounts in `mailsync`; nothing is written to disk here.
pub struct UreqHttp {
    user: String,
    pass: String,
    agent: ureq::Agent,
}

impl UreqHttp {
    #[must_use]
    pub fn new(user: impl Into<String>, pass: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            user: user.into(),
            pass: pass.into(),
            agent,
        }
    }

    fn auth_header(&self) -> String {
        let raw = format!("{}:{}", self.user, self.pass);
        format!("Basic {}", base64_encode(raw.as_bytes()))
    }
}

impl Http for UreqHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        headers: &[(String, String)],
    ) -> Result<Response, String> {
        let m = ureq::http::Method::from_bytes(method.as_bytes())
            .map_err(|e| format!("dav: bad method {method}: {e}"))?;
        let mut builder = ureq::http::Request::builder()
            .method(m)
            .uri(url)
            .header("Authorization", self.auth_header());
        for (name, value) in headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let req = builder
            .body(body.to_vec())
            .map_err(|e| format!("dav: building request for {url}: {e}"))?;
        let resp = self
            .agent
            .run(req)
            .map_err(|e| format!("dav: {method} {url}: {e}"))?;

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
        let mut reader = resp.into_body().into_reader();
        let mut resp_body = Vec::new();
        reader
            .read_to_end(&mut resp_body)
            .map_err(|e| format!("dav: reading response body from {url}: {e}"))?;
        Ok(Response {
            status,
            headers: resp_headers,
            body: resp_body,
        })
    }
}

/// Standard base64 (RFC 4648, padded) — hand-rolled so no extra dependency is
/// pulled in just to build one Authorization header.
#[must_use]
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ------------------------------------------------------------- URL joining

/// Join a base URL and a relative path, tolerating trailing/leading slashes.
/// Absolute `path` values (`http…`) are returned unchanged so callers can
/// feed server-provided hrefs straight back in.
#[must_use]
pub fn join_url(base: &str, path: &str) -> String {
    if path.contains("://") {
        return path.to_string();
    }
    let base = base.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return base.to_string();
    }
    format!("{base}/{}", path.trim_start_matches('/'))
}

// ------------------------------------------------------- request builders

/// PROPFIND (Depth set by the caller) asking what a collection is: name,
/// whether it is a calendar/address book (`resourcetype`), its sync token
/// material (`getetag`) and a human description.
#[must_use]
#[allow(dead_code)] // used by CalDAV discovery (Android app phase)
pub fn propfind_depth1() -> String {
    String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
        "<d:propfind xmlns:d=\"DAV:\">",
        "<d:prop>",
        "<d:displayname/>",
        "<d:resourcetype/>",
        "<d:getetag/>",
        "<c:calendar-description xmlns:c=\"urn:ietf:params:xml:ns:caldav\"/>",
        "</d:prop>",
        "</d:propfind>"
    ))
}

/// CALDAV:calendar-query REPORT — every VEVENT overlapping `[start, end]`
/// (RFC 3339 UTC strings), pulling the full iCalendar body plus its ETag so
/// writes can be conditional afterwards.
#[must_use]
pub fn calendar_query(start: &str, end: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            "<c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">",
            "<d:prop>",
            "<d:getetag/>",
            "<c:calendar-data/>",
            "</d:prop>",
            "<c:filter>",
            "<c:comp-filter name=\"VCALENDAR\">",
            "<c:comp-filter name=\"VEVENT\">",
            "<c:time-range start=\"{start}\" end=\"{end}\"/>",
            "</c:comp-filter>",
            "</c:comp-filter>",
            "</c:filter>",
            "</c:calendar-query>"
        ),
        start = start,
        end = end
    )
}

/// CARDDAV:addressbook-query REPORT — all vCards (their text form plus
/// ETag), capped so an enormous address book cannot stall the UI.
#[must_use]
pub fn addressbook_query() -> String {
    String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
        "<c:addressbook-query xmlns:d=\"DAV:\" ",
        "xmlns:c=\"urn:ietf:params:xml:ns:carddav\">",
        "<d:prop>",
        "<d:getetag/>",
        "<c:addressbook-data/>",
        "</d:prop>",
        "<limit><nresults>500</nresults></limit>",
        "</c:addressbook-query>"
    ))
}

// ------------------------------------------------------------- XML parsing

/// One resource from a 207 Multi-Status body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavItem {
    pub href: String,
    pub etag: Option<String>,
    /// VEVENT/VCARD text (`calendar-data` / `addressbook-data`), when asked for.
    pub data: Option<String>,
}

/// Expand the five predefined XML entities plus `&#NN;` / `&#xHH;` numeric
/// references. Scans left to right so `&amp;lt;` correctly yields `&lt;`.
#[must_use]
pub fn xml_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let Some(semi) = s[i..].find(';') else {
            out.push('&');
            i += 1;
            continue;
        };
        let end = i + semi;
        let ent = &s[i + 1..end];
        if ent == "amp" {
            out.push('&');
        } else if ent == "lt" {
            out.push('<');
        } else if ent == "gt" {
            out.push('>');
        } else if ent == "quot" {
            out.push('"');
        } else if ent == "apos" {
            out.push('\'');
        } else if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
            } else {
                out.push_str(&s[i..=end]);
            }
        } else if let Some(dec) = ent.strip_prefix('#') {
            if let Ok(cp) = dec.parse::<u32>() {
                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
            } else {
                out.push_str(&s[i..=end]);
            }
        } else {
            // Unknown entity: keep the raw text.
            out.push_str(&s[i..=end]);
        }
        i = end + 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Open,
    Close,
    SelfClose,
    /// `<?…?>`, `<!DOCTYPE …>`, comments and CDATA — skipped by the scanner.
    Misc,
}

#[derive(Debug, Clone)]
struct Tag {
    local: String,
    kind: TagKind,
    /// Offset of the opening `<`.
    start: usize,
    /// Offset just past the closing `>`.
    end: usize,
}

/// Find the next tag starting at or after `from`. Attribute values may
/// contain `>`, so quoting is respected while scanning.
fn next_tag(xml: &str, from: usize) -> Option<Tag> {
    let bytes = xml.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &xml[i..];
        if let Some(body) = rest.strip_prefix("<!--") {
            // Comments are skipped whole (a `>` inside one is not a tag end).
            let end = body.find("-->").map(|p| i + 4 + p + 3);
            i = end.unwrap_or(xml.len());
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            let end = rest.find('>').map(|p| i + p + 1).unwrap_or(xml.len());
            return Some(Tag {
                local: String::new(),
                kind: TagKind::Misc,
                start: i,
                end,
            });
        }
        let closing = rest[1..].starts_with('/');
        let name_start = if closing { i + 2 } else { i + 1 };
        let name_end = name_start
            + bytes[name_start..]
                .iter()
                .take_while(|b| {
                    b.is_ascii_alphanumeric() || **b == b':' || **b == b'_' || **b == b'-'
                })
                .count();
        let name = &xml[name_start..name_end];
        // Walk to the tag end, ignoring `>` inside quoted attribute values.
        let mut j = name_end;
        let mut quote = ' ';
        let mut self_closing = false;
        while j < bytes.len() {
            let c = bytes[j] as char;
            if quote != ' ' {
                if c == quote {
                    quote = ' ';
                }
            } else if c == '"' || c == '\'' {
                quote = c;
            } else if c == '>' {
                break;
            } else if c == '/' && bytes.get(j + 1) == Some(&b'>') {
                self_closing = true;
                break;
            }
            j += 1;
        }
        let kind = if closing {
            TagKind::Close
        } else if self_closing {
            TagKind::SelfClose
        } else {
            TagKind::Open
        };
        return Some(Tag {
            local: local_name(name),
            kind,
            start: i,
            end: (j + 1).min(xml.len()),
        });
    }
    None
}

/// Local name of a possibly namespace-prefixed tag (`d:response` → `response`).
fn local_name(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((_, local)) => local.to_ascii_lowercase(),
        None => name.to_ascii_lowercase(),
    }
}

/// Text content of the *first* element with this local name inside `xml`,
/// CDATA unwrapped, entities left intact for the caller's [`xml_unescape`].
fn element_text(xml: &str, local: &str) -> Option<String> {
    let mut pos = 0;
    loop {
        let tag = next_tag(xml, pos)?;
        pos = tag.end;
        if tag.kind == TagKind::Open && tag.local == local {
            let mut depth = 1usize;
            let mut cursor = tag.end;
            loop {
                let inner = next_tag(xml, cursor)?;
                cursor = inner.end;
                if inner.local != local {
                    continue;
                }
                match inner.kind {
                    TagKind::Open => depth += 1,
                    TagKind::Close => {
                        depth -= 1;
                        if depth == 0 {
                            let raw = &xml[tag.end..inner.start];
                            return Some(unwrap_cdata(raw));
                        }
                    }
                    TagKind::SelfClose | TagKind::Misc => {}
                }
            }
        }
    }
}

fn unwrap_cdata(raw: &str) -> String {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("<![CDATA[") {
        return rest.strip_suffix("]]>").unwrap_or(rest).trim().to_string();
    }
    t.to_string()
}

/// Parse a 207 Multi-Status document into its `<response>` entries. Tolerant
/// by design: any prefix (`d:`, `D:`, none), attribute noise, CDATA-wrapped
/// payloads and missing properties are all accepted — an entry only needs an
/// href to be useful.
#[must_use]
pub fn parse_multistatus(xml: &str) -> Vec<DavItem> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(tag) = next_tag(xml, pos) {
        pos = tag.end;
        if tag.kind != TagKind::Open || tag.local != "response" {
            continue;
        }
        // Advance to the matching close (responses do not nest in practice,
        // but track depth anyway so a stray inner tag cannot truncate).
        let mut depth = 1usize;
        let mut cursor = tag.end;
        let end = loop {
            let Some(inner) = next_tag(xml, cursor) else {
                break xml.len();
            };
            cursor = inner.end;
            if inner.local != "response" {
                continue;
            }
            match inner.kind {
                TagKind::Open => depth += 1,
                TagKind::Close => {
                    depth -= 1;
                    if depth == 0 {
                        break inner.start;
                    }
                }
                TagKind::SelfClose | TagKind::Misc => {}
            }
        };
        let inner = &xml[tag.end..end];
        let href = element_text(inner, "href").map(|h| xml_unescape(&h));
        let etag = element_text(inner, "getetag").map(|e| xml_unescape(&e));
        let data = element_text(inner, "calendar-data")
            .or_else(|| element_text(inner, "addressbook-data"))
            .map(|d| xml_unescape(&d));
        if let Some(href) = href {
            out.push(DavItem { href, etag, data });
        }
        pos = end;
    }
    out
}

// ---------------------------------------------------------------- client

const XML_CONTENT_TYPE: &str = "application/xml; charset=utf-8";

/// REPORT a calendar collection for VEVENTs overlapping `[start, end]`.
/// DAV servers answer 207 Multi-Status — anything else is a real error
/// (403/404 for wrong URL or credentials, 200 means it is not a CalDAV
/// collection at all).
pub fn fetch_events(
    http: &dyn Http,
    base_url: &str,
    start: &str,
    end: &str,
) -> Result<Vec<DavItem>, String> {
    let body = calendar_query(start, end);
    let resp = http.request(
        "REPORT",
        base_url,
        body.as_bytes(),
        &[
            ("Depth".to_string(), "1".to_string()),
            ("Content-Type".to_string(), XML_CONTENT_TYPE.to_string()),
        ],
    )?;
    if resp.status != 207 {
        return Err(format!(
            "caldav: calendar REPORT returned HTTP {} (expected 207 Multi-Status) from {base_url}",
            resp.status
        ));
    }
    Ok(parse_multistatus(&String::from_utf8_lossy(&resp.body)))
}

/// REPORT an address book collection for its vCards.
pub fn fetch_contacts(http: &dyn Http, base_url: &str) -> Result<Vec<DavItem>, String> {
    let body = addressbook_query();
    let resp = http.request(
        "REPORT",
        base_url,
        body.as_bytes(),
        &[
            ("Depth".to_string(), "1".to_string()),
            ("Content-Type".to_string(), XML_CONTENT_TYPE.to_string()),
        ],
    )?;
    if resp.status != 207 {
        return Err(format!(
            "carddav: addressbook REPORT returned HTTP {} (expected 207 Multi-Status) from {base_url}",
            resp.status
        ));
    }
    Ok(parse_multistatus(&String::from_utf8_lossy(&resp.body)))
}

/// PUT a resource (VEVENT / VCARD file body). When `if_match_etag` is given
/// it is sent as `If-Match`, so a server that changed the resource since we
/// read it answers 412 Precondition Failed instead of letting us clobber it.
/// Returns the new ETag when the server reports one.
pub fn put_resource(
    http: &dyn Http,
    url: &str,
    body: &[u8],
    if_match_etag: Option<&str>,
) -> Result<String, String> {
    let mut headers = vec![(
        "Content-Type".to_string(),
        "text/calendar; charset=utf-8".to_string(),
    )];
    if let Some(etag) = if_match_etag {
        headers.push(("If-Match".to_string(), etag.to_string()));
    }
    let resp = http.request("PUT", url, body, &headers)?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("dav: PUT {url} returned HTTP {}", resp.status));
    }
    Ok(resp.etag().unwrap_or_default())
}

/// GET a resource, keeping its ETag for later conditional writes.
#[allow(dead_code)] // used by WebDAV pull-review and the Android app phase
pub fn get_resource(http: &dyn Http, url: &str) -> Result<DavItem, String> {
    let resp = http.request("GET", url, &[], &[])?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("dav: GET {url} returned HTTP {}", resp.status));
    }
    Ok(DavItem {
        href: url.to_string(),
        etag: resp.etag(),
        data: Some(String::from_utf8_lossy(&resp.body).to_string()),
    })
}

/// OPTIONS the server and return its `DAV:` header value (e.g.
/// `1, 3, extended-mkcol` or `1, 2, calendar-access`). The UI uses this to
/// check that a URL really speaks DAV (and CalDAV/CardDAV) before saving it.
#[allow(dead_code)] // URL validation for the Android app phase
pub fn probe(http: &dyn Http, base_url: &str) -> Result<String, String> {
    let resp = http.request("OPTIONS", base_url, &[], &[])?;
    if !(200..300).contains(&resp.status) {
        return Err(format!(
            "dav: OPTIONS {base_url} returned HTTP {} — not a usable DAV endpoint",
            resp.status
        ));
    }
    Ok(resp.header("DAV").unwrap_or_default())
}

// --------------------------------------------------- whole-file WebDAV sync

/// HEAD the remote file and compare against `known_etag`.
///
/// * `Ok(None)` — unchanged, nothing to do.
/// * `Ok(Some(etag))` — changed (or absent: the ETag is then empty and an
///   upload is the right next step).
pub fn remote_changed(
    http: &dyn Http,
    url: &str,
    known_etag: Option<&str>,
) -> Result<Option<String>, String> {
    let resp = http.request("HEAD", url, &[], &[])?;
    if resp.status == 404 {
        // No remote file yet — first push (or the file was deleted elsewhere).
        return Ok(Some(String::new()));
    }
    if !(200..300).contains(&resp.status) {
        return Err(format!("dav: HEAD {url} returned HTTP {}", resp.status));
    }
    let etag = resp.etag().unwrap_or_default();
    if Some(etag.as_str()) == known_etag {
        return Ok(None);
    }
    Ok(Some(etag))
}

/// GET the remote file to `dest_path`, returning its byte length.
pub fn download_to(http: &dyn Http, url: &str, dest_path: &Path) -> Result<u64, String> {
    let resp = http.request("GET", url, &[], &[])?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("dav: GET {url} returned HTTP {}", resp.status));
    }
    let len = resp.body.len() as u64;
    std::fs::write(dest_path, &resp.body)
        .map_err(|e| format!("dav: writing {}: {e}", dest_path.display()))?;
    Ok(len)
}

/// PUT the local file at `src_path`, honouring `if_match` for optimistic
/// concurrency. Returns the new ETag (empty when the server omits it).
pub fn upload_from(
    http: &dyn Http,
    url: &str,
    src_path: &Path,
    if_match: Option<&str>,
) -> Result<String, String> {
    let body =
        std::fs::read(src_path).map_err(|e| format!("dav: reading {}: {e}", src_path.display()))?;
    let mut headers = vec![(
        "Content-Type".to_string(),
        "application/octet-stream".to_string(),
    )];
    if let Some(etag) = if_match {
        headers.push(("If-Match".to_string(), etag.to_string()));
    }
    let resp = http.request("PUT", url, &body, &headers)?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("dav: PUT {url} returned HTTP {}", resp.status));
    }
    Ok(resp.etag().unwrap_or_default())
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Radicale-style response: `d:` prefix, quoted CDATA-free etag, plain data.
    const RADICALE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:response>
    <d:href>/cal/user/abc123.ics</d:href>
    <d:propstat>
      <d:prop>
        <d:getetag>"a1b2c3"</d:getetag>
        <c:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
SUMMARY:Standup
END:VEVENT
END:VCALENDAR
</c:calendar-data>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/cal/user/def456.ics</d:href>
    <d:propstat>
      <d:prop>
        <d:getetag>"998877"</d:getetag>
        <c:calendar-data>BEGIN:VCALENDAR
END:VCALENDAR
</c:calendar-data>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

    /// Apple/Nextcloud style: uppercase `D:` prefix, CDATA-wrapped etag,
    /// XML-escaped href.
    const UPPERCASE_CDATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/calendar/&amp;personal/green%20room.ics</D:href>
    <D:propstat><D:prop>
      <D:getetag><![CDATA["xyz-etag"]]></D:getetag>
    </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
</D:multistatus>"#;

    /// No namespace prefix at all (some minimal servers), addressbook shape.
    const NO_PREFIX: &str = "<multistatus><response>\
<href>/contacts/v1.vcf</href>\
<propstat><prop><getetag>\"vc-1\"</getetag>\
<addressbook-data>BEGIN:VCARD\nFN:A &amp; B Co\nEND:VCARD</addressbook-data>\
</prop></propstat></response></multistatus>";

    #[test]
    fn parses_radicale_style_propfind_multistatus() {
        let items = parse_multistatus(RADICALE);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].href, "/cal/user/abc123.ics");
        assert_eq!(items[0].etag.as_deref(), Some("\"a1b2c3\""));
        assert!(items[0]
            .data
            .as_deref()
            .unwrap_or("")
            .contains("SUMMARY:Standup"));
        assert_eq!(items[1].href, "/cal/user/def456.ics");
        assert_eq!(items[1].etag.as_deref(), Some("\"998877\""));
    }

    #[test]
    fn parses_uppercase_prefix_and_cdata_etag() {
        let items = parse_multistatus(UPPERCASE_CDATA);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].href, "/calendar/&personal/green%20room.ics");
        assert_eq!(items[0].etag.as_deref(), Some("\"xyz-etag\""));
        assert!(items[0].data.is_none());
    }

    #[test]
    fn parses_prefixless_addressbook_multistatus() {
        let items = parse_multistatus(NO_PREFIX);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].href, "/contacts/v1.vcf");
        assert_eq!(items[0].etag.as_deref(), Some("\"vc-1\""));
        let data = items[0].data.as_deref().unwrap_or_default();
        assert!(data.contains("FN:A & B Co"), "{data}");
    }

    #[test]
    fn multistatus_noise_and_empty_input_are_tolerated() {
        assert!(parse_multistatus("").is_empty());
        assert!(parse_multistatus("<html><body>404</body></html>").is_empty());
        // A response without an href is dropped rather than fabricated.
        let items = parse_multistatus(
            "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:getetag>\"x\"</d:getetag></d:response></d:multistatus>",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn xml_unescape_handles_entities_and_numeric_refs() {
        assert_eq!(xml_unescape("a &amp; b"), "a & b");
        assert_eq!(
            xml_unescape("&lt;tag&gt; &quot;q&quot; &apos;p&apos;"),
            "<tag> \"q\" 'p'"
        );
        assert_eq!(xml_unescape("caf&#233;"), "café");
        assert_eq!(xml_unescape("&#x263A;"), "\u{263a}");
        // Left-to-right scan: never double-decodes.
        assert_eq!(xml_unescape("&amp;lt;"), "&lt;");
        // Degenerate input passes through untouched.
        assert_eq!(xml_unescape("100 &lt 5 &unknown;"), "100 &lt 5 &unknown;");
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // The actual use: an RFC 7617 basic auth header.
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn join_url_handles_slashes_and_absolute_paths() {
        assert_eq!(
            join_url("https://x.ru/cals/", "a.ics"),
            "https://x.ru/cals/a.ics"
        );
        assert_eq!(
            join_url("https://x.ru/cals", "/a.ics"),
            "https://x.ru/cals/a.ics"
        );
        assert_eq!(
            join_url("https://x.ru/cals//", "//a.ics"),
            "https://x.ru/cals/a.ics"
        );
        assert_eq!(join_url("https://x.ru/cals/", ""), "https://x.ru/cals");
        assert_eq!(join_url("https://x.ru/cals/", "/"), "https://x.ru/cals");
        // Server-provided hrefs go back untouched.
        assert_eq!(
            join_url("https://x.ru/cals/", "https://other/a.ics"),
            "https://other/a.ics"
        );
    }

    #[test]
    fn builders_contain_expected_elements() {
        let prop = propfind_depth1();
        for needle in [
            "propfind",
            "displayname",
            "resourcetype",
            "getetag",
            "calendar-description",
            "urn:ietf:params:xml:ns:caldav",
        ] {
            assert!(prop.contains(needle), "propfind missing {needle}");
        }

        let q = calendar_query("20260101T000000Z", "20260201T000000Z");
        assert!(q.contains("calendar-query"));
        assert!(q.contains("comp-filter name=\"VEVENT\""));
        assert!(q.contains("time-range start=\"20260101T000000Z\" end=\"20260201T000000Z\""));
        assert!(q.contains("calendar-data"));
        assert!(!q.contains("addressbook-data"));

        let ab = addressbook_query();
        assert!(ab.contains("addressbook-query"));
        assert!(ab.contains("urn:ietf:params:xml:ns:carddav"));
        assert!(ab.contains("addressbook-data"));
        assert!(ab.contains("<limit><nresults>500</nresults></limit>"));
    }

    // ------------------------------------------------- transport-dependent

    struct Call {
        method: String,
        url: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    }

    /// Records every request and replays canned responses in order.
    struct FakeHttp {
        responses: RefCell<Vec<Response>>,
        calls: RefCell<Vec<Call>>,
    }

    impl FakeHttp {
        /// Shorthand for a single canned response.
        fn one(response: Response) -> Self {
            Self::new([response])
        }

        fn new<I>(responses: I) -> Self
        where
            I: IntoIterator<Item = Response>,
            I::IntoIter: DoubleEndedIterator,
        {
            Self {
                responses: RefCell::new(responses.into_iter().rev().collect()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Http for FakeHttp {
        fn request(
            &self,
            method: &str,
            url: &str,
            body: &[u8],
            headers: &[(String, String)],
        ) -> Result<Response, String> {
            self.calls.borrow_mut().push(Call {
                method: method.to_string(),
                url: url.to_string(),
                body: body.to_vec(),
                headers: headers.to_vec(),
            });
            self.responses
                .borrow_mut()
                .pop()
                .ok_or_else(|| "fake: no canned response".to_string())
        }
    }

    fn xml_response(status: u16, body: &str) -> Response {
        Response {
            status,
            headers: vec![("Content-Type".to_string(), XML_CONTENT_TYPE.to_string())],
            body: body.as_bytes().to_vec(),
        }
    }

    fn header_of<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    #[test]
    fn fetch_events_sends_depth_1_report_and_parses() {
        let fake = FakeHttp::new([xml_response(207, RADICALE)]);
        let items = fetch_events(
            &fake,
            "https://x.ru/cal/user/",
            "20260101T000000Z",
            "20260201T000000Z",
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        let call = &fake.calls.borrow()[0];
        assert_eq!(call.method, "REPORT");
        assert_eq!(call.url, "https://x.ru/cal/user/");
        assert_eq!(header_of(&call.headers, "Depth"), "1");
        assert!(String::from_utf8_lossy(&call.body).contains("calendar-query"));
    }

    #[test]
    fn fetch_contacts_sends_addressbook_report() {
        let fake = FakeHttp::new([xml_response(207, NO_PREFIX)]);
        let items = fetch_contacts(&fake, "https://x.ru/book/").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].etag.as_deref(), Some("\"vc-1\""));
        assert_eq!(fake.calls.borrow()[0].method, "REPORT");
    }

    #[test]
    fn non_207_report_is_an_error_naming_the_status() {
        let fake = FakeHttp::new([xml_response(403, "<html>forbidden</html>")]);
        let err = fetch_events(
            &fake,
            "https://x.ru/cal/",
            "20260101T000000Z",
            "20260201T000000Z",
        )
        .unwrap_err();
        assert!(err.contains("403"), "{err}");
        let fake = FakeHttp::new([xml_response(200, "<ok/>")]);
        let err = fetch_contacts(&fake, "https://x.ru/book/").unwrap_err();
        assert!(err.contains("200"), "{err}");
    }

    #[test]
    fn put_resource_sends_if_match_only_when_given() {
        let with = FakeHttp::one(Response {
            status: 204,
            headers: vec![("ETag".to_string(), "\"new-etag\"".to_string())],
            body: Vec::new(),
        });
        let etag = put_resource(
            &with,
            "https://x.ru/a.ics",
            b"BEGIN:VCALENDAR",
            Some("\"old\""),
        )
        .unwrap();
        assert_eq!(etag, "\"new-etag\"");
        let call = &with.calls.borrow()[0];
        assert_eq!(call.method, "PUT");
        assert_eq!(call.body, b"BEGIN:VCALENDAR");
        assert_eq!(header_of(&call.headers, "If-Match"), "\"old\"");
        assert!(header_of(&call.headers, "Content-Type").contains("text/calendar"));

        let without = FakeHttp::one(Response {
            status: 201,
            headers: Vec::new(),
            body: Vec::new(),
        });
        assert_eq!(
            put_resource(&without, "https://x.ru/b.ics", b"x", None).unwrap(),
            ""
        );
        assert!(header_of(&without.calls.borrow()[0].headers, "If-Match").is_empty());
    }

    #[test]
    fn put_resource_conflict_is_an_error_with_status() {
        let fake = FakeHttp::one(Response {
            status: 412,
            headers: Vec::new(),
            body: Vec::new(),
        });
        let err = put_resource(&fake, "https://x.ru/a.ics", b"x", Some("\"stale\"")).unwrap_err();
        assert!(err.contains("412"), "{err}");
    }

    #[test]
    fn get_resource_captures_etag_and_href() {
        let fake = FakeHttp::one(Response {
            status: 200,
            headers: vec![("ETag".to_string(), "\"g1\"".to_string())],
            body: b"BEGIN:VCARD\nFN:Zoe\nEND:VCARD".to_vec(),
        });
        let item = get_resource(&fake, "https://x.ru/c/z.vcf").unwrap();
        assert_eq!(item.href, "https://x.ru/c/z.vcf");
        assert_eq!(item.etag.as_deref(), Some("\"g1\""));
        assert!(item.data.as_deref().unwrap_or_default().contains("Zoe"));
        assert_eq!(fake.calls.borrow()[0].method, "GET");
    }

    #[test]
    fn probe_returns_dav_header_and_flags_plain_http() {
        let dav = FakeHttp::one(Response {
            status: 200,
            headers: vec![("DAV".to_string(), "1, 3, extended-mkcol".to_string())],
            body: Vec::new(),
        });
        assert_eq!(
            probe(&dav, "https://x.ru/").unwrap(),
            "1, 3, extended-mkcol"
        );
        assert_eq!(dav.calls.borrow()[0].method, "OPTIONS");

        let plain = FakeHttp::one(Response {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        });
        assert_eq!(probe(&plain, "https://x.ru/").unwrap(), "");
        let refused = FakeHttp::one(Response {
            status: 401,
            headers: Vec::new(),
            body: Vec::new(),
        });
        assert!(probe(&refused, "https://x.ru/")
            .unwrap_err()
            .contains("401"));
    }

    #[test]
    fn remote_changed_compares_etags_and_treats_404_as_new() {
        let same = FakeHttp::one(Response {
            status: 200,
            headers: vec![("ETag".to_string(), "\"e1\"".to_string())],
            body: Vec::new(),
        });
        assert_eq!(
            remote_changed(&same, "https://x/db.enc", Some("\"e1\"")).unwrap(),
            None
        );

        let changed = FakeHttp::one(Response {
            status: 200,
            headers: vec![("ETag".to_string(), "\"e2\"".to_string())],
            body: Vec::new(),
        });
        assert_eq!(
            remote_changed(&changed, "https://x/db.enc", Some("\"e1\"")).unwrap(),
            Some("\"e2\"".to_string())
        );

        let missing = FakeHttp::one(Response {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        });
        assert_eq!(
            remote_changed(&missing, "https://x/db.enc", Some("\"e1\"")).unwrap(),
            Some(String::new())
        );

        let broken = FakeHttp::one(Response {
            status: 500,
            headers: Vec::new(),
            body: Vec::new(),
        });
        assert!(remote_changed(&broken, "https://x/db.enc", None)
            .unwrap_err()
            .contains("500"));
    }

    #[test]
    fn download_and_upload_roundtrip_via_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("db.enc");
        std::fs::write(&local, b"encrypted-bytes").unwrap();

        let up = FakeHttp::one(Response {
            status: 201,
            headers: vec![("ETag".to_string(), "\"up1\"".to_string())],
            body: Vec::new(),
        });
        let etag = upload_from(&up, "https://x/db.enc", &local, None).unwrap();
        assert_eq!(etag, "\"up1\"");
        let call = &up.calls.borrow()[0];
        assert_eq!(call.body, b"encrypted-bytes");
        assert_eq!(
            header_of(&call.headers, "Content-Type"),
            "application/octet-stream"
        );

        let down = FakeHttp::one(Response {
            status: 200,
            headers: Vec::new(),
            body: b"remote-bytes".to_vec(),
        });
        let dest = dir.path().join("restored.enc");
        assert_eq!(download_to(&down, "https://x/db.enc", &dest).unwrap(), 12);
        assert_eq!(std::fs::read(&dest).unwrap(), b"remote-bytes");

        let failed = FakeHttp::one(Response {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        });
        assert!(download_to(&failed, "https://x/nope", &dest)
            .unwrap_err()
            .contains("404"));
        assert!(upload_from(&failed, "https://x/nope", &dir.path().join("missing"), None).is_err());
    }
}
