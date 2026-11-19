//! The store's tests by area, sharing scratch stores and fixtures.

mod messages;
mod outbox;
mod search;
mod sql;
mod tasks;
mod vectors;

use super::*;
use crate::model::{MessageState, SNAPSHOT_MESSAGES};

pub(super) struct Scratch(pub(super) LocalStore, pub(super) std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

pub(super) fn scratch() -> Scratch {
    let home = std::env::temp_dir().join(format!("beans-transcript-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    Scratch(LocalStore::open(&home.join("beans.sqlite3")).unwrap(), home)
}

pub(super) fn task_fixture(store: &LocalStore, id: &str) -> (TaskLease,crate::model::Job) {
    let owner = "owner-test";
    let epoch = store.current_task_account_epoch().unwrap().unwrap_or_else(||store.recover_task_owner(owner).unwrap());
    let lease = TaskLease { account_epoch:epoch,owner_epoch:owner.into(),task_id:id.into(),execution_id:uuid::Uuid::new_v4().to_string(),incarnation:0 };
    let job = crate::model::Job { id:id.into(),chat_id:"chat-test".into(),bot_id:"bot-test".into(),kind:"turn".into(),trigger_message_id:String::new(),routine_id:None,check:None,requested_by:"device-test".into(),from_bot_id:None,hops:0,round:0,is_winding_down:false,setup:None,created_at:0.0 };
    (lease,job)
}

pub(super) fn invocation() -> InvocationBinding {
    InvocationBinding { invocation_id:"invocation-test".into(),parent_invocation_id:None,ordinal:0,attempt_id:"attempt-test".into(),receipt_id:"receipt-test".into(),revision:1,digest:[42;32] }
}

pub(super) fn message(id: &str, at: f64) -> Message {
    let mut message = Message::new("chat", Author::You, Body::text(id));
    message.id = id.into();
    message.created_at = at;
    message
}
