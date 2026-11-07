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
    let scratch = Scratch(tempfile::tempdir().unwrap().keep());
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

#[tokio::test]
async fn search_reads_current_chat_bot_message_and_file_content() {
    let scratch = Scratch(tempfile::tempdir().unwrap().keep());
    let app = App::load(Config { home: scratch.0.clone(), port: 0 }).unwrap();
    identity::create(&app, Some("Search fixture".into())).unwrap();
    let (chat, bot) = {
        let state = app.state.lock().unwrap();
        (state.chats[0].meta.id.clone(), state.bots[0].id.clone())
    };
    app.update_chat_meta(&chat, |meta| {
        meta.title = Some("needle title".into());
        meta.bot_ids = vec![bot.clone()];
    }).unwrap();
    {
        let mut state = app.state.lock().unwrap();
        let profile = state.bots.iter_mut().find(|item| item.id == bot).unwrap();
        profile.name = "needle bot".into();
        profile.description.clear();
    }
    let mut message = Message::new(&chat, Author::You, Body::Text {
        text: "needle message".into(),
        attachments: vec![Attachment { id: "file".into(), name: "needle file.pdf".into(), mime: "application/pdf".into(), size: 1, width: None, height: None }],
        mentions: vec![], reply_to: None,
    });
    app.upsert_message(message.clone(), false);
    let found = search(&app, "needle").await;
    assert!(found["chats"][0]["snippet"].as_str().unwrap().contains("needle title"));
    assert_eq!(found["messages"][0]["snippet"], "needle message");
    assert_eq!(found["files"][0]["snippet"], "needle file.pdf");
    app.update_chat_meta(&chat, |meta| meta.title = Some("other title".into())).unwrap();
    assert!(search(&app, "needle").await["chats"][0]["snippet"].as_str().unwrap().contains("needle bot"));
    app.state.lock().unwrap().bots.iter_mut().find(|item| item.id == bot).unwrap().name = "other bot".into();
    let Body::Text { text, attachments, .. } = &mut message.body else { unreachable!() };
    *text = "other message".into();
    attachments[0].name = "other file.pdf".into();
    app.upsert_message(message, false);
    let removed = search(&app, "needle").await;
    for category in ["chats", "messages", "files"] {
        assert!(removed[category].as_array().unwrap().is_empty(), "{category}: {removed}");
    }
}
