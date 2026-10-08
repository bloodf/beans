//! Field errors of the local JSON API: a rejected request names the offending field, copies no
//! file into the store and writes no message. The app is paused, so an accepted send is stored
//! but starts no turn and calls no model.
use std::sync::Arc;

use beans::{api::dispatch, app::App, config::Config};
use serde_json::{json, Value};

struct Scratch {
    app: Arc<App>,
    home: std::path::PathBuf,
    chat: String,
    attachment: std::path::PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let id = uuid::Uuid::new_v4();
        let home = std::env::temp_dir().join(format!("beans-api-fields-{id}"));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        beans::identity::create(&app, Some("fixture".into())).unwrap();
        app.set_paused(true);
        let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
        let attachment = std::env::temp_dir().join(format!("beans-api-fields-{id}.txt"));
        std::fs::write(&attachment, b"fixture").unwrap();
        Self { app, home, chat, attachment }
    }

    fn files(&self) -> usize {
        std::fs::read_dir(self.app.config.files_dir()).map(|entries| entries.count()).unwrap_or(0)
    }

    fn messages(&self) -> usize {
        self.app.message_page(&self.chat, None, 200).0.len()
    }

    fn file(&self) -> Value {
        json!({ "path": self.attachment.to_string_lossy() })
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        dispatch(&self.app, method, params).await
    }

    /// The error of a request that must change nothing.
    async fn rejected(&self, method: &str, params: Value) -> String {
        let before = (self.files(), self.messages());
        let error = self.call(method, params).await.expect_err("request must be rejected");
        assert_eq!((self.files(), self.messages()), before, "{method} stored work before failing: {error}");
        error
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
    let s = Scratch::new();
    for key in ["message", "prompt", "body"] {
        let error = s.rejected("chats.send", json!({ "chat_id": s.chat, key: "hello" })).await;
        assert!(error.contains("text"), "{key}: {error}");
    }
}

#[tokio::test]
async fn a_text_of_the_wrong_type_is_rejected_not_dropped() {
    let s = Scratch::new();
    for text in [json!(5), json!(true), json!(["hi"]), json!({})] {
        let error = s.rejected("chats.send", json!({ "chat_id": s.chat, "text": text, "attachments": [s.file()] })).await;
        assert_eq!(error, "text must be a string");
    }
}

#[tokio::test]
async fn a_bad_chat_stores_no_attachment() {
    let s = Scratch::new();
    let error = s.rejected("chats.send", json!({ "text": "hi", "attachments": [s.file()] })).await;
    assert_eq!(error, "missing chat_id");
    let error = s.rejected("chats.send", json!({ "chat_id": 5, "text": "hi", "attachments": [s.file()] })).await;
    assert_eq!(error, "chat_id must be a string");
    let error = s.rejected("chats.send", json!({ "chat_id": "nope", "text": "hi", "attachments": [s.file()] })).await;
    assert_eq!(error, "Unknown chat");
}

#[tokio::test]
async fn attachments_and_mentions_of_the_wrong_shape_are_rejected_not_ignored() {
    let s = Scratch::new();
    for attachments in [json!("x"), json!({ "path": "a" }), json!([5]), json!([{ "name": "no path" }])] {
        let error = s.rejected("chats.send", json!({ "chat_id": s.chat, "text": "hi", "attachments": attachments })).await;
        assert!(error.contains("attachments"), "{attachments}: {error}");
    }
    for mentions in [json!("x"), json!([5]), json!({})] {
        let error = s.rejected("chats.send", json!({ "chat_id": s.chat, "text": "hi", "mentions": mentions })).await;
        assert!(error.contains("mentions"), "{mentions}: {error}");
    }
}

#[tokio::test]
async fn a_shared_string_field_tells_absent_from_wrong_type() {
    let s = Scratch::new();
    for (method, key) in [("chats.dm", "bot_id"), ("chats.stop", "chat_id"), ("routines.describe", "schedule")] {
        assert_eq!(s.call(method, json!({})).await.unwrap_err(), format!("missing {key}"));
        assert_eq!(s.call(method, json!({ key: "" })).await.unwrap_err(), format!("missing {key}"));
        assert_eq!(s.call(method, json!({ key: 5 })).await.unwrap_err(), format!("{key} must be a string"));
    }
    let bots = s.app.state.lock().unwrap().bots.len();
    assert_eq!(s.call("bots.delete", json!({ "id": 5 })).await.unwrap_err(), "id must be a string");
    assert_eq!(s.app.state.lock().unwrap().bots.len(), bots, "a rejected delete must keep the bot");
}

#[tokio::test]
async fn valid_sends_keep_working() {
    let s = Scratch::new();
    let (files, messages) = (s.files(), s.messages());

    let sent = s.call("chats.send", json!({ "chat_id": s.chat, "text": " hello ", "message_id": "m-1" })).await.unwrap();
    assert_eq!(sent["message"]["id"], "m-1");
    assert_eq!(sent["message"]["body"]["text"], "hello");
    assert_eq!(s.files(), files);

    for attachment_only in [json!({ "chat_id": s.chat, "attachments": [s.file()] }), json!({ "chat_id": s.chat, "text": "", "attachments": [s.file()], "mentions": [] })] {
        let before = s.files();
        let sent = s.call("chats.send", attachment_only).await.unwrap();
        assert_eq!(sent["message"]["body"]["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(s.files(), before + 1, "the attachment is copied exactly once");
    }
    assert!(s.messages() > messages);

    assert!(s.call("chats.send", json!({ "chat_id": s.chat, "text": "   " })).await.is_err(), "empty text and no files is still refused");
}
