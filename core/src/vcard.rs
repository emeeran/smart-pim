//! vCard 3.0/4.0 engine (T1.8): parse VCF into contact records, serialize
//! contacts back out. Photos are skipped with a warning (hash-addressed media
//! store handles them at the attachment layer, not inline base64).

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContactRecord {
    pub uid: Option<String>,
    /// Store row id (0 for records not yet persisted); not serialized to VCF.
    #[serde(skip)]
    pub id: i64,
    /// Item-table id — used by the UI for related-item links. `default` keeps
    /// VCF/JSON deserialization source-compatible.
    #[serde(default)]
    pub item_id: i64,
    pub first_name: String,
    pub last_name: String,
    pub display_name: String,
    pub organization: Option<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub addresses: Vec<String>,
    pub urls: Vec<String>,
    pub birthday: Option<String>, // YYYY-MM-DD when parseable
    pub notes: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ParsedVcf {
    pub contacts: Vec<ContactRecord>,
    pub warnings: Vec<String>,
}

/// Parse a VCF body. Malformed properties become warnings, not errors.
pub fn parse_vcf(input: &str) -> ParsedVcf {
    let mut out = ParsedVcf::default();
    let mut current: Option<ContactRecord> = None;
    for line in crate::ical::unfold(input) {
        let Some((name_part, value)) = line.split_once(':') else {
            continue;
        };
        let mut segs = name_part.split(';');
        let name = segs.next().unwrap_or("").to_uppercase();
        let params: Vec<(String, String)> = segs
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_uppercase(), v.to_uppercase()))
            .collect();
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => {
                current = Some(ContactRecord::default());
            }
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                if let Some(c) = current.take() {
                    if c.display_name.is_empty()
                        && c.first_name.is_empty()
                        && c.last_name.is_empty()
                        && c.emails.is_empty()
                    {
                        out.warnings
                            .push("skipped VCARD without name or email".into());
                    } else {
                        out.contacts.push(c);
                    }
                }
            }
            "PHOTO" => {
                if current.is_some() {
                    out.warnings
                        .push("PHOTO skipped (media store handles images)".into());
                }
            }
            _ => {
                let Some(c) = current.as_mut() else { continue };
                match name.as_str() {
                    "FN" => c.display_name = value.to_string(),
                    "N" => {
                        // N:Last;First;Middle;Prefix;Suffix
                        let mut parts = value.split(';');
                        c.last_name = parts.next().unwrap_or("").to_string();
                        c.first_name = parts.next().unwrap_or("").to_string();
                    }
                    "ORG" => {
                        c.organization = value
                            .split(';')
                            .next()
                            .map(str::to_string)
                            .filter(|s| !s.is_empty());
                    }
                    "EMAIL" => c.emails.push(value.trim().to_string()),
                    "TEL" => c.phones.push(value.trim().to_string()),
                    "ADR" => {
                        // ADR:po;ext;street;locality;region;postal;country
                        let joined = value.split(';').skip(2).collect::<Vec<_>>().join(" ");
                        let cleaned = joined.split_whitespace().collect::<Vec<_>>().join(" ");
                        if !cleaned.is_empty() {
                            c.addresses.push(cleaned);
                        }
                    }
                    "URL" => c.urls.push(value.trim().to_string()),
                    "BDAY" | "ANNIVERSARY" => {
                        if let Some(d) = normalize_date(value) {
                            if name == "BDAY" {
                                c.birthday = Some(d);
                            }
                        } else {
                            out.warnings.push(format!("unparseable {name}: {value}"));
                        }
                    }
                    "NOTE" => c.notes = Some(value.to_string()),
                    "CATEGORIES" => {
                        c.categories = value
                            .split(',')
                            .map(str::trim)
                            .map(str::to_string)
                            .collect();
                    }
                    "UID" => c.uid = Some(value.to_string()),
                    _ => {}
                }
                let _ = params;
            }
        }
    }
    out
}

fn normalize_date(v: &str) -> Option<String> {
    let v = v.trim();
    if let Ok(d) = chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(v, "%Y%m%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(v, "%d/%m/%Y") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    None
}

fn esc(s: &str) -> String {
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

/// Serialize records as VCF 4.0.
#[must_use]
pub fn write_vcf(contacts: &[ContactRecord]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64 + contacts.len() * 160);
    for c in contacts {
        let _ = write!(out, "BEGIN:VCARD\r\nVERSION:4.0\r\n");
        let _ = write!(out, "FN:{}\r\n", esc(&c.display_name));
        let _ = write!(out, "N:{};{};;;\r\n", esc(&c.last_name), esc(&c.first_name));
        if let Some(org) = &c.organization {
            let _ = write!(out, "ORG:{}\r\n", esc(org));
        }
        for e in &c.emails {
            let _ = write!(out, "EMAIL:{}\r\n", esc(e));
        }
        for p in &c.phones {
            let _ = write!(out, "TEL:{}\r\n", esc(p));
        }
        for a in &c.addresses {
            let _ = write!(out, "ADR:;;{}\r\n", esc(a));
        }
        for u in &c.urls {
            let _ = write!(out, "URL:{}\r\n", esc(u));
        }
        if let Some(b) = &c.birthday {
            let _ = write!(out, "BDAY:{b}\r\n");
        }
        if let Some(n) = &c.notes {
            let _ = write!(out, "NOTE:{}\r\n", esc(n));
        }
        if !c.categories.is_empty() {
            let cats: Vec<String> = c.categories.iter().map(|cat| esc(cat)).collect();
            let _ = write!(out, "CATEGORIES:{}\r\n", cats.join(","));
        }
        if let Some(uid) = &c.uid {
            let _ = write!(out, "UID:{uid}\r\n");
        }
        out.push_str("END:VCARD\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vcard3() {
        let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Powell;Marie;;;\r\nFN:Marie Powell\r\nORG:Example;Engineering\r\nTEL;TYPE=CELL:+1 555 901 2345\r\nEMAIL;TYPE=INTERNET:mariep@example.com\r\nEMAIL:marie@example.org\r\nADR:;;1 Main St;Springfield;IL;12345;USA\r\nURL:https://example.com\r\nBDAY:19740301\r\nCATEGORIES:Management,Team\r\nEND:VCARD\r\n";
        let parsed = parse_vcf(vcf);
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.contacts.len(), 1);
        let c = &parsed.contacts[0];
        assert_eq!(c.first_name, "Marie");
        assert_eq!(c.last_name, "Powell");
        assert_eq!(c.organization.as_deref(), Some("Example"));
        assert_eq!(c.emails.len(), 2);
        assert_eq!(c.phones, vec!["+1 555 901 2345"]);
        assert!(c.addresses[0].contains("Springfield"));
        assert_eq!(c.birthday.as_deref(), Some("1974-03-01"));
        assert_eq!(c.categories, vec!["Management", "Team"]);
    }

    #[test]
    fn photo_produces_warning_not_error() {
        let parsed = parse_vcf(
            "BEGIN:VCARD\r\nFN:X\r\nPHOTO;ENCODING=BASE64;TYPE=JPEG:aGVsbG8=\r\nEND:VCARD\r\n",
        );
        assert_eq!(parsed.contacts.len(), 1);
        assert!(parsed.warnings.iter().any(|w| w.contains("PHOTO")));
    }

    #[test]
    fn roundtrip() {
        let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Chase;Sylvia;;;\r\nFN:Sylvia Chase\r\nEMAIL:sylvia@example.com\r\nEND:VCARD\r\n";
        let parsed = parse_vcf(vcf);
        let text = write_vcf(&parsed.contacts);
        let reparsed = parse_vcf(&text);
        assert_eq!(reparsed.contacts[0].display_name, "Sylvia Chase");
        assert_eq!(reparsed.contacts[0].emails, vec!["sylvia@example.com"]);
    }

    #[test]
    fn empty_vcard_is_skipped_with_warning() {
        let parsed = parse_vcf("BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n");
        assert_eq!(parsed.contacts, [] as [ContactRecord; 0]);
        assert_eq!(parsed.warnings.len(), 1);
    }
}
