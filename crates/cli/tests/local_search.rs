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

// The pre-index implementation, verbatim in behavior: decode every row, newest first, skip
// orphans, fill each category to its limit. The indexed search must return the same JSON.
fn oracle_terms(input: &str) -> Vec<String> {
    input.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_lowercase).collect()
}
fn oracle_matches(text: &str, terms: &[String]) -> bool {
    let text = text.to_lowercase();
    terms.iter().all(|term| text.contains(term))
}
fn oracle_snippet(text: &str, terms: &[String]) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() { return String::new(); }
    let matched = words.iter().position(|w| { let w = w.to_lowercase(); terms.iter().any(|t| w.contains(t)) }).unwrap_or(0);
    let (start, end) = (matched.saturating_sub(8), (matched + 16).min(words.len()));
    format!("{}{}{}", if start > 0 { "… " } else { "" }, words[start..end].join(" "), if end < words.len() { " …" } else { "" })
}
fn oracle(app: &App, query: &str, limit: usize) -> (Value, Value) {
    let terms = oracle_terms(query);
    let state = app.state.lock().unwrap();
    let mut rows = Vec::new();
    for chat in &state.chats {
        for (position, message) in app.store.all(&chat.meta.id).unwrap().into_iter().enumerate() {
            rows.push((position, message));
        }
    }
    rows.sort_by(|(ap, a), (bp, b)| b.created_at.partial_cmp(&a.created_at).unwrap()
        .then_with(|| a.chat_id.cmp(&b.chat_id)).then_with(|| bp.cmp(ap)).then_with(|| a.id.cmp(&b.id)));
    let (mut messages, mut files) = (Vec::new(), Vec::new());
    for (_, message) in rows {
        let text = match &message.body {
            Body::Text { text, attachments, .. } => {
                for attachment in attachments {
                    if files.len() < limit && oracle_matches(&attachment.name, &terms) {
                        files.push(json!({"chat_id": message.chat_id, "message_id": message.id, "attachment_id": attachment.id,
                            "name": attachment.name, "snippet": oracle_snippet(&attachment.name, &terms), "created_at": message.created_at}));
                    }
                }
                text.as_str()
            }
            Body::Handoff { reason, .. } => reason,
            Body::Notice { text, .. } => text,
            Body::Permission { summary, .. } => summary,
            Body::Tool { .. } => continue,
        };
        if messages.len() < limit && oracle_matches(text, &terms) {
            messages.push(json!({"chat_id": message.chat_id, "message_id": message.id, "snippet": oracle_snippet(text, &terms),
                "author": message.author, "created_at": message.created_at}));
        }
        if messages.len() == limit && files.len() == limit { break; }
    }
    (Value::Array(messages), Value::Array(files))
}

#[tokio::test]
async fn indexed_search_matches_the_full_decoding_scan() {
    let scratch = Scratch(tempfile::tempdir().unwrap().keep());
    let config = Config { home: scratch.0.clone(), port: 0 };
    let app = App::load(config.clone()).unwrap();
    identity::create(&app, Some("Search fixture".into())).unwrap();
    let bot = app.state.lock().unwrap().bots[0].id.clone();
    let mut chats = vec![app.state.lock().unwrap().chats[0].meta.id.clone()];
    for title in ["Ops", "Travel"] {
        let meta = beans::model::ChatMeta { id: String::new(), kind: "group".into(), title: Some(title.into()), bot_ids: vec![bot.clone()],
            owner_bot_id: None, description: None, is_pinned: false, created_at: 0.0 };
        chats.push(app.create_chat(meta).unwrap().meta.id);
    }
    chats.push("orphan-chat".into());
    let words = ["Release", "résumé", "RÉSUMÉ", "上海市", "在上海", "東京タワー", "ΣΊΣΥΦΟΣ", "σίσυφος", "ΟΔΟΣ",
        "İstanbul", "istanbul", "Straße", "STRASSE", "deploy", "redeployed", "Infrastructure", "naïve", "ǅemal", "ﬁle", "a", "x1"];
    let names = ["manifest 上海.pdf", "Résumé.DOCX", "alpha.txt", "beta notes.md", "ΟΔΟΣ.png", "İZMİR.csv"];
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let mut next = |n: usize| { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((seed >> 33) as usize) % n };
    let mut stored = Vec::new();
    for i in 0..400 {
        let text = (0..1 + next(6)).map(|_| words[next(words.len())]).collect::<Vec<_>>().join(if next(3) == 0 { "-" } else { " " });
        let body = match next(6) {
            0 => Body::Notice { text, routine_id: None },
            1 => Body::Handoff { from: bot.clone(), to: bot.clone(), reason: text },
            2 => serde_json::from_value(json!({"kind": "tool", "name": "bash", "summary": text, "detail": text, "is_running": false})).unwrap(),
            3 => serde_json::from_value(json!({"kind": "permission", "plugin_id": "p", "plugin_name": "P", "tool": "t", "summary": text, "decision": "pending"})).unwrap(),
            _ => Body::Text { text, mentions: vec![], reply_to: None, attachments: (0..next(3)).map(|n| Attachment {
                id: format!("file-{i}-{n}"), name: names[next(names.len())].into(), mime: "application/octet-stream".into(), size: 1, width: None, height: None,
            }).collect() },
        };
        let mut message = Message::new(&chats[next(chats.len())], if next(2) == 0 { Author::You } else { Author::System }, body);
        message.id = format!("m-{i:03}");
        message.created_at = (1 + next(4)) as f64;
        stored.push(message);
    }
    for message in &stored[..300] { app.store.upsert(message).unwrap(); }
    // Older pages land before what is here, including a page meeting rows already stored.
    let older: Vec<Message> = stored[280..].iter().rev().cloned().collect();
    app.store.insert_older(&older).unwrap();
    for message in stored.iter_mut().step_by(7) {
        if let Body::Text { text, .. } | Body::Notice { text, .. } = &mut message.body { *text = format!("edited {text} ΣΊΣΥΦΟΣ"); }
        app.store.upsert(message).unwrap();
    }
    for message in stored.iter().step_by(11) { app.store.remove(&message.chat_id, &message.id).unwrap(); }
    app.store.set_history_before("orphan-chat", Some(5)).unwrap();
    let queries = ["上海", "海", "release", "RÉSUMÉ", "sumé", "σίσυφος", "ΣΊΣΥΦΟΣ", "οδος", "İstanbul", "i̇stanbul",
        "straße", "strasse", "ploy infra", "deploy 上海", "pdf", "manifest 上海", "alpha beta", "docx résumé", "a", "e", "ǆ", "fi",
        "ﬁle", "edited", "x1 edited", "İZMİR", "izmir", "tool bash", "nothing-matches-this"];
    let check = |app: &std::sync::Arc<App>| {
        let app = app.clone();
        async move {
            for query in queries {
                for limit in [1, 3, 50] {
                    let actual = api::dispatch(&app, "chats.search", json!({"query": query, "limit": limit})).await.unwrap();
                    let (messages, files) = oracle(&app, query, limit);
                    assert_eq!(actual["messages"], messages, "messages for {query:?} limit {limit}");
                    assert_eq!(actual["files"], files, "files for {query:?} limit {limit}");
                    assert_eq!(actual["history_complete"], true);
                }
            }
        }
    };
    check(&app).await;
    app.store.set_history_before(&chats[1], Some(5)).unwrap();
    assert_eq!(search(&app, "release").await["history_complete"], false);
    app.store.set_history_before(&chats[1], None).unwrap();
    drop(app);
    let app = App::load(config).unwrap();
    check(&app).await;
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
