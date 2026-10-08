//! Synthetic public API search: no relay, provider, attachment bytes or real account.
use beans::{api, app::App, config::Config, identity, model::{Attachment, Author, Body, Message}};
use serde_json::{json, Value};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn search(app: &std::sync::Arc<App>, query: &str) -> Value {
    api::dispatch(app, "chats.search", json!({"query": query, "limit": 1})).await.unwrap()
}

#[tokio::test]
async fn local_search_projects_files_and_survives_reopen_and_deletion() {
    let scratch = Scratch(std::env::temp_dir().join(format!("beans-search-{}", uuid::Uuid::new_v4())));
    let config = Config { home: scratch.0.clone(), port: 0 };
    let app = App::load(config.clone()).unwrap();
    identity::create(&app, Some("Search fixture".into())).unwrap();
    let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
    let mut message = Message::new(&chat, Author::You, Body::Text {
        text: "Release résumé 上海".into(),
        attachments: vec![Attachment { id: "fixture-file".into(), name: "manifest 上海.pdf".into(), mime: "application/pdf".into(), size: 1, width: None, height: None }],
        mentions: vec![], reply_to: None,
    });
    message.id = "search-message".into();
    app.upsert_message(message.clone(), false);
    // Newer orphan hits must not consume either category's limit.
    let mut orphan = message.clone();
    orphan.id = "orphan".into();
    orphan.chat_id = "absent-chat".into();
    orphan.created_at += 100.0;
    app.store.upsert(&orphan).unwrap();
    let expected = search(&app, "上海").await;
    assert_eq!(expected["messages"][0]["message_id"], "search-message");
    assert_eq!(expected["files"][0]["attachment_id"], "fixture-file");
    assert_eq!(expected["files"][0]["chat_id"], chat);
    assert!(search(&app, "manifest").await["messages"].as_array().unwrap().is_empty());
    assert_eq!(search(&app, "RELEASE résumé").await["messages"][0]["message_id"], "search-message");
    assert!(search(&app, " * \" ").await["files"].as_array().unwrap().is_empty());
    drop(app);
    let app = App::load(config).unwrap();
    assert_eq!(search(&app, "上海").await, expected);
    let Body::Text { attachments, .. } = &mut message.body else { unreachable!() };
    attachments.clear();
    app.upsert_message(message, false);
    assert!(search(&app, "manifest").await["files"].as_array().unwrap().is_empty());
    app.delete_chat(&chat);
    let deleted = search(&app, "上海").await;
    assert!(deleted["messages"].as_array().unwrap().is_empty());
    assert!(deleted["files"].as_array().unwrap().is_empty());
}
