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
    queued: Vec<Vec<String>>,
    sent: Vec<Vec<String>>,
    roster: String,
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
        let db = rusqlite::Connection::open_with_flags(self.app.config.database_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let rows = |sql: &str| {
            let mut statement = db.prepare(sql).unwrap();
            let count = statement.column_count();
            statement.query_map([], |row| {
                (0..count).map(|i| row.get_ref(i).map(|value| format!("{value:?}"))).collect::<rusqlite::Result<Vec<_>>>()
            }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        let queued = rows("SELECT * FROM outbox ORDER BY position");
        let sent = rows("SELECT * FROM sent_jobs ORDER BY id");
        let roster = serde_json::to_string(&self.app.snapshot()).unwrap();
        Snapshot { files, messages, jobs, queued, sent, roster }
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

#[tokio::test]
async fn failed_preparation_preserves_existing_bytes_and_queued_slots() {
    for paused in [true, false] {
        let s = Scratch::new(true);
        let mut file = s.file();
        file["id"] = json!("att-existing");
        s.call("chats.send", json!({"chat_id": s.chat, "text": "original", "message_id": "m-existing", "attachments": [file]})).await.unwrap();
        s.app.store.insert_sent_job(&beans::app::SentJob {
            id: "waiting-job".into(), chat_id: s.chat.clone(), bot_id: "fixture-bot".into(),
            routine_id: None, runner_id: "fixture-runner".into(), sent_at: 1.0,
        }).unwrap();
        s.app.set_paused(paused);
        // Both an existing slot and a new id must survive a later missing source unchanged.
        for id in ["att-existing", "att-new"] {
            file["id"] = json!(id);
            std::fs::write(&s.attachment, b"replacement").unwrap();
            s.send_rejected(json!({"message_id": "m-existing", "attachments": [file, {"path": s.home.join("missing")}]}), "No such file").await;
        }
        assert_eq!(std::fs::read(s.app.config.files_dir().join("att-existing")).unwrap(), b"fixture");
        s.rejected("bots.update", json!({"id": "unknown", "avatar": {"path": s.attachment, "mime": "image/png"}}), "Unknown bot").await;
    }
}

#[cfg(feature = "server")]
#[tokio::test]
async fn reference_frames_round_trip_over_ws() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Frame;
    let mut s = Scratch::new(true);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    // Reopen the same synthetic, paused home; no sync loop or provider is started.
    s.app = App::load(Config { home: s.home.clone(), port }).unwrap();
    let server = tokio::spawn(beans::ws::serve(s.app.clone(), false));
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(socket) = tokio_tungstenite::connect_async(&url).await { break socket; }
            assert!(!server.is_finished(), "server exited before connection");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    async fn exchange(socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, frame: String, id: Value) -> Value {
        socket.send(Frame::Text(frame.into())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let Frame::Text(text) = frame {
                    let reply: Value = serde_json::from_str(&text).unwrap();
                    if reply.get("id") == Some(&id) { break reply; }
                    assert!(reply.get("event").is_some(), "unexpected frame: {reply}");
                }
            }
        }).await.unwrap()
    }
    // Exact documented frames, substituting only the chat id.
    let hello = exchange(&mut socket, r#"{"id":1,"method":"hello","params":{}}"#.into(), json!(1)).await;
    assert_eq!(hello["result"]["has_identity"], true);
    let send = r#"{"id":2,"method":"chats.send","params":{"chat_id":"<chat id>","text":"Hello"}}"#.replace("<chat id>", &s.chat);
    let reply = exchange(&mut socket, send, json!(2)).await;
    let id = reply["result"]["message"]["id"].as_str().unwrap();
    let stored = s.app.message(&s.chat, id).unwrap();
    assert_eq!(stored.author, Author::You);
    assert!(matches!(stored.body, Body::Text { ref text, .. } if text == "Hello"));
    for params in [json!({"chat_id": s.chat, "text": 5}), json!({"chat_id": s.chat})] {
        let before = s.snapshot();
        let reply = exchange(&mut socket, json!({"id": "field", "method": "chats.send", "params": params}).to_string(), json!("field")).await;
        assert!(reply["error"]["message"].as_str().unwrap().contains("text"));
        assert_eq!(s.snapshot(), before);
    }
    let reply = exchange(&mut socket, json!({"id": 3, "method": "chats.send", "params": {"chat_id": s.chat, "attachments": [s.file()]}}).to_string(), json!(3)).await;
    let stored = s.app.message(&s.chat, reply["result"]["message"]["id"].as_str().unwrap()).unwrap();
    assert!(matches!(stored.body, Body::Text { ref text, ref attachments, .. } if text.is_empty() && attachments[0].size == 7));
    for frame in [r#"{"id":4,"method":"hello"}"#, r#"{"id":4,"method":"hello","params":null}"#] {
        assert_eq!(exchange(&mut socket, frame.into(), json!(4)).await["result"], hello["result"]);
    }
    assert_eq!(exchange(&mut socket, r#"{"id":5,"method":"unknown"}"#.into(), json!(5)).await, json!({"id":5,"error":{"message":"unknown method unknown"}}));
    let malformed = exchange(&mut socket, "[]".into(), Value::Null).await;
    assert!(malformed["error"]["message"].as_str().unwrap().starts_with("bad request: "));
    assert!(s.app.running_jobs.lock().unwrap().is_empty());
    assert!(s.app.store.sent_jobs().unwrap().is_empty());
    #[cfg(feature = "cli")]
    {
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_beans")).args(["--home", s.home.to_str().unwrap(), "status"]).output().unwrap();
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        let status: Value = serde_json::from_slice(&status.stdout).unwrap();
        assert_eq!(status, s.app.snapshot());
    }
    socket.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
}
