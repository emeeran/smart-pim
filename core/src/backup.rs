//! Automatic backup: `VACUUM INTO` snapshots with retention pruning.
//! Non-destructive: only ever creates files, never touches the live DB or
//! existing snapshots beyond pruning the oldest beyond `keep`.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::Result;
use crate::model::now_iso;

/// Create `dir/<timestamp>.db` as a consistent snapshot of `conn`.
pub fn create_backup(conn: &Connection, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = now_iso().replace([':', '.'], "-");
    let target = dir.join(format!("{stamp}.db"));
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])?;
    Ok(target)
}

/// Delete the oldest `*.db` snapshots beyond `keep`. Returns removed count.
pub fn prune_backups(dir: &Path, keep: usize) -> Result<usize> {
    let mut snaps: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "db"))
        .collect();
    snaps.sort();
    let excess = snaps.len().saturating_sub(keep);
    for path in &snaps[..excess] {
        std::fs::remove_file(path)?;
    }
    Ok(excess)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn backup_is_valid_independent_db_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let bdir = dir.path().join("backups");
        let store = Store::open_memory("pw").unwrap();
        store
            .conn()
            .execute("INSERT INTO tag (name) VALUES ('x')", [])
            .unwrap();
        for _i in 0..4 {
            let p = create_backup(store.conn(), &bdir).unwrap();
            assert!(p.exists());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let removed = prune_backups(&bdir, 2).unwrap();
        assert_eq!(removed, 2);
        let remaining: Vec<_> = std::fs::read_dir(&bdir).unwrap().collect();
        assert_eq!(remaining.len(), 2);
        // Snapshot opens standalone (no key: VACUUM INTO output is plaintext
        // only when source used plaintext; SQLCipher keeps the key).
        let snap = std::fs::read_dir(&bdir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let opened = Store::open(&snap, "pw");
        assert!(opened.is_ok());
        let store = opened.unwrap();
        let n: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
