//! The local SQLite store shared by the desktop CLI and the embedded phone core: account
//! metadata, sync bookkeeping, messages, and the relay outbox.
//!
//! Every statement lives in `sql/<area>/<name>.sql` and is embedded with `sql!`, so a missing
//! file fails the build and `tests::every_sql_file_prepares` checks each one against the
//! schema. The Rust files hold only logic: `tasks` (execution authority), `messages` (the
//! transcript), `state` (the account projection), `outbox` (relay uploads), `search` (exact
//! search) and `vectors` (semantic search).

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;

use crate::app::{OutboxItem, SentJob, Slot, State};
use crate::model::{Author, Body, LiveTurn, Message};

/// The statement in `src/local_store/sql/<name>.sql`, embedded at build time.
macro_rules! sql {
    ($name:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/local_store/sql/", $name, ".sql"))
    };
}
pub(crate) use sql;

/// Same-shaped tables one `{table}` template serves (`state.rs`): ordered JSON rows and id lists.
const TEMPLATE_TABLES: [&str; 10] =
    ["devices", "device_names", "bots", "chats", "routines", "workflows", "group_deletes", "blob_deletes", "applied_blobs", "device_seen"];

/// A `{table}` template for one of `TEMPLATE_TABLES`; any other name is a programming error.
fn for_table(template: &str, table: &str) -> String {
    assert!(TEMPLATE_TABLES.contains(&table), "{table} is not a template table");
    template.replace("{table}", table)
}

/// Columns added after their table first shipped, with the statement that adds each. Derived
/// search tables need none: they are rebuilt when `SEARCH_INDEX_VERSION` changes.
const ADDED_COLUMNS: [(&str, &str, &str); 5] = [
    ("metadata", "paused", sql!("schema/add_metadata_paused")),
    ("metadata", "policy_json", sql!("schema/add_metadata_policy_json")),
    ("metadata", "roster_slot_seq", sql!("schema/add_metadata_roster_slot_seq")),
    ("pending_chat_creates", "original_json", sql!("schema/add_pending_chat_creates_original_json")),
    ("task_authority", "admission_floor", sql!("schema/add_task_authority_admission_floor")),
];

fn add_missing_columns(connection: &Connection) -> anyhow::Result<()> {
    for (table, column, add) in ADDED_COLUMNS {
        let present: bool = connection.query_row(sql!("schema/has_column"), params![table, column], |row| row.get(0))?;
        if !present {
            connection.execute(add, [])?;
        }
    }
    Ok(())
}

mod messages;
mod outbox;
mod search;
mod state;
mod tasks;
pub mod vectors;

pub use messages::*;
pub(crate) use outbox::*;
pub(crate) use search::*;
pub use tasks::*;
use state::*;

pub struct LocalStore {
    pub(crate) connection: Mutex<Connection>,
    /// The in-memory int8 index over `message_vectors` (`vectors`).
    pub(crate) vectors: Mutex<vectors::VectorCaches>,
}

impl LocalStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let config = crate::config::Config { home: parent.into(), port: 0 };
        let admission = config.ensure_home_admission()?;
        Self::open_with_admission(path, admission.as_ref(), false)
    }

    /// Deliberate staged scratch entry; ordinary App callers retain legacy admission.
    pub fn open_staged(path: &Path) -> anyhow::Result<Self> {
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let config = crate::config::Config { home: parent.into(), port: 0 };
        let admission = config.staged_storage_admission()?;
        Self::open_with_admission(path, Some(&admission), true)
    }

    pub(crate) fn open_with_admission(path: &Path, admission: Option<&crate::config::StorageAdmission>, staged: bool) -> anyhow::Result<Self> {
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let config = crate::config::Config { home: parent.into(), port: 0 };
        #[cfg(unix)]
        admission.ok_or_else(|| anyhow::anyhow!("Storage admission guard is required"))?.validate_target(parent)?;
        if staged {
            admission.ok_or_else(|| anyhow::anyhow!("Staged admission guard is required"))?.revalidate_staged()?;
        } else {
            config.validate_home()?;
            if let Some(admission) = admission { admission.revalidate_legacy()?; }
        }
        if path.exists() { crate::config::validate_database(path)?; }
        let mut connection =
            Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // Classify before CREATE IF NOT EXISTS can conceal a partial current schema.
        let version:i64=connection.pragma_query_value(None,"user_version",|r|r.get(0))?;
        let tables:std::collections::HashSet<String>=connection.prepare(sql!("schema/table_names"))?
            .query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        anyhow::ensure!((0..=4).contains(&version), "Unsupported future task schema version");
        if version==4 {
            anyhow::ensure!(tables.contains("task_rejected_envelopes"), "Task rejection safety schema is incomplete");
        }
        let current=["task_authority","task_fences","local_tasks","task_invocations","task_effect_receipts","routine_check_commits","task_safety_version"];
        let task_count=current.iter().filter(|name|tables.contains(**name)).count();
        if version>=3 {
            anyhow::ensure!(tables.contains("routine_schedule_authority") && tables.contains("routine_schedule_occurrences"),"Schedule safety schema is incomplete");
        }
        if version>=2 {
            anyhow::ensure!(task_count==current.len(),"Current task schema is incomplete; restoring missing safety tables is forbidden");
        } else if !tables.is_empty() && task_count==0 {
            anyhow::ensure!(version==1,"Unsupported predecessor schema version");
            for table in ["metadata","bots","chats","messages","outbox","sent_jobs","device_turns","codemode_store","memory_turn_admissions","memory_deliveries"] {
                anyhow::ensure!(tables.contains(table),"Unsupported partial predecessor schema");
            }
        } else if task_count>0 {
            anyhow::ensure!(task_count==current.len(),"Partial task safety schema");
        }
        connection.pragma_update(None, "application_id", crate::config::SQLITE_APPLICATION_ID)?;
        connection.pragma_update(None, "synchronous", 2)?;
        connection.execute_batch(
            sql!("schema/core"),
        )?;
        add_missing_columns(&connection)?;
        install_search_index(&mut connection)?;
        crate::config::set_private(path)?;
        Ok(Self {
            connection: Mutex::new(connection),
            vectors: Mutex::new(Default::default()),
        })
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        tx.execute_batch(sql!("schema/clear"))?;
        tx.commit()?;
        connection.execute_batch(sql!("schema/checkpoint"))?;
        drop(connection);
        *self.vectors.lock().unwrap() = Default::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
