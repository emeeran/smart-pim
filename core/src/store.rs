use std::path::Path;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::schema;

/// Open handle to the encrypted database. Not `Clone`: ownership is deliberate,
/// the UI layer will hold it in a Tauri managed state.
pub struct Store {
    conn: Connection,
}

/// Derive a `SQLCipher` key from a passphrase (argon2id, RFC 9106-recommended
/// parameters) and zeroize it after use.
#[must_use]
pub fn derive_key(passphrase: &str) -> Zeroizing<String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    // Fixed: the DB file itself is the secret carrier. Renamed at the Smart PIM
    // rebrand — databases created before the rebrand used a different salt and
    // are NOT openable with this key derivation (accepted breaking change).
    let salt = b"smartpim-db-key-v1";
    let params = Params::new(64 * 1024, 3, 4, Some(32)).expect("static params are valid");
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    a2.hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .expect("hash_password_into cannot fail with valid params");
    let mut hex = String::with_capacity(64);
    for b in out {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    Zeroizing::new(hex)
}

impl Store {
    /// Open (or create) the database at `path`, unlocking with `passphrase`.
    pub fn open(path: &Path, passphrase: &str) -> Result<Self> {
        let key = derive_key(passphrase);
        let conn = Connection::open(path)?;
        Self::init(conn, &key)
    }

    /// In-memory store (tests, throwaway previews).
    pub fn open_memory(passphrase: &str) -> Result<Self> {
        let key = derive_key(passphrase);
        let conn = Connection::open_in_memory()?;
        Self::init(conn, &key)
    }

    fn init(conn: Connection, key: &str) -> Result<Self> {
        conn.pragma_update(None, "key", key)?;
        // Sanity probe: SQLCipher fails this with wrong key; plain files would
        // pass pragmas but fail on first read. Force a read now.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let ver: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        let _ = ver;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        schema::apply_all(&mut self.conn)
    }

    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn transaction(&mut self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self.conn.transaction()?)
    }

    /// Verify the file at `path` opens with `passphrase` without migrating.
    pub fn probe(path: &Path, passphrase: &str) -> Result<bool> {
        let key = derive_key(passphrase);
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", key.as_str())?;
        match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::NotADatabase =>
            {
                Ok(false)
            }
            Err(e) => Err(Error::from(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_schema_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlcipher");
        {
            let store = Store::open(&path, "correct horse").unwrap();
            let n: i64 = store
                .conn()
                .query_row("SELECT count(*) FROM item", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0);
        }
        assert!(Store::probe(&path, "correct horse").unwrap());
        assert!(!Store::probe(&path, "wrong").unwrap());
        let _ = Store::open(&path, "correct horse").unwrap();
    }

    #[test]
    fn migration_runs_once() {
        let mut store = Store::open_memory("x").unwrap();
        store.migrate().unwrap();
        let v: i64 = store
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 7); // latest schema (V7: Google Calendar/Tasks sync)
    }
}
