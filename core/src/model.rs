//! Shared value types for the universal item model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Event,
    Task,
    Note,
    Diary,
    Contact,
    Password,
    Message,
}

impl Kind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Event => "event",
            Kind::Task => "task",
            Kind::Note => "note",
            Kind::Diary => "diary",
            Kind::Contact => "contact",
            Kind::Password => "password",
            Kind::Message => "message",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "event" => Kind::Event,
            "task" => Kind::Task,
            "note" => Kind::Note,
            "diary" => Kind::Diary,
            "contact" => Kind::Contact,
            "password" => Kind::Password,
            "message" => Kind::Message,
            _ => return None,
        })
    }
}

/// One row of the universal `item` table.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: i64,
    pub kind: Kind,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub private: bool,
    pub list_id: Option<i64>,
    pub sort_key: Option<String>,
}

/// A list / calendar / group from `item_list` (the trees in the left panel).
#[derive(Debug, Clone, Serialize)]
pub struct List {
    pub id: i64,
    pub kind: String,
    pub parent_id: Option<i64>,
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i64,
    pub settings: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

/// Snapshot of an item at a point in time (version history).
#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub id: i64,
    pub item_id: i64,
    pub saved_at: String,
    pub snapshot: String,
}

/// Entry of the queryable history (audit log).
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub item_id: Option<i64>,
    pub action: String,
    pub detail: Option<String>,
}

/// A cross-link ("Related Items") between two items; stored one way, symmetric
/// via trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ItemLink {
    pub from_id: i64,
    pub to_id: i64,
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
