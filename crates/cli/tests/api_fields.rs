//! Field errors of the local JSON API. A rejected `chats.send` names the offending field and
//! leaves the attachment store, the transcript and the running jobs exactly as they were. Accepted
//! sends run on a paused account: they are stored, but start no turn and call no model.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use beans::{api::dispatch, app::App, config::Config, model::{Author, Body, Message}};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MARKER: &str = "SECRET-MARKER-9f3a";

struct Scratch {
    app: Arc<App>,
    home: std::path::PathBuf,
    chat: String,
    attachment: std::path::PathBuf,
}

/// Everything a rejected request must leave alone.
#[derive(Debug, PartialEq)]
struct Snapshot {
    files: BTreeMap<String, (u64, String)>,
    messages: Vec<String>,
    jobs: BTreeSet<String>,
}

impl Scratch {
    fn new(paused: bool) -> Self {
        let id = uuid::Uuid::new_v4();
        let home = std::env::temp_dir().join(format!("beans-api-fields-{id}"));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        beans::identity::create(&app, Some("fixture".into())).unwrap();
        app.set_paused(paused);
        let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
        let attachment = std::env::temp_dir().join(format!("beans-api-fields-{id}.txt"));
        std::fs::write(&attachment, b"fixture").unwrap();
        Self { app, home, chat, attachment }
    }

    fn snapshot(&self) -> Snapshot {
        let mut files = BTreeMap::new();
        match std::fs::read_dir(self.app.config.files_dir()) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.unwrap();
                    let bytes = std::fs::read(entry.path()).unwrap();
                    files.insert(entry.file_name().to_string_lossy().into_owned(), (bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes))));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("reading the files directory: {error}"),
        }
        let messages = self.stored().iter().map(|message| serde_json::to_string(message).unwrap()).collect();
        let jobs = self.app.running_jobs.lock().unwrap().keys().cloned().collect();
        Snapshot { files, messages, jobs }
    }

    fn stored(&self) -> Vec<Message> {
        self.app.store.page(&self.chat, None, 200).unwrap().0
    }

    fn user_ids(&self) -> BTreeSet<String> {
        self.stored().into_iter().filter(|message| message.author == Author::You).map(|message| message.id).collect()
    }

    fn file(&self) -> Value {
        json!({ "path": self.attachment.to_string_lossy() })
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        dispatch(&self.app, method, params).await
    }

    /// The request must fail naming `expect`, and change nothing.
    async fn rejected(&self, method: &str, params: Value, expect: &str) -> String {
        let before = self.snapshot();
        let error = self.call(method, params).await.expect_err("request must be rejected");
        assert!(error.contains(expect), "{method}: expected {expect:?} in {error:?}");
        assert_eq!(self.snapshot(), before, "{method} changed state before failing: {error}");
        error
    }

    async fn send_rejected(&self, fields: Value, expect: &str) -> String {
        let mut params = json!({ "chat_id": self.chat, "text": "hi", "attachments": [self.file()] });
        for (key, value) in fields.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.rejected("chats.send", params, expect).await
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
        let _ = std::fs::remove_file(&self.attachment);
    }
}

#[tokio::test]
async fn a_send_without_text_names_text() {
    let s = Scratch::new(true);
    for key in ["message", "prompt", "body"] {
        s.rejected("chats.send", json!({ "chat_id": s.chat, key: "hello" }), "text").await;
        s.rejected("chats.send", json!({ "chat_id": s.chat, key: "hello", "attachments": [] }), "text").await;
    }
    s.rejected("chats.send", json!({ "chat_id": s.chat, "text": null, "attachments": null }), "text").await;
    s.rejected("chats.send", json!({ "chat_id": s.chat, "text": "   " }), "Empty message").await;
}

#[tokio::test]
async fn a_text_of_the_wrong_type_is_rejected_not_dropped() {
    let s = Scratch::new(true);
    for text in [json!(5), json!(true), json!(["hi"]), json!({})] {
        s.send_rejected(json!({ "text": text }), "text must be a string").await;
    }
}

#[tokio::test]
async fn a_bad_chat_stores_no_attachment() {
    let s = Scratch::new(true);
    for (chat_id, expect) in [(json!(null), "missing chat_id"), (json!(""), "missing chat_id"), (json!(5), "chat_id must be a string"), (json!("nope"), "Unknown chat")] {
        s.send_rejected(json!({ "chat_id": chat_id }), expect).await;
    }
    s.rejected("chats.send", json!({ "text": "hi", "attachments": [s.file()] }), "missing chat_id").await;
}

#[tokio::test]
async fn collections_of_the_wrong_shape_are_rejected_not_ignored() {
    let s = Scratch::new(true);
    for attachments in [json!("x"), json!({ "path": "a" }), json!([5]), json!([{ "name": "no path" }]), json!([s.file(), { "name": "no path" }])] {
        s.send_rejected(json!({ "attachments": attachments }), "attachments").await;
    }
    for mentions in [json!("x"), json!([5]), json!({})] {
        s.send_rejected(json!({ "mentions": mentions }), "mentions").await;
    }
}

#[tokio::test]
async fn message_id_and_reply_to_are_checked_before_any_copy() {
    let s = Scratch::new(true);
    s.send_rejected(json!({ "message_id": 5 }), "message_id must be a string").await;
    s.send_rejected(json!({ "reply_to": 5 }), "reply_to must be a string").await;
    s.send_rejected(json!({ "reply_to": "no-such-message" }), "replied").await;
}

#[tokio::test]
async fn a_rejected_send_cannot_replace_what_is_stored() {
    let s = Scratch::new(true);
    // An attachment id and a message id that already exist: a rejected send reusing them
    // keeps the same entry count, so only the bytes and rows show a change.
    let files = s.app.config.files_dir();
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("att-existing"), b"original").unwrap();
    s.call("chats.send", json!({ "chat_id": s.chat, "text": "first", "message_id": "m-seed" })).await.unwrap();

    let overwrite = json!({ "path": s.attachment.to_string_lossy(), "id": "att-existing" });
    s.send_rejected(json!({ "attachments": [overwrite], "mentions": "x" }), "mentions").await;
    s.send_rejected(json!({ "message_id": "m-seed", "text": 5 }), "text must be a string").await;
    assert_eq!(std::fs::read(files.join("att-existing")).unwrap(), b"original");
}

#[tokio::test]
async fn an_error_never_echoes_the_rejected_value() {
    let s = Scratch::new(true);
    for fields in [json!({ "text": { "k": MARKER } }), json!({ "attachments": MARKER }), json!({ "mentions": { "k": MARKER } }), json!({ "message_id": [MARKER] }), json!({ "chat_id": [MARKER] })] {
        let error = s.send_rejected(fields, "").await;
        assert!(!error.contains(MARKER), "{error}");
    }
}

#[tokio::test]
async fn a_shared_string_field_tells_absent_from_wrong_type() {
    let s = Scratch::new(true);
    for (method, key) in [("chats.dm", "bot_id"), ("chats.stop", "chat_id"), ("routines.describe", "schedule")] {
        assert_eq!(s.call(method, json!({})).await.unwrap_err(), format!("missing {key}"));
        assert_eq!(s.call(method, json!({ key: null })).await.unwrap_err(), format!("missing {key}"));
        assert_eq!(s.call(method, json!({ key: "" })).await.unwrap_err(), format!("missing {key}"));
        for wrong in [json!(5), json!(true), json!(["x"]), json!({})] {
            assert_eq!(s.call(method, json!({ key: wrong })).await.unwrap_err(), format!("{key} must be a string"));
        }
    }
    let bots = s.app.state.lock().unwrap().bots.len();
    assert_eq!(s.call("bots.delete", json!({ "id": 5 })).await.unwrap_err(), "id must be a string");
    assert_eq!(s.app.state.lock().unwrap().bots.len(), bots, "a rejected delete must keep the bot");
}

#[tokio::test]
async fn rejections_start_no_job_on_a_running_account() {
    // Not paused: only a rejection that reached the turn would show as a job or a message.
    let s = Scratch::new(false);
    s.send_rejected(json!({ "text": 5 }), "text must be a string").await;
    s.send_rejected(json!({ "chat_id": "nope" }), "Unknown chat").await;
    s.send_rejected(json!({ "mentions": "x" }), "mentions").await;
    s.rejected("chats.send", json!({ "chat_id": s.chat, "message": "hi" }), "text").await;
    assert!(s.user_ids().is_empty());
    assert!(s.app.running_jobs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn valid_sends_are_stored_exactly() {
    let s = Scratch::new(true);
    let (users, files) = (s.user_ids(), s.snapshot().files);

    let sent = s.call("chats.send", json!({ "chat_id": s.chat, "text": " hello ", "message_id": "m-1" })).await.unwrap();
    assert_eq!(sent["message"]["id"], "m-1");
    let stored = s.app.message(&s.chat, "m-1").expect("m-1 is stored");
    assert_eq!(stored.author, Author::You);
    let Body::Text { text, attachments, .. } = stored.body else { panic!("not a text message") };
    assert_eq!((text.as_str(), attachments.len()), ("hello", 0));
    assert_eq!(s.snapshot().files, files, "a text-only send copies nothing");

    for (index, params) in [
        json!({ "chat_id": s.chat, "message_id": "m-2", "attachments": [s.file()] }),
        json!({ "chat_id": s.chat, "message_id": "m-3", "text": "", "attachments": [s.file()], "mentions": [], "reply_to": null }),
    ].into_iter().enumerate() {
        let id = format!("m-{}", index + 2);
        s.call("chats.send", params).await.unwrap();
        let stored = s.app.message(&s.chat, &id).expect("stored");
        assert_eq!(stored.author, Author::You);
        let Body::Text { attachments, .. } = stored.body else { panic!("not a text message") };
        assert_eq!(attachments.len(), 1);
        let attachment = &attachments[0];
        assert_eq!((attachment.size, attachment.name.as_str()), (7, s.attachment.file_name().unwrap().to_str().unwrap()));
        assert_eq!(std::fs::read(s.app.config.files_dir().join(&attachment.id)).unwrap(), b"fixture");
    }
    assert_eq!(std::fs::read(&s.attachment).unwrap(), b"fixture", "the source file is left alone");
    let added: BTreeSet<String> = s.user_ids().difference(&users).cloned().collect();
    assert_eq!(added, ["m-1", "m-2", "m-3"].map(String::from).into_iter().collect());
    assert!(s.app.running_jobs.lock().unwrap().is_empty(), "a paused account starts no turn");
}
