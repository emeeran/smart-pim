//! Smart PIM core engine.
//!
//! Owns persistence (SQLCipher/SQLite), migrations, the universal item model,
//! full-text search, backup, and the import/export pipelines. No UI, no network
//! clients (those live above this crate and drive it through its API).

pub mod aisearch;
pub mod backup;
pub mod calrepo;
pub mod contacts;
pub mod csvio;
pub mod diary;
pub mod error;
pub mod events;
pub mod export;
pub mod gsync;
pub mod htmlsan;
pub mod ical;
pub mod import;
pub mod mailstore;
pub mod model;
pub mod notes;
pub mod ops;
pub mod pagination;
pub mod pwrepo;
pub mod related;
pub mod repo;
pub mod rules;
pub mod schedule;
pub mod schema;
pub mod search;
pub mod store;
pub mod sync;
pub mod tasks;
pub mod vcard;

pub use error::{Error, Result};
pub use store::Store;
