//! CSV engine: RFC 4180 parsing (via the `csv` crate), header mapping with
//! alias suggestions, and row validation for the import wizard (S13, T1.6).

use std::collections::HashMap;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct CsvData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Parse CSV (comma or `;` sniffed from the first line) into headers + rows.
pub fn parse_csv(bytes: &[u8]) -> Result<CsvData> {
    let first_line = std::str::from_utf8(bytes)
        .map_err(|e| Error::parse("csv", e.to_string()))?
        .lines()
        .next()
        .unwrap_or("");
    let delimiter = sniff_delimiter(first_line);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter as u8)
        .from_reader(bytes);
    let mut records = reader.records();
    let Some(first) = records.next() else {
        return Err(Error::parse("csv", "file is empty"));
    };
    let first = first.map_err(Error::from)?;
    let headers: Vec<String> = first.iter().map(str::to_string).collect();
    let mut rows = Vec::new();
    for rec in records {
        let rec = rec.map_err(Error::from)?;
        rows.push(rec.iter().map(str::to_string).collect());
    }
    Ok(CsvData { headers, rows })
}

/// Very small delimiter sniffer: counts candidates in the first line.
#[must_use]
pub fn sniff_delimiter(first_line: &str) -> char {
    let commas = first_line.matches(',').count();
    let semis = first_line.matches(';').count();
    let tabs = first_line.matches('\t').count();
    if tabs > commas && tabs > semis {
        '\t'
    } else if semis > commas {
        ';'
    } else {
        ','
    }
}

/// Column → target-field mapping chosen in the import wizard.
pub type ColumnMap = HashMap<String, usize>; // field name -> column index

/// Suggest a mapping by matching known field aliases against headers
/// (case-insensitive, whitespace-normalized).
#[must_use]
pub fn suggest_mapping(headers: &[String], fields: &[(&str, &[&str])]) -> ColumnMap {
    let norm = |s: &str| s.trim().to_lowercase().replace([' ', '-', '_'], "");
    let normalized: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    let mut map = ColumnMap::new();
    for (field, aliases) in fields {
        for (i, h) in normalized.iter().enumerate() {
            if aliases.iter().any(|a| *a == h) && !map.values().any(|&v| v == i) {
                map.insert((*field).to_string(), i);
                break;
            }
        }
    }
    map
}

/// Contacts field aliases (S46/S49); required = at least a name part or email.
#[must_use]
pub fn contact_fields() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("first_name", &["firstname", "givenname", "first"]),
        ("last_name", &["lastname", "surname", "familyname", "last"]),
        ("display_name", &["displayname", "fullname", "name"]),
        ("organization", &["organization", "company", "employer"]),
        ("email", &["email", "emailaddress", "mail", "email1"]),
        ("email2", &["email2", "secondaryemail"]),
        ("phone", &["phone", "phonenumber", "tel", "mobile", "cell"]),
        ("address", &["address", "street", "streetaddress"]),
        ("url", &["url", "website", "webpage", "homepage"]),
        ("birthday", &["birthday", "birthdate", "dob"]),
        ("notes", &["notes", "note", "comment", "comments"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_semicolon_csv() {
        let data =
            parse_csv(b"Name;Email;Notes\n\"Doe, Jane\";j@x.com;\"line1\nline2\"\n").unwrap();
        assert_eq!(data.headers, vec!["Name", "Email", "Notes"]);
        assert_eq!(data.rows.len(), 1);
        assert_eq!(data.rows[0][0], "Doe, Jane");
        assert!(data.rows[0][2].contains('\n'));
    }

    #[test]
    fn empty_file_is_error() {
        assert!(parse_csv(b"").is_err());
    }

    #[test]
    fn suggests_contact_mapping() {
        let map = suggest_mapping(
            &[
                "Full Name".into(),
                "E-Mail Address".into(),
                "Company".into(),
            ],
            &contact_fields(),
        );
        assert_eq!(map["display_name"], 0);
        assert_eq!(map["email"], 1);
        assert_eq!(map["organization"], 2);
        assert!(!map.contains_key("birthday"));
    }

    #[test]
    fn sniffs_delimiters() {
        assert_eq!(sniff_delimiter("a;b;c"), ';');
        assert_eq!(sniff_delimiter("a,b,c"), ',');
        assert_eq!(sniff_delimiter("a\tb\tc"), '\t');
    }
}
