//! Semantic search plumbing (`docs/ai_integ_plan.md`, "Search" and "Risks"):
//! embedding vectors stored as little-endian f32 BLOBs inside the `SQLCipher`
//! database
//! (`embedding` table, V5), so the encrypted-at-rest guarantee holds. Vectors
//! are keyed by `item_id` + `updated_at`; stale entries are re-embedded
//! lazily. Cosine similarity and hybrid (FTS + cosine) fusion run in Rust
//! over an FTS5 prefilter — the sqlite-vec extension is deferred (see
//! `docs/data-model.md` "AI model").

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::Result;

/// Text per item for embedding. Diary included — embedding it is gated
/// separately upstream (`ai.feature.diary`), never on by default.
const EMBED_SQL: &str = "SELECT i.id, i.kind, i.updated_at,
        CASE i.kind
            WHEN 'note' THEN (SELECT note.title || ' ' || note.content FROM note WHERE note.item_id = i.id)
            WHEN 'task' THEN (SELECT task.title || ' ' || COALESCE(task.notes, '') FROM task WHERE task.item_id = i.id)
            WHEN 'diary' THEN (SELECT diary_entry.content FROM diary_entry WHERE diary_entry.item_id = i.id)
            ELSE NULL
        END
    FROM item i WHERE i.deleted_at IS NULL AND i.kind IN ('note','task','diary')";

/// One item awaiting (re-)embedding with its extracted text.
#[derive(Debug, Clone, Serialize)]
pub struct EmbedItem {
    pub item_id: i64,
    pub kind: String,
    pub updated_at: String,
    pub text: String,
}

/// Items whose stored embedding is missing, stale (item changed after the
/// embedding was written) or written by a different model. Oldest first,
/// bounded by `limit`; text is truncated to 4000 chars.
///
/// # Errors
/// Propagates SQL errors.
pub fn stale_items(conn: &Connection, model: &str, limit: i64) -> Result<Vec<EmbedItem>> {
    let mut stmt = conn.prepare(EMBED_SQL)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (item_id, kind, updated_at, text) = row?;
        let Some(text) = text else { continue };
        let fresh: Option<String> = conn
            .query_row(
                "SELECT updated_at FROM embedding WHERE item_id = ?1 AND model = ?2",
                params![item_id, model],
                |r| r.get(0),
            )
            .optional()?;
        let stale = fresh.as_deref().is_none_or(|at| at < updated_at.as_str());
        if stale {
            out.push(EmbedItem {
                item_id,
                kind,
                updated_at,
                text: truncate(&text, 4000),
            });
            if out.len() as i64 >= limit {
                break;
            }
        }
    }
    Ok(out)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

/// Store (or refresh) the embedding for one item. Uses the item's current
/// `updated_at` so staleness detection works.
///
/// # Errors
/// Propagates SQL errors.
pub fn store_embedding(conn: &Connection, item_id: i64, model: &str, vec: &[f32]) -> Result<()> {
    let updated_at: String = conn.query_row(
        "SELECT updated_at FROM item WHERE id = ?1",
        [item_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO embedding (item_id, model, updated_at, vec) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(item_id) DO UPDATE SET model = excluded.model, \
         updated_at = excluded.updated_at, vec = excluded.vec",
        params![item_id, model, updated_at, encode_vec(vec)],
    )?;
    Ok(())
}

/// Cosine similarity over all stored embeddings of `model` (full scan; with
/// an FTS prefilter upstream this stays sub-second into the tens of
/// thousands). Returns `(item_id, score)` sorted by score descending, scores
/// clamped to ≥ 0.
///
/// # Errors
/// Propagates SQL errors.
pub fn semantic_candidates(
    conn: &Connection,
    model: &str,
    query: &[f32],
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    let mut stmt = conn.prepare("SELECT item_id, vec FROM embedding WHERE model = ?1")?;
    let rows = stmt.query_map([model], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut scored: Vec<(i64, f32)> = rows
        .flatten()
        .filter_map(|(id, blob)| {
            let v = decode_vec(&blob);
            cosine(query, &v).map(|c| (id, c.max(0.0)))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored)
}

/// Encode f32s as little-endian bytes.
#[must_use]
pub fn encode_vec(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode little-endian bytes as f32s (misaligned trailing bytes dropped).
#[must_use]
pub fn decode_vec(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

/// Cosine similarity; `None` on length mismatch or a zero vector.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(dot / (na * nb).sqrt())
}

/// Hybrid rank fusion: normalized cosine (0..1) plus a small boost for FTS
/// agreement (`fts_rank` negative BM25, `best_rank` the best in the result
/// set). Weights favour semantic, matching "FTS stays the default".
#[must_use]
pub fn fuse(cos: f32, fts_rank: f32, best_rank: f32) -> f32 {
    let semantic = cos.clamp(0.0, 1.0);
    // BM25 ranks are negative; 0-adjacent (best_rank) is most relevant, so
    // the lexical score is best/current — 1.0 when this hit IS the best.
    let lexical = if fts_rank < 0.0 && best_rank < 0.0 {
        (best_rank / fts_rank).clamp(0.0, 1.0)
    } else {
        0.0
    };
    0.7 * semantic + 0.3 * lexical
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    fn note(s: &Store, title: &str) -> i64 {
        let item =
            crate::repo::create_item(s.conn(), crate::model::Kind::Note, None, false).unwrap();
        s.conn()
            .execute(
                "INSERT INTO note (item_id, title, content) VALUES (?1, ?2, 'body text')",
                rusqlite::params![item, title],
            )
            .unwrap();
        item
    }

    #[test]
    fn vec_roundtrip_and_cosine() {
        let v = vec![0.25f32, -1.5, 3.75];
        let b = encode_vec(&v);
        assert_eq!(b.len(), 12);
        assert_eq!(decode_vec(&b), v);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]).unwrap() - 1.0).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0]).unwrap()).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), None);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), None);
    }

    #[test]
    fn staleness_detection_and_reindex_cycle() {
        let s = store();
        let item = note(&s, "rust notes");
        assert_eq!(
            stale_items(s.conn(), "m1", 10).unwrap().len(),
            1,
            "missing = stale"
        );
        store_embedding(s.conn(), item, "m1", &[1.0, 0.0]).unwrap();
        assert!(stale_items(s.conn(), "m1", 10).unwrap().is_empty());
        assert_eq!(
            stale_items(s.conn(), "m2", 10).unwrap().len(),
            1,
            "other model = stale"
        );
        // Item edit bumps updated_at → embedding goes stale again (explicit
        // later timestamp: now_iso is second-resolution and the test runs
        // within the same second).
        s.conn()
            .execute(
                "UPDATE item SET updated_at = '2027-01-01T00:00:00+00:00' WHERE id = ?1",
                [item],
            )
            .unwrap();
        assert_eq!(stale_items(s.conn(), "m1", 10).unwrap().len(), 1);
    }

    #[test]
    fn candidates_rank_by_cosine() {
        let s = store();
        let a = note(&s, "a");
        let b = note(&s, "b");
        let c_no_match = note(&s, "c");
        store_embedding(s.conn(), a, "m", &[1.0, 0.0]).unwrap();
        store_embedding(s.conn(), b, "m", &[0.9, 0.1]).unwrap();
        store_embedding(s.conn(), c_no_match, "m", &[0.0, 1.0]).unwrap();
        let hits = semantic_candidates(s.conn(), "m", &[1.0, 0.0], 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, a);
        assert_eq!(hits[1].0, b);
    }

    #[test]
    fn fuse_prefers_semantic_with_lexical_boost() {
        assert!(fuse(1.0, -2.0, -2.0) > fuse(1.0, -100.0, -2.0));
        assert!((fuse(1.0, -2.0, -2.0) - 1.0).abs() < 1e-6);
        assert!(fuse(0.0, -2.0, -2.0) > fuse(0.0, 0.0, -2.0));
    }
}
