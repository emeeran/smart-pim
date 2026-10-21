//! Contacts repository: vCard/CSV records ↔ `contact` + `contact_field`.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::model::Kind;
use crate::repo;
use crate::vcard::ContactRecord;

/// Insert or update by UID, mirroring `upsert_event` semantics. Returns the
/// contact row id.
pub fn upsert_contact(conn: &Connection, c: &ContactRecord) -> Result<i64> {
    let display = if c.display_name.is_empty() {
        format!("{} {}", c.first_name, c.last_name)
            .trim()
            .to_string()
    } else {
        c.display_name.clone()
    };
    let existing: Option<i64> = match &c.uid {
        Some(uid) => conn
            .query_row("SELECT id FROM contact WHERE uid = ?1", [uid], |r| r.get(0))
            .optional()?,
        None => None,
    };

    if let Some(contact_id) = existing {
        let item_id: i64 = conn.query_row(
            "SELECT item_id FROM contact WHERE id = ?1",
            [contact_id],
            |r| r.get(0),
        )?;
        let item = repo::get_item(conn, item_id)?;
        repo::snapshot_item(conn, &item)?;
        conn.execute(
            "UPDATE contact SET first_name=?1, last_name=?2, display_name=?3, organization=?4, \
             birthday=?5, notes=?6 WHERE id=?7",
            params![
                c.first_name,
                c.last_name,
                display,
                c.organization,
                c.birthday,
                c.notes,
                contact_id
            ],
        )?;
        conn.execute(
            "DELETE FROM contact_field WHERE contact_id = ?1",
            [contact_id],
        )?;
        insert_fields(conn, contact_id, c)?;
        let _ = repo::touch_item(conn, item_id);
        return Ok(contact_id);
    }

    let item_id = repo::create_item(conn, Kind::Contact, None, false)?;
    conn.execute(
        "INSERT INTO contact (item_id, uid, first_name, last_name, display_name, organization, \
         birthday, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            item_id,
            c.uid,
            c.first_name,
            c.last_name,
            display,
            c.organization,
            c.birthday,
            c.notes
        ],
    )?;
    let contact_id = conn.last_insert_rowid();
    insert_fields(conn, contact_id, c)?;
    Ok(contact_id)
}

fn insert_fields(conn: &Connection, contact_id: i64, c: &ContactRecord) -> Result<()> {
    for e in &c.emails {
        conn.execute(
            "INSERT INTO contact_field (contact_id, kind, value, is_primary) \
             VALUES (?1, 'email', ?2, 0)",
            params![contact_id, e],
        )?;
    }
    for p in &c.phones {
        conn.execute(
            "INSERT INTO contact_field (contact_id, kind, value, is_primary) \
             VALUES (?1, 'phone', ?2, 0)",
            params![contact_id, p],
        )?;
    }
    for a in &c.addresses {
        conn.execute(
            "INSERT INTO contact_field (contact_id, kind, value, is_primary) \
             VALUES (?1, 'address', ?2, 0)",
            params![contact_id, a],
        )?;
    }
    for u in &c.urls {
        conn.execute(
            "INSERT INTO contact_field (contact_id, kind, value, is_primary) \
             VALUES (?1, 'url', ?2, 0)",
            params![contact_id, u],
        )?;
    }
    Ok(())
}

/// Records for export (round-trip), with phone/email/address/url fields.
#[must_use]
pub fn all_contacts(conn: &Connection) -> Vec<ContactRecord> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT c.uid, c.first_name, c.last_name, c.display_name, c.organization, c.birthday, \
         c.notes, c.id, c.item_id FROM contact c JOIN item i ON i.id = c.item_id \
         WHERE i.deleted_at IS NULL ORDER BY c.display_name",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(ContactRecord {
            uid: r.get(0)?,
            first_name: r.get(1)?,
            last_name: r.get(2)?,
            display_name: r.get(3)?,
            organization: r.get(4)?,
            birthday: r.get(5)?,
            notes: r.get(6)?,
            id: r.get(7)?,
            item_id: r.get(8)?,
            ..Default::default()
        })
    }) else {
        return Vec::new();
    };
    let mut out = rows.filter_map(std::result::Result::ok).collect::<Vec<_>>();
    for c in &mut out {
        if let Ok(mut f) =
            conn.prepare("SELECT kind, value FROM contact_field WHERE contact_id = ?1 ORDER BY id")
        {
            if let Ok(fr) = f.query_map([c.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }) {
                for row in fr.flatten() {
                    match row.0.as_str() {
                        "email" => c.emails.push(row.1),
                        "phone" => c.phones.push(row.1),
                        "address" => c.addresses.push(row.1),
                        "url" => c.urls.push(row.1),
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// Email → contact id resolution for per-contact mail history (S48).
pub fn contact_id_by_email(conn: &Connection, email: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT contact_id FROM contact_field WHERE kind = 'email' AND value = ?1 LIMIT 1",
            [email],
            |r| r.get(0),
        )
        .optional()?)
}

/// Move a contact to trash (S8).
pub fn delete_contact(conn: &Connection, contact_id: i64) -> Result<()> {
    let item_id: i64 = conn
        .query_row(
            "SELECT item_id FROM contact WHERE id = ?1",
            [contact_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(crate::error::Error::NotFound(contact_id))?;
    repo::trash_item(conn, item_id)
}

/// One contact by store id (editor detail view).
pub fn contact_get(conn: &Connection, contact_id: i64) -> Result<ContactRecord> {
    let all = all_contacts(conn);
    all.into_iter()
        .find(|c| c.id == contact_id)
        .ok_or(crate::error::Error::NotFound(contact_id))
}

// ---------------------------------------------------------------- dedupe

/// One merge candidate group (plan §3 Contacts: deterministic matching does
/// the work; the merge itself always previews a diff in the UI).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DupeGroup {
    pub contact_ids: Vec<i64>,
    pub reason: String,
}

/// Duplicate candidates: same normalized email, or same normalized name with
/// the same email domain / organization. Pure SQL + Rust; no model involved.
///
/// # Errors
/// Propagates SQL errors.
pub fn dupe_candidates(conn: &Connection) -> Result<Vec<DupeGroup>> {
    let contacts = all_contacts(conn);
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let domain = |email: &str| email.rsplit('@').next().unwrap_or("").to_lowercase();

    let mut out: Vec<DupeGroup> = Vec::new();
    for i in 0..contacts.len() {
        for j in (i + 1)..contacts.len() {
            let (a, b) = (&contacts[i], &contacts[j]);
            let email_hit = a
                .emails
                .iter()
                .any(|e| b.emails.iter().any(|f| norm(e) == norm(f) && !e.is_empty()));
            let same_name =
                !a.display_name.is_empty() && norm(&a.display_name) == norm(&b.display_name);
            let same_domain = a
                .emails
                .first()
                .zip(b.emails.first())
                .is_some_and(|(e, f)| !domain(e).is_empty() && domain(e) == domain(f));
            let same_org =
                a.organization.is_some() && a.organization == b.organization && same_name;
            let reason = if email_hit {
                "same email address"
            } else if same_name && same_domain {
                "same name and email domain"
            } else if same_org {
                "same name and organization"
            } else {
                continue;
            };
            match out.iter_mut().find(|g| g.contact_ids.contains(&a.id)) {
                Some(g) => {
                    if !g.contact_ids.contains(&b.id) {
                        g.contact_ids.push(b.id);
                    }
                }
                None => out.push(DupeGroup {
                    contact_ids: vec![a.id, b.id],
                    reason: reason.to_string(),
                }),
            }
        }
    }
    Ok(out)
}

/// Merge `secondary` into `primary`: fill missing scalars, union fields
/// (exact kind+value duplicates skipped), copy related-item links, preserve
/// the losing record as a version snapshot on the survivor, then trash the
/// secondary. Returns the primary id.
///
/// # Errors
/// Propagates SQL/repo errors; both ids must exist.
pub fn merge_contact(conn: &Connection, primary_id: i64, secondary_id: i64) -> Result<i64> {
    if primary_id == secondary_id {
        return Err(crate::error::Error::Row {
            row: 0,
            message: "cannot merge a contact into itself".into(),
        });
    }
    let mut keep = contact_get(conn, primary_id)?;
    let drop = contact_get(conn, secondary_id)?;

    if keep.display_name.is_empty() {
        keep.display_name.clone_from(&drop.display_name);
    }
    if keep.first_name.is_empty() {
        keep.first_name.clone_from(&drop.first_name);
    }
    if keep.last_name.is_empty() {
        keep.last_name.clone_from(&drop.last_name);
    }
    if keep.organization.is_none() {
        keep.organization.clone_from(&drop.organization);
    }
    if keep.birthday.is_none() {
        keep.birthday.clone_from(&drop.birthday);
    }
    if keep.notes.is_none() {
        keep.notes.clone_from(&drop.notes);
    }
    if keep.uid.is_none() {
        keep.uid.clone_from(&drop.uid);
    }
    for (keep_list, drop_list) in [
        (&mut keep.emails, &drop.emails),
        (&mut keep.phones, &drop.phones),
        (&mut keep.addresses, &drop.addresses),
        (&mut keep.urls, &drop.urls),
        (&mut keep.categories, &drop.categories),
    ] {
        for v in drop_list {
            if !keep_list.iter().any(|x| x.eq_ignore_ascii_case(v)) {
                keep_list.push(v.clone());
            }
        }
    }

    // Preserve the losing record, apply the union, copy links, trash.
    let keep_item: i64 = conn.query_row(
        "SELECT item_id FROM contact WHERE id = ?1",
        [primary_id],
        |r| r.get(0),
    )?;
    let drop_item: i64 = conn.query_row(
        "SELECT item_id FROM contact WHERE id = ?1",
        [secondary_id],
        |r| r.get(0),
    )?;
    repo::snapshot_item(conn, &repo::get_item(conn, keep_item)?)?;
    conn.execute(
        "INSERT INTO item_version (item_id, saved_at, snapshot) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            keep_item,
            crate::model::now_iso(),
            serde_json::to_string(&drop)?
        ],
    )?;
    upsert_contact(conn, &keep)?;

    for other in repo::related_items(conn, drop_item) {
        if other != keep_item {
            repo::link_items(conn, keep_item, other)?;
        }
    }
    repo::unlink_items(conn, keep_item, drop_item)?;
    repo::trash_item(conn, drop_item)?;
    Ok(primary_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn dedupe_finds_and_merges() {
        let mut store = Store::open_memory("x").unwrap();
        let conn = store.conn_mut();
        let a = upsert_contact(
            conn,
            &crate::vcard::parse_vcf(
                "BEGIN:VCARD\r\nUID:a1\r\nFN:Marie Powell\r\nEMAIL:marie@x.io\r\nTEL:+1 555\r\nEND:VCARD\r\n",
            )
            .contacts
            .remove(0),
        )
        .unwrap();
        let b = upsert_contact(
            conn,
            &crate::vcard::parse_vcf(
                "BEGIN:VCARD\r\nUID:b1\r\nFN:M. Powell\r\nEMAIL:MARIE@x.io\r\nORG:Example\r\nEND:VCARD\r\n",
            )
            .contacts
            .remove(0),
        )
        .unwrap();
        let groups = dupe_candidates(conn).unwrap();
        assert!(groups
            .iter()
            .any(|g| g.contact_ids.contains(&a) && g.contact_ids.contains(&b)));

        // A note linked to the secondary survives via the primary.
        let note_item =
            crate::repo::create_item(conn, crate::model::Kind::Note, None, false).unwrap();
        crate::repo::link_items(conn, note_item, {
            conn.query_row("SELECT item_id FROM contact WHERE id = ?1", [b], |r| {
                r.get(0)
            })
            .unwrap()
        })
        .unwrap();

        merge_contact(conn, a, b).unwrap();
        let kept = contact_get(conn, a).unwrap();
        assert_eq!(kept.emails.len(), 1, "exact duplicate field not duplicated");
        assert_eq!(kept.phones, vec!["+1 555"]);
        assert_eq!(kept.organization.as_deref(), Some("Example"));
        assert!(crate::repo::related_items(
            conn,
            conn.query_row("SELECT item_id FROM contact WHERE id = ?1", [a], |r| r
                .get(0))
                .unwrap()
        )
        .contains(&note_item));
        let alive: i64 = conn
            .query_row(
                "SELECT count(*) FROM contact c JOIN item i ON i.id = c.item_id \
                 WHERE i.deleted_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(alive, 1, "secondary trashed");
        assert_eq!(dupe_candidates(conn).unwrap(), [] as [DupeGroup; 0]);
    }

    #[test]
    fn merge_into_self_rejected() {
        let mut store = Store::open_memory("x").unwrap();
        let conn = store.conn_mut();
        let a = upsert_contact(
            conn,
            &crate::vcard::parse_vcf(
                "BEGIN:VCARD\r\nUID:s1\r\nFN: Solo\r\nEMAIL:s@x.io\r\nEND:VCARD\r\n",
            )
            .contacts
            .remove(0),
        )
        .unwrap();
        assert!(merge_contact(conn, a, a).is_err());
    }

    #[test]
    fn vcf_roundtrip_through_store() {
        let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:mp-1\r\nN:Powell;Marie;;;\r\nFN:Marie Powell\r\nORG:Example\r\nEMAIL:mariep@example.com\r\nTEL:+1 555 901 2345\r\nBDAY:19740301\r\nEND:VCARD\r\n";
        let parsed = crate::vcard::parse_vcf(vcf);
        let mut store = Store::open_memory("x").unwrap();
        let conn = store.conn_mut();
        let id = upsert_contact(conn, &parsed.contacts[0]).unwrap();
        // re-upsert same UID updates, not duplicates
        let id2 = upsert_contact(conn, &parsed.contacts[0]).unwrap();
        assert_eq!(id, id2);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM contact", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(all_contacts(conn)[0].emails, vec!["mariep@example.com"]);
        assert_eq!(
            contact_id_by_email(conn, "mariep@example.com").unwrap(),
            Some(id)
        );
        let (first, bday): (String, Option<String>) = conn
            .query_row(
                "SELECT first_name, birthday FROM contact WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(first, "Marie");
        assert_eq!(bday.as_deref(), Some("1974-03-01"));
    }
}
