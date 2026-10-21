//! Allowlist HTML sanitizer for rendering untrusted mail HTML (S19).
//!
//! Remote content is blocked by default, so this is the second layer: even
//! when the user loads images, active content must never execute. Strategy:
//! parse tags with a hand-rolled scanner, keep only allowlisted tags and
//! attributes, drop `script`/`style`/`iframe`/... **including their body**,
//! strip event handlers and `javascript:`/`data:` URLs, and drop anything
//! malformed rather than guessing.
//!
//! This is defense in depth, not a substitute for the sandboxed webview —
//! but the reading pane must not become an XSS delivery channel.

use std::fmt::Write as _;

/// Tags whose entire element (including content) is removed.
const DROP_WITH_CONTENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "title", "head", "noscript", "form",
];

/// Tags kept (content preserved, tags re-emitted only if allowlisted).
const KEEP_TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "dd",
    "del",
    "div",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "li",
    "ol",
    "p",
    "pre",
    "s",
    "small",
    "span",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
];

/// Attributes kept per tag ("*" = any kept tag).
const KEEP_ATTRS: &[(&str, &[&str])] = &[
    ("*", &["dir", "lang", "title"]),
    ("a", &["href"]),
    ("img", &["alt", "height", "src", "width"]),
    ("td", &["colspan", "rowspan"]),
    ("th", &["colspan", "rowspan"]),
];

fn allowed_attrs(tag: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (t, attrs) in KEEP_ATTRS {
        if *t == tag || *t == "*" {
            out.extend_from_slice(attrs);
        }
    }
    out
}

/// A URL is safe when it is http(s), mailto, or a bare fragment/relative.
fn safe_url(url: &str) -> bool {
    let lower = url.trim().to_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
        return false;
    }
    if lower.starts_with("data:") {
        // Allow only inline images (remote content is blocked elsewhere).
        return lower.starts_with("data:image/");
    }
    true
}

/// Escape text and attribute values.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Sanitize an HTML document fragment. Never panics on malformed input;
/// malformed tags are dropped as text-free markers.
///
/// One function because the scanner state (`drop_depth`, cursor) is genuinely
/// single-use; splitting it would thread three pieces of state through every
/// helper for no readability gain.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn sanitize_html(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut drop_depth = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            // Copy text up to the next '<'; outside dropping contexts only.
            let end = input[i..].find('<').map_or(input.len(), |p| i + p);
            if drop_depth == 0 {
                out.push_str(&esc(&input[i..end]));
            }
            i = end;
            continue;
        }
        // Stray '<' that can't start a tag is literal text.
        let plausible = bytes
            .get(i + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'/' || *c == b'!');
        if !plausible {
            out.push_str("&lt;");
            i += 1;
            continue;
        }
        // Comment?
        if input[i..].starts_with("<!--") {
            match input[i + 4..].find("-->") {
                Some(p) => i += 4 + p + 3,
                None => break,
            }
            continue;
        }
        let Some(rel) = input[i + 1..].find('>') else {
            // Unterminated tag at end of input: escape the remainder as text.
            out.push_str(&esc(&input[i..]));
            break;
        };
        let tag_end = i + 1 + rel;
        let inner = &input[i + 1..tag_end];
        i = tag_end + 1;

        let (closing, name_part) = match inner.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, inner),
        };
        let name: String = name_part
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase();
        if name.is_empty() {
            continue; // malformed: drop the marker entirely
        }

        if DROP_WITH_CONTENT.contains(&name.as_str()) {
            if closing {
                drop_depth = drop_depth.saturating_sub(1);
            } else if !name_part.ends_with('/') {
                drop_depth += 1;
            }
            continue;
        }
        if drop_depth > 0 {
            continue;
        }
        if !KEEP_TAGS.contains(&name.as_str()) {
            continue; // drop tag, keep its text content
        }
        if closing {
            let _ = write!(out, "</{name}>");
            continue;
        }
        // Attributes: walk `name_part` after the tag name.
        let attrs_str = &name_part[name.len()..];
        let mut kept = String::new();
        let mut rest = attrs_str;
        while let Some(eq) = rest.find('=') {
            let before = &rest[..eq];
            let name_start = before.rfind(char::is_whitespace).map_or(0, |p| p + 1);
            let attr_name = before[name_start..].trim().to_lowercase();
            if attr_name.is_empty() {
                break;
            }
            let after = &rest[eq + 1..];
            let after_trimmed = after.trim_start();
            let (value, value_len) = if let Some(s) = after_trimmed.strip_prefix('"') {
                match s.find('"') {
                    Some(q) => (s[..q].to_string(), q + 2),
                    None => (s.to_string(), s.len()),
                }
            } else if let Some(s) = after_trimmed.strip_prefix('\'') {
                match s.find('\'') {
                    Some(q) => (s[..q].to_string(), q + 2),
                    None => (s.to_string(), s.len()),
                }
            } else {
                let v: String = after_trimmed
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '>')
                    .collect();
                let l = v.len();
                (v, l)
            };
            let lower_val = value.to_lowercase();
            let safe = allowed_attrs(&name).contains(&attr_name.as_str())
                && !attr_name.starts_with("on")
                && (attr_name != "href" && attr_name != "src" || safe_url(&lower_val));
            if safe {
                let _ = write!(kept, " {attr_name}=\"{}\"", esc(&value));
            }
            let advance = eq + 1 + (after.len() - after_trimmed.len()) + value_len;
            if advance == 0 || advance >= rest.len() {
                break;
            }
            rest = &rest[advance..];
        }
        let self_close = if name_part.ends_with('/') { " /" } else { "" };
        let _ = write!(out, "<{name}{kept}{self_close}>");
    }
    out
}

/// Plain text of an HTML fragment: tags dropped, the five named entities and
/// `&#NN;` numeric references decoded. Used by the document exporters.
#[must_use]
pub fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find('<') {
        out.push_str(&xml_unescape(&rest[..open]));
        let Some(close) = rest[open..].find('>') else {
            // Unterminated tag: drop the rest, matching the sanitizer.
            rest = "";
            break;
        };
        rest = &rest[open + close + 1..];
    }
    out.push_str(&xml_unescape(rest));
    out
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(';');
        let decoded = end.and_then(|semi| {
            let ent = &tail[1..semi];
            match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => ent
                    .strip_prefix('#')
                    .and_then(|n| n.parse::<u32>().ok())
                    .and_then(char::from_u32),
            }
            .map(|c| (semi + 1, c))
        });
        if let Some((consumed, ch)) = decoded {
            out.push(ch);
            rest = &tail[consumed..];
        } else {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip_html_drops_tags_and_decodes_entities() {
        assert_eq!(
            super::strip_html("<p>Hello &amp; &lt;world&gt;</p><p>line two</p>"),
            "Hello & <world>line two"
        );
        assert_eq!(super::strip_html("plain"), "plain");
        assert_eq!(super::strip_html("a < b"), "a ");
        assert_eq!(super::strip_html("x&#65;y"), "xAy");
    }

    use super::*;

    #[test]
    fn keeps_safe_formatting_and_links() {
        let src = r#"<p>Hello <b>world</b></p><a href="https://x.example">link</a>"#;
        assert_eq!(sanitize_html(src), src);
    }

    #[test]
    fn drops_script_with_content_and_event_handlers() {
        let out = sanitize_html(r#"<p>a</p><script>alert(1)</script><p onclick="evil()">b</p>"#);
        assert_eq!(out, "<p>a</p><p>b</p>");
    }

    #[test]
    fn drops_style_iframe_object_entirely() {
        let out = sanitize_html(
            r#"<style>p{}</style><iframe src="x"></iframe><object data="y"></object>ok"#,
        );
        assert_eq!(out, "ok");
    }

    #[test]
    fn neutralizes_javascript_urls_but_keeps_https_and_mailto() {
        let out = sanitize_html(
            r#"<a href="javascript:alert(1)">x</a><a href="https://ok.example">y</a><a href="mailto:a@b.c">z</a>"#,
        );
        assert!(out.contains(r#"<a href="https://ok.example">"#));
        assert!(out.contains(r#"<a href="mailto:a@b.c">"#));
        assert!(!out.contains("javascript"));
    }

    #[test]
    fn img_allows_https_and_data_image_only() {
        let out = sanitize_html(
            r#"<img src="https://x/y.png"><img src="data:image/png;base64,AA"><img src="data:text/html,x">"#,
        );
        assert_eq!(
            out.matches("src=").count(),
            2,
            "unsafe src attrs are stripped: {out}"
        );
        assert!(!out.contains("data:text"));
    }

    #[test]
    fn malformed_tags_are_dropped_not_interpreted() {
        let out = sanitize_html("a < b <img src=x onerror=alert(1)");
        // Stray '<' and the unterminated tag survive only as escaped TEXT.
        assert!(out.contains("a &lt; b"));
        assert!(
            !out.contains("<img"),
            "unterminated tag must not be emitted: {out}"
        );
        assert!(out.contains("&lt;img"));
    }

    #[test]
    fn unterminated_script_drops_rest_of_document() {
        let out = sanitize_html("<p>safe</p><script>alert(1)");
        assert_eq!(out, "<p>safe</p>");
    }

    #[test]
    fn comments_removed() {
        let out = sanitize_html("a<!-- hidden --><b>c</b>");
        assert_eq!(out, "a<b>c</b>");
    }

    #[test]
    fn unknown_tags_dropped_content_kept() {
        let out = sanitize_html("<blink>old</blink><p>new</p>");
        assert_eq!(out, "old<p>new</p>");
    }
}
