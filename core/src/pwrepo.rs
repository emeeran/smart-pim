//! Passwords repository (T3.39–T3.41 backend): entries, generator, strength
//! heuristic. `password_enc` lives inside the SQLCipher-encrypted database;
//! the column name refers to future per-item encryption (P1, with export
//! lockdown) — no additional transform is applied in v1.

use rand::Rng;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::Kind;
use crate::repo;

#[derive(Debug, Clone, Serialize)]
pub struct PwRow {
    pub id: i64,
    pub item_id: i64,
    pub title: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub strength: Option<i64>,
}

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<PwRow> {
    Ok(PwRow {
        id: r.get(0)?,
        item_id: r.get(1)?,
        title: r.get(2)?,
        username: r.get(3)?,
        password: r.get(4)?,
        url: r.get(5)?,
        notes: r.get(6)?,
        strength: r.get(7)?,
    })
}

pub fn list_entries(conn: &Connection, limit: i64) -> Result<Vec<PwRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.item_id, p.title, p.username, p.password_enc, p.url, p.notes, p.strength \
         FROM pw_entry p JOIN item i ON i.id = p.item_id \
         WHERE i.deleted_at IS NULL ORDER BY p.title COLLATE NOCASE LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], row_from)?;
    Ok(rows.flatten().collect())
}

pub fn save_entry(
    conn: &Connection,
    id: Option<i64>,
    title: &str,
    username: Option<&str>,
    password: Option<&str>,
    url: Option<&str>,
    notes: Option<&str>,
) -> Result<i64> {
    let strength = password.map(strength_score);
    if let Some(id) = id {
        let existing: i64 = conn
            .query_row("SELECT id FROM pw_entry WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or(Error::NotFound(id))?;
        let item_id: i64 = conn.query_row(
            "SELECT item_id FROM pw_entry WHERE id = ?1",
            [existing],
            |r| r.get(0),
        )?;
        let item = repo::get_item(conn, item_id)?;
        repo::snapshot_item(conn, &item)?;
        conn.execute(
            "UPDATE pw_entry SET title=?2, username=?3, password_enc=?4, url=?5, notes=?6, \
             strength=?7 WHERE id=?1",
            params![existing, title, username, password, url, notes, strength],
        )?;
        let _ = repo::touch_item(conn, item_id);
        return Ok(existing);
    }
    let item_id = repo::create_item(conn, Kind::Password, None, false)?;
    conn.execute(
        "INSERT INTO pw_entry (item_id, title, username, password_enc, url, notes, strength) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![item_id, title, username, password, url, notes, strength],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_entry(conn: &Connection, id: i64) -> Result<()> {
    let item_id: i64 = conn
        .query_row("SELECT item_id FROM pw_entry WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(Error::NotFound(id))?;
    repo::trash_item(conn, item_id)
}

/// Generate a random password: `length` chars over four classes, guaranteeing
/// one of each, drawn from a 93-character printable set.
#[must_use]
pub fn generate(length: usize) -> String {
    const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const DIGIT: &[u8] = b"0123456789";
    const SYMBOL: &[u8] = b"!#$%&*+-?@^~";
    let mut rng = rand::rng();
    let len = length.clamp(8, 128);
    let mut chars: Vec<u8> = vec![
        LOWER[rng.random_range(0..LOWER.len())],
        UPPER[rng.random_range(0..UPPER.len())],
        DIGIT[rng.random_range(0..DIGIT.len())],
        SYMBOL[rng.random_range(0..SYMBOL.len())],
    ];
    let all: Vec<u8> = [LOWER, UPPER, DIGIT, SYMBOL].concat();
    while chars.len() < len {
        chars.push(all[rng.random_range(0..all.len())]);
    }
    // Fisher–Yates with the same RNG so the guaranteed classes land anywhere.
    for i in (1..chars.len()).rev() {
        let j = rng.random_range(0..=i);
        chars.swap(i, j);
    }
    String::from_utf8(chars).unwrap_or_default()
}

/// 0–100 heuristic: length + class variety + repetition penalty. Honest
/// about being a heuristic; a real zxcvbn-class estimator is P1.
#[must_use]
pub fn strength_score(password: &str) -> i64 {
    let len = password.chars().count() as i64;
    let classes = [
        password.chars().any(char::is_lowercase),
        password.chars().any(char::is_uppercase),
        password.chars().any(|c: char| c.is_ascii_digit()),
        password.chars().any(|c: char| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count() as i64;
    let unique = password
        .chars()
        .collect::<std::collections::HashSet<_>>()
        .len() as i64;
    let mut score = len * 4 + classes * 10 - (len - unique).max(0) * 3;
    if len < 8 {
        score -= 15;
    }
    score.clamp(0, 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    #[test]
    fn crud_with_snapshot_and_trash() {
        let s = store();
        let conn = s.conn();
        let id = save_entry(
            conn,
            None,
            "GitHub",
            Some("me"),
            Some("s3cret!"),
            Some("https://github.com"),
            None,
        )
        .unwrap();
        let list = list_entries(conn, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "GitHub");
        assert!(list[0].strength.unwrap_or(0) > 30);

        save_entry(
            conn,
            Some(id),
            "GitHub",
            Some("me"),
            Some("a much longer passphrase with words"),
            None,
            None,
        )
        .unwrap();
        let versions: i64 = conn
            .query_row(
                "SELECT count(*) FROM item_version v JOIN pw_entry p ON p.item_id = v.item_id WHERE p.id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(versions, 1);

        delete_entry(conn, id).unwrap();
        assert!(list_entries(conn, 10).unwrap().is_empty());
    }

    #[test]
    fn generator_length_and_classes() {
        for len in [8usize, 16, 32, 128] {
            let pw = generate(len);
            assert_eq!(pw.chars().count(), len);
            assert!(pw.chars().any(char::is_lowercase));
            assert!(pw.chars().any(char::is_uppercase));
            assert!(pw.chars().any(|c: char| c.is_ascii_digit()));
            assert!(pw.chars().any(|c: char| !c.is_alphanumeric()));
        }
        assert_eq!(generate(3).chars().count(), 8, "minimum length enforced");
    }

    #[test]
    fn strength_orders_reasonably() {
        assert!(strength_score("short") < strength_score("correct horse battery staple"));
        assert!(strength_score("aaaaaaaaaaaa") < strength_score("aB3$xY9#kQ2&"));
        assert_eq!(strength_score(""), 0);
        assert!(strength_score("correct horse battery staple") <= 100);
    }
}
