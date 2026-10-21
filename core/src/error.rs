use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
    #[error("key derivation error: {0}")]
    Kdf(String),
    #[error("cannot open database: wrong passphrase or corrupted file")]
    Unlock,
    #[error("parse error in {what}: {message}")]
    Parse { what: &'static str, message: String },
    #[error("validation failed at row {row}: {message}")]
    Row { row: usize, message: String },
    #[error("item {0} not found")]
    NotFound(i64),
}

impl Error {
    #[must_use]
    pub fn parse(what: &'static str, message: impl fmt::Display) -> Self {
        Self::Parse {
            what,
            message: message.to_string(),
        }
    }
}

/// A row-level problem found during import validation (dry run) or commit.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RowIssue {
    pub row: usize,
    pub severity: IssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

impl fmt::Display for RowIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "row {}: {:?}: {}", self.row, self.severity, self.message)
    }
}
