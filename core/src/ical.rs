//! ICS (RFC 5545) engine: unfold/parse VEVENTs, serialize events, and expand
//! RRULE recurrence over a read window (T1.7). TZID handling via chrono-tz;
//! unknown TZIDs produce a warning instead of dropping the event.

use chrono::{TimeZone, Utc};
use chrono_tz::Tz;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct EventRecord {
    pub uid: Option<String>,
    pub title: String,
    /// RFC 3339 UTC (or floating local ISO when no zone was given).
    pub start: String,
    pub end: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    pub rrule: Option<String>,
    pub categories: Vec<String>,
    pub private: bool,
}

/// A parsed calendar: events plus non-fatal issues for the validation report.
#[derive(Debug, Default)]
pub struct ParsedCalendar {
    pub events: Vec<EventRecord>,
    pub warnings: Vec<String>,
}

/// Unfold RFC 5545 continuation lines (`\r\n` or `\n` followed by space/tab).
#[must_use]
pub fn unfold(input: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in input.lines() {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push_str(&raw[1..]);
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    lines
}

fn parse_ics_dt(value: &str, tzid: Option<&str>, all_day: &mut bool) -> Option<String> {
    let v = value.trim();
    if v.len() > 8 && v.as_bytes()[8] == b'T' {
        let (date, time) = v.split_at(8);
        let time = &time[1..];
        if let Some(stripped) = time.strip_suffix('Z') {
            return naive_to_rfc3339_utc(date, stripped, None);
        }
        if let Some(tzid) = tzid {
            if let Ok(tz) = tzid.parse::<Tz>() {
                let dt = chrono::NaiveDateTime::parse_from_str(
                    &format!("{date}T{time}"),
                    "%Y%m%dT%H%M%S",
                )
                .ok()?;
                return Some(
                    tz.from_local_datetime(&dt)
                        .earliest()?
                        .with_timezone(&Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                );
            }
        }
        // floating local time — store naive ISO, interpreted as local later
        *all_day = false;
        return Some(format!("{date}T{time}"));
    }
    // VALUE=DATE
    *all_day = true;
    let d = chrono::NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
    Some(d.format("%Y-%m-%d").to_string())
}

fn naive_to_rfc3339_utc(date: &str, time: &str, _none: Option<()>) -> Option<String> {
    let dt =
        chrono::NaiveDateTime::parse_from_str(&format!("{date}T{time}"), "%Y%m%dT%H%M%S").ok()?;
    Some(
        Utc.from_utc_datetime(&dt)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Parse an ICS file body into events. Never fails on a single bad property —
/// collects warnings instead (report-friendly, spec S13).
pub fn parse_ics(input: &str) -> ParsedCalendar {
    let mut out = ParsedCalendar::default();
    let mut current: Option<EventRecord> = None;
    for line in unfold(input) {
        let Some((name_part, value)) = line.split_once(':') else {
            continue;
        };
        let mut segs = name_part.split(';');
        let name = segs.next().unwrap_or("").to_uppercase();
        let params: Vec<(String, String)> = segs
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_uppercase(), v.to_string()))
            .collect();
        let tzid = params
            .iter()
            .find(|(k, _)| k == "TZID")
            .map(|(_, v)| v.clone());
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => {
                current = Some(EventRecord::default());
            }
            "END" if value.eq_ignore_ascii_case("VEVENT") => {
                if let Some(ev) = current.take() {
                    if ev.title.is_empty() {
                        out.warnings.push("skipped VEVENT without SUMMARY".into());
                    } else {
                        out.events.push(ev);
                    }
                }
            }
            _ => {
                let Some(ev) = current.as_mut() else { continue };
                match name.as_str() {
                    "SUMMARY" => ev.title = unescape_text(value),
                    "UID" => ev.uid = Some(value.to_string()),
                    "LOCATION" => ev.location = Some(unescape_text(value)),
                    "DESCRIPTION" => ev.description = Some(unescape_text(value)),
                    "RRULE" => ev.rrule = Some(value.to_string()),
                    "CLASS" => ev.private = value.eq_ignore_ascii_case("PRIVATE"),
                    "CATEGORIES" => {
                        ev.categories = value.split(',').map(|c| unescape_text(c.trim())).collect();
                    }
                    "DTSTART" => {
                        let mut all_day = false;
                        if let Some(s) = parse_ics_dt(value, tzid.as_deref(), &mut all_day) {
                            ev.start = s;
                            ev.all_day =
                                all_day || params.iter().any(|(k, v)| k == "VALUE" && v == "DATE");
                        } else {
                            out.warnings.push(format!("unparseable DTSTART: {value}"));
                        }
                    }
                    "DTEND" => {
                        let mut all_day = false;
                        if let Some(s) = parse_ics_dt(value, tzid.as_deref(), &mut all_day) {
                            ev.end = Some(s);
                        } else {
                            out.warnings.push(format!("unparseable DTEND: {value}"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            c => out.push(c),
        }
    }
    out
}

fn unescape_text(s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\;", ";")
        .replace("\\,", ",")
        .replace("\\\\", "\\")
}

fn ics_utc(iso: &str, all_day: bool) -> String {
    if all_day {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d") {
            return d.format("%Y%m%d").to_string();
        }
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        return dt.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string();
    }
    if let Ok(nd) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S") {
        return nd.format("%Y%m%dT%H%M%S").to_string();
    }
    String::new()
}

/// Serialize records as a VCALENDAR (UTC times, VALUE=DATE for all-day).
#[must_use]
pub fn write_ics(events: &[EventRecord], cal_name: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(256 + events.len() * 160);
    let _ = write!(
        out,
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Smart PIM//Core//EN\r\nX-WR-CALNAME:{}\r\n",
        escape_text(cal_name)
    );
    for ev in events {
        let date_param = if ev.all_day { ";VALUE=DATE" } else { "" };
        let _ = write!(
            out,
            "BEGIN:VEVENT\r\nUID:{}\r\n",
            ev.uid.clone().unwrap_or_default()
        );
        let _ = write!(out, "SUMMARY:{}\r\n", escape_text(&ev.title));
        let _ = write!(
            out,
            "DTSTART{date_param}:{}\r\n",
            ics_utc(&ev.start, ev.all_day)
        );
        if let Some(end) = &ev.end {
            let _ = write!(out, "DTEND{date_param}:{}\r\n", ics_utc(end, ev.all_day));
        }
        if let Some(rr) = &ev.rrule {
            let _ = write!(out, "RRULE:{rr}\r\n");
        }
        if let Some(loc) = &ev.location {
            let _ = write!(out, "LOCATION:{}\r\n", escape_text(loc));
        }
        if let Some(desc) = &ev.description {
            let _ = write!(out, "DESCRIPTION:{}\r\n", escape_text(desc));
        }
        if !ev.categories.is_empty() {
            let cats: Vec<String> = ev.categories.iter().map(|c| escape_text(c)).collect();
            let _ = write!(out, "CATEGORIES:{}\r\n", cats.join(","));
        }
        if ev.private {
            out.push_str("CLASS:PRIVATE\r\n");
        }
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

/// Expand `RRULE` over `[from, to]` (RFC 3339 UTC), capped at `cap`
/// occurrences (data-model invariant: recurrence is computed, never stored).
/// Falls back to the single start date when the rule cannot be parsed.
pub fn expand_rrule(
    rrule: &str,
    start_utc: &str,
    from_utc: &str,
    to_utc: &str,
    cap: usize,
) -> Result<Vec<String>> {
    use rrule::RRuleSet;
    let start = parse_utc(start_utc).ok_or_else(|| Error::parse("rrule", "bad start time"))?;
    let from = parse_utc(from_utc).ok_or_else(|| Error::parse("rrule", "bad window start"))?;
    let to = parse_utc(to_utc).ok_or_else(|| Error::parse("rrule", "bad window end"))?;
    let dtstart = start.format("%Y%m%dT%H%M%SZ").to_string();
    let set: RRuleSet = format!("DTSTART:{dtstart}\nRRULE:{rrule}")
        .parse()
        .map_err(|e: rrule::RRuleError| Error::parse("rrule", e.to_string()))?;
    let occ = set.all(u16::try_from(cap.min(730)).unwrap_or(730)).dates;
    let mut out = Vec::new();
    for dt in occ {
        let dt = dt.with_timezone(&Utc);
        if dt >= from && dt <= to {
            out.push(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        }
        if out.len() >= cap {
            break;
        }
    }
    Ok(out)
}

fn parse_utc(s: &str) -> Option<chrono::DateTime<Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    let nd = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Some(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IssueSeverity;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:abc-123\r\nSUMMARY:Team Briefing\r\nDTSTART:20260401T093000Z\r\nDTEND:20260401T103000Z\r\nLOCATION:Room 2\r\nDESCRIPTION:line one\\nline two\r\nCATEGORIES:Work,Planning\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Offsite\r\nDTSTART;VALUE=DATE:20260510\r\nDTEND;VALUE=DATE:20260511\r\nCLASS:PRIVATE\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Berlin sync\r\nDTSTART;TZID=Europe/Berlin:20260601T100000\r\nDURATION:PT1H\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parses_events_all_day_and_tzid() {
        let cal = parse_ics(SAMPLE);
        assert!(cal.warnings.is_empty(), "{:?}", cal.warnings);
        assert_eq!(cal.events.len(), 3);
        let e0 = &cal.events[0];
        assert_eq!(e0.title, "Team Briefing");
        assert_eq!(e0.start, "2026-04-01T09:30:00Z");
        assert_eq!(e0.categories, vec!["Work", "Planning"]);
        assert_eq!(e0.description.as_deref(), Some("line one\nline two"));
        let e1 = &cal.events[1];
        assert!(e1.all_day);
        assert!(e1.private);
        assert_eq!(e1.start, "2026-05-10");
        let e2 = &cal.events[2];
        assert!(
            e2.start.ends_with('Z'),
            "TZID converted to UTC: {}",
            e2.start
        );
    }

    #[test]
    fn roundtrip_write_parse() {
        let cal = parse_ics(SAMPLE);
        let text = write_ics(&cal.events, "Work");
        let reparsed = parse_ics(&text);
        assert_eq!(reparsed.events.len(), cal.events.len());
        assert_eq!(reparsed.events[0].title, "Team Briefing");
        assert_eq!(reparsed.events[1].start, "2026-05-10");
        assert!(text.contains("BEGIN:VCALENDAR"));
    }

    #[test]
    fn unfolds_continuation_lines() {
        let cal = parse_ics("BEGIN:VEVENT\r\nSUMMARY:very long\r\n title that continues\r\nDTSTART:20260101T000000Z\r\nEND:VEVENT\r\n");
        assert_eq!(cal.events[0].title, "very longtitle that continues");
    }

    #[test]
    fn expands_daily_rule() {
        let occ = expand_rrule(
            "FREQ=DAILY;COUNT=5",
            "2026-03-30T10:00:00+00:00",
            "2026-03-30T00:00:00+00:00",
            "2026-04-05T00:00:00+00:00",
            100,
        )
        .unwrap();
        assert_eq!(occ.len(), 5);
        assert!(occ[0].starts_with("2026-03-30T10:00:00Z"), "{}", occ[0]);
    }

    #[test]
    fn bad_rrule_is_reported_not_panicked() {
        assert!(expand_rrule(
            "FREQ=NOPE",
            "2026-03-30T10:00:00+00:00",
            "2026-03-30T00:00:00+00:00",
            "2026-04-05T00:00:00+00:00",
            10
        )
        .is_err());
    }

    #[test]
    fn severity_of_warning_imports() {
        let mut report = crate::import::ImportReport::default();
        for w in parse_ics("BEGIN:VEVENT\r\nDTSTART:garbage\r\nEND:VEVENT\r\n").warnings {
            report.warn(0, w);
        }
        assert!(!report.has_errors());
        assert_eq!(report.issues[0].severity, IssueSeverity::Warning);
    }
}
