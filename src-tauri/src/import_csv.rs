//! CSV import pipeline (S13 / T3.44 backend): parse → suggested mapping →
//! dry-run validation report → single-transaction commit. The source file
//! is never modified; a failing row aborts the whole import.

use std::collections::HashMap;

use pfcore::contacts;
use pfcore::csvio::{self, CsvData};
use pfcore::import::{self, ImportReport};
use pfcore::vcard::ContactRecord;
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CsvPreview {
    pub headers: Vec<String>,
    pub sample: Vec<Vec<String>>,
    pub total_rows: usize,
    /// Suggested `contact field -> column index` mapping.
    pub mapping: HashMap<String, usize>,
    pub contact_fields: Vec<String>,
}

pub fn preview(text: &str) -> pfcore::Result<CsvPreview> {
    let data = csvio::parse_csv(text.as_bytes())?;
    let mapping = csvio::suggest_mapping(&data.headers, &csvio::contact_fields());
    Ok(CsvPreview {
        sample: data.rows.iter().take(5).cloned().collect(),
        headers: data.headers.clone(),
        total_rows: data.rows.len(),
        mapping,
        contact_fields: csvio::contact_fields()
            .iter()
            .map(|(f, _)| f.to_string())
            .collect(),
    })
}

fn cell(data: &CsvData, map: &HashMap<String, usize>, row_idx: usize, field: &str) -> String {
    map.get(field)
        .and_then(|col| data.rows.get(row_idx).and_then(|r| r.get(*col)))
        .map_or(String::new(), String::clone)
}

/// Dry-run validation with the chosen mapping. Never touches the store.
pub fn validate(text: &str, map: &HashMap<String, usize>) -> pfcore::Result<ImportReport> {
    let data = csvio::parse_csv(text.as_bytes())?;
    Ok(import::validate_only(&data.rows, |i, _row, report| {
        let name = cell(&data, map, i, "display_name");
        let email = cell(&data, map, i, "email");
        if name.is_empty() && email.is_empty() {
            report.error(i, "row has neither a name nor an email");
        } else if name.is_empty() {
            report.warn(i, "no display name; derived from email");
        }
        for field in ["email", "phone"] {
            let v = cell(&data, map, i, field);
            if !v.is_empty() && v.contains('@') == (field == "phone") {
                report.warn(i, format!("{field} value looks wrong: {v}"));
            }
        }
    }))
}

/// Commit in ONE transaction; any error row rolls back everything.
pub fn commit(
    conn: &mut Connection,
    text: &str,
    map: &HashMap<String, usize>,
) -> pfcore::Result<ImportReport> {
    let data = csvio::parse_csv(text.as_bytes())?;
    import::commit_all(conn, &data.rows, |tx, i, _row, report| {
        let name = cell(&data, map, i, "display_name");
        let email = cell(&data, map, i, "email");
        if name.is_empty() && email.is_empty() {
            report.error(i, "row has neither a name nor an email");
            return Err(pfcore::Error::Row {
                row: i,
                message: "empty row".into(),
            });
        }
        let mut record = ContactRecord {
            uid: Some(format!("csv-import-{i}-{}", data.rows.len())),
            display_name: if name.is_empty() { email.clone() } else { name },
            ..Default::default()
        };
        if !email.is_empty() {
            record.emails.push(email);
        }
        let phone = cell(&data, map, i, "phone");
        if !phone.is_empty() {
            record.phones.push(phone);
        }
        let org = cell(&data, map, i, "organization");
        if !org.is_empty() {
            record.organization = Some(org);
        }
        contacts::upsert_contact(tx, &record)?;
        report.inserted += 1;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfcore::store::Store;

    const CSV: &str = "Full Name,E-Mail Address,Phone,Company\nAda Lovelace,ada@example.com,+44 1,Analytical Engines\n,bad@example.com,,\n";

    #[test]
    fn preview_suggests_mapping() {
        let p = preview(CSV).unwrap();
        assert_eq!(p.total_rows, 2);
        assert_eq!(p.mapping.get("display_name"), Some(&0));
        assert!(p.mapping.contains_key("email"));
        assert_eq!(p.sample.len(), 2);
    }

    #[test]
    fn validate_reports_and_commit_is_atomic() {
        let p = preview(CSV).unwrap();
        // Row 2 has only an email → acceptable with a warning, not an error.
        let report = validate(CSV, &p.mapping).unwrap();
        assert!(!report.has_errors());
        assert_eq!(
            report.issues.len(),
            1,
            "row 2 gets the derived-name warning"
        );

        let bad = "Full Name,E-Mail Address\n,\n";
        let p2 = preview(bad).unwrap();
        let report2 = validate(bad, &p2.mapping).unwrap();
        assert!(report2.has_errors());

        // Atomic commit: the bad row aborts everything (report carries the error).
        let mut store = Store::open_memory("x").unwrap();
        let report3 = commit(store.conn_mut(), bad, &p2.mapping).unwrap();
        assert!(report3.has_errors());
        let n: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM contact", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "a failing row rolls back the whole import");

        // Good import lands all rows.
        let ok = commit(store.conn_mut(), CSV, &p.mapping).unwrap();
        assert_eq!(ok.inserted, 2);
    }
}
