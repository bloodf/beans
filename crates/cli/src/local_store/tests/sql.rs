//! The statements in `sql/`: each one against a fresh store's schema, each one in use, and the
//! column migrations on an older store. A typo or a dropped column fails here, not in the one
//! code path that happens to run the statement.

use super::*;
use std::path::{Path, PathBuf};

fn files_under(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == extension) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Every statement file, by the name `sql!` takes (`area/name`), with its text.
fn statements() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/local_store/sql");
    files_under(&root, "sql")
        .into_iter()
        .map(|path| {
            let name = path.strip_prefix(&root).unwrap().with_extension("").to_string_lossy().replace('\\', "/");
            (name, std::fs::read_to_string(&path).unwrap())
        })
        .collect()
}

#[test]
fn every_statement_prepares_against_the_schema() {
    let scratch = scratch();
    let connection = scratch.0.connection.lock().unwrap();
    let statements = statements();
    assert!(statements.len() > 150, "the statements were found");
    for (name, text) in statements {
        // Schema batches run at every open; column migrations are checked below.
        if name.starts_with("schema/") && name != "schema/has_column" {
            continue;
        }
        // A `{table}` template serves same-shaped tables; `devices` is one of them.
        let text = text.replace("{table}", "devices");
        if let Err(error) = connection.prepare(&text) {
            panic!("{name}.sql: {error}");
        }
    }
}

#[test]
fn every_statement_is_used() {
    let sources: String = files_under(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), "rs").iter().map(|path| std::fs::read_to_string(path).unwrap()).collect();
    let unused: Vec<String> = statements().into_iter().map(|(name, _)| name).filter(|name| !sources.contains(&format!("sql!(\"{name}\")"))).collect();
    assert!(unused.is_empty(), "statements no code runs: {unused:?}");
}

#[test]
fn added_columns_upgrade_an_older_store() {
    let connection = Connection::open_in_memory().unwrap();
    // These tables as they first shipped, before the columns `ADDED_COLUMNS` adds.
    connection
        .execute_batch(
            "CREATE TABLE metadata (id INTEGER PRIMARY KEY, auto_review_json TEXT NOT NULL, last_seq INTEGER NOT NULL,
                 machine_blob_hash TEXT, credentials_uploaded INTEGER NOT NULL);
             CREATE TABLE pending_chat_creates (id TEXT PRIMARY KEY NOT NULL);
             CREATE TABLE task_authority (id INTEGER PRIMARY KEY, account_epoch TEXT NOT NULL, owner_epoch TEXT NOT NULL, closed INTEGER NOT NULL);
             INSERT INTO metadata VALUES (1, '{}', 7, NULL, 0);",
        )
        .unwrap();
    add_missing_columns(&connection).unwrap();
    add_missing_columns(&connection).unwrap();
    for (table, column, _) in ADDED_COLUMNS {
        let present: bool = connection.query_row(sql!("schema/has_column"), params![table, column], |r| r.get(0)).unwrap();
        assert!(present, "{table}.{column}");
    }
    let kept: (i64, bool, String) = connection.query_row("SELECT last_seq, paused, policy_json FROM metadata", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(kept, (7, false, "{}".to_string()), "existing rows keep their values and take the defaults");
}
