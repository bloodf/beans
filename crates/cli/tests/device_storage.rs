#![cfg(not(feature = "runner"))]

use beans::{app::App, config::Config, model::{Author, Body, Chat, ChatMeta, Message}};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn device_storage_keeps_only_the_app_view_of_tools() {
    let scratch = Scratch(std::env::temp_dir().join(format!("beans-device-storage-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: scratch.0.clone(), port: 0 }).unwrap();
    app.state.lock().unwrap().chats.push(Chat {
        meta: ChatMeta {
            id: "chat".into(), kind: "group".into(), title: None, bot_ids: Vec::new(),
            owner_bot_id: None, description: None, is_pinned: false, created_at: 1.0,
        },
        unread_count: 0, usage: None, compactions: Vec::new(),
    });
    let message = Message::new("chat", Author::Bot { bot_id: "bot".into() }, Body::Tool {
        name: "bash".into(), summary: "Ran a command".into(), detail: "d".repeat(1_000),
        is_running: false, call_id: "call".into(),
        arguments: serde_json::json!({ "command": "echo secret" }),
        result: Some("secret output".into()), is_error: false, description: None,
        target_bot_id: None, script_command: None, run: None,
    });
    let id = message.id.clone();
    app.upsert_message(message, false);
    // App::message reads SQLite directly; do not project through Message::for_app here.
    let stored = app.message("chat", &id).unwrap();
    let Body::Tool { detail, arguments, result, .. } = stored.body else { panic!("a tool row") };
    assert_eq!(detail.chars().count(), 400);
    assert!(arguments.is_null());
    assert_eq!(result, None);
}
