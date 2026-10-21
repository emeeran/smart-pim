//! Import framework (spec S13): every import is
//! parse → validate (dry-run report) → commit in ONE transaction.
//! The source file is never modified; any error-severity issue aborts the
//! whole import leaving the database byte-identical.

use rusqlite::Connection;

use crate::error::{IssueSeverity, Result, RowIssue};

/// Outcome of a dry run / commit, serialized to the UI as the validation
/// report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub total_rows: usize,
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
    pub issues: Vec<RowIssue>,
}

impl ImportReport {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }

    pub fn error(&mut self, row: usize, message: impl Into<String>) {
        self.issues.push(RowIssue {
            row,
            severity: IssueSeverity::Error,
            message: message.into(),
        });
    }

    pub fn warn(&mut self, row: usize, message: impl Into<String>) {
        self.issues.push(RowIssue {
            row,
            severity: IssueSeverity::Warning,
            message: message.into(),
        });
    }
}

/// Validate only — the dry run handed to the user before commit.
pub fn validate_only<T, F>(rows: &[T], validate: F) -> ImportReport
where
    F: Fn(usize, &T, &mut ImportReport),
{
    let mut report = ImportReport {
        total_rows: rows.len(),
        ..Default::default()
    };
    for (i, row) in rows.iter().enumerate() {
        validate(i, row, &mut report);
    }
    report
}

/// Run `apply` for every row inside a single transaction. If `apply` returns
/// an error the transaction is rolled back and the report carries the issue,
/// so callers can never end up with a partial import.
pub fn commit_all<T, F>(conn: &mut Connection, rows: &[T], mut apply: F) -> Result<ImportReport>
where
    F: FnMut(&Connection, usize, &T, &mut ImportReport) -> Result<()>,
{
    let mut report = ImportReport {
        total_rows: rows.len(),
        ..Default::default()
    };
    let tx = conn.transaction()?;
    let applied: std::result::Result<(), (usize, crate::error::Error)> = (|| {
        for (i, row) in rows.iter().enumerate() {
            if let Err(e) = apply(&tx, i, row, &mut report) {
                return Err((i, e));
            }
        }
        Ok(())
    })();
    match applied {
        Ok(()) => {
            tx.commit()?;
            Ok(report)
        }
        Err((row, e)) => {
            // Transaction drops -> rollback. Report reflects nothing was written.
            report = ImportReport {
                total_rows: rows.len(),
                ..Default::default()
            };
            report.error(row, e.to_string());
            Ok(report)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn failed_row_aborts_everything() {
        let mut store = Store::open_memory("x").unwrap();
        let rows = vec![1, 2, 3];
        let report = commit_all(store.conn_mut(), &rows, |conn, i, n, report| {
            conn.execute("INSERT INTO tag (name) VALUES (?1)", [format!("t{n}")])?;
            if i == 2 {
                report.error(i, "simulated bad row");
                return Err(crate::error::Error::Row {
                    row: i,
                    message: "boom".into(),
                });
            }
            Ok(())
        })
        .unwrap();
        assert!(report.has_errors());
        let n: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "rollback must remove earlier inserts");
    }

    #[test]
    fn clean_import_commits_all() {
        let mut store = Store::open_memory("x").unwrap();
        let rows = vec!["a", "b"];
        let report = commit_all(store.conn_mut(), &rows, |conn, _i, n, report| {
            conn.execute("INSERT INTO tag (name) VALUES (?1)", [n])?;
            report.inserted += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(report.inserted, 2);
        let n: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn validate_only_collects_issues() {
        let report = validate_only(&["ok", ""], |i, row, report| {
            if row.is_empty() {
                report.error(i, "empty name");
            }
        });
        assert_eq!(report.total_rows, 2);
        assert!(report.has_errors());
        assert_eq!(report.issues[0].row, 1);
    }
}
