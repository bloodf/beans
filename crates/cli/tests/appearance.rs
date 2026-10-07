use lorca::{api, app::App, config::Config, crypto, identity, model::{Bot, RosterBlob}};
use serde_json::{json, Value};

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn app(&self) -> std::sync::Arc<App> {
        App::load(Config { home: self.0.clone(), port: 0 }).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn setup() -> (Scratch, std::sync::Arc<App>, String) {
    let scratch = Scratch(std::env::temp_dir().join(format!("lorca-appearance-{}", uuid::Uuid::new_v4())));
    let app = scratch.app();
    identity::create(&app, Some("Appearance test".into())).unwrap();
    let runner = app.this_device_id().unwrap();
    (scratch, app, runner)
}
fn look() -> Value {
    json!({"version":1,"base":{"shape":"cloud","expression":"happy","background":"square","hue":359.5,"tone":"ink","palette":{"head":"#ABCDEF","eye":"#010203","bg":"#FFFFFF"},"motion":false},"states":{"working":{"shape":"boxy","palette":{"eye":"#102030"}},"error":{"expression":"sad"}}})
}
async fn create(app: &std::sync::Arc<App>, runner: &str, look: Value) -> Value {
    api::dispatch(app, "bots.create", json!({"id":"custom","name":"Custom","runner_id":runner,"look":look})).await.unwrap()["bot"].clone()
}
fn outbox(app: &App) -> Vec<(String, String, Vec<u8>)> {
    let db = rusqlite::Connection::open(app.config.home.join("lorca.sqlite3")).unwrap();
    let mut rows = db.prepare("SELECT id, kind, ciphertext FROM outbox ORDER BY id").unwrap();
    rows.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap()
}
fn files(app: &App) -> Vec<String> {
    if !app.config.files_dir().exists() { return Vec::new(); }
    let mut names: Vec<_> = std::fs::read_dir(app.config.files_dir()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn appearance_keep_reset_replace_and_photo_are_independent() {
    let (_scratch, app, runner) = setup();
    let original = look();
    assert_eq!(create(&app, &runner, original.clone()).await["look"], original);
    let image = app.config.home.join("photo.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3])).save(&image).unwrap();
    let uploaded = api::dispatch(&app, "bots.update", json!({"id":"custom","avatar":{"path":image,"mime":"image/png"}})).await.unwrap()["bot"].clone();
    assert_eq!(uploaded["look"], original);
    let photo = uploaded["avatar"].clone();
    let photo_id = photo["id"].as_str().unwrap();
    let changed = json!({"version":1,"base":{"tone":"pale"}});
    let replaced = api::dispatch(&app, "bots.update", json!({"id":"custom","look":changed})).await.unwrap()["bot"].clone();
    assert_eq!(replaced["look"], changed, "object replaces, never patches old fields or states");
    assert_eq!(replaced["avatar"], photo);
    let renamed = api::dispatch(&app, "bots.update", json!({"id":"custom","name":"Renamed"})).await.unwrap();
    assert_eq!(renamed["bot"]["look"], changed);
    let reset = api::dispatch(&app, "bots.update", json!({"id":"custom","look":null})).await.unwrap();
    assert!(reset["bot"].get("look").is_none());
    assert_eq!(reset["bot"]["avatar"], photo);
    assert!(app.config.files_dir().join(photo_id).is_file());
    assert!(app.state.lock().unwrap().blob_deletes.is_empty());
    api::dispatch(&app, "bots.update", json!({"id":"custom","look":original})).await.unwrap();
    let removed = api::dispatch(&app, "bots.update", json!({"id":"custom","avatar":null})).await.unwrap();
    assert_eq!(removed["bot"]["look"], original);
    assert!(removed["bot"].get("avatar").is_none());
    assert_eq!(removed["bot"]["id"], "custom");
}

#[tokio::test]
async fn invalid_appearance_has_no_profile_image_file_or_outbox_effects() {
    let (_scratch, app, runner) = setup();
    create(&app, &runner, look()).await;
    let image = app.config.home.join("photo.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3])).save(&image).unwrap();
    let before_bot = serde_json::to_value(app.bot("custom").unwrap()).unwrap();
    let before_queue = outbox(&app);
    let before_files = files(&app);
    let mut invalid = vec![json!(false), json!([]), json!({}), json!({"version":0,"base":{}}), json!({"version":2,"base":{}}), json!({"version":1}), json!({"version":1,"base":null}), json!({"version":1,"base":{},"overrides":{}}), json!({"version":1,"base":{},"states":{"unknown":{}}}), json!({"version":1,"base":{},"states":null})];
    for base in [json!({"shape":"blob"}), json!({"expression":"angry"}), json!({"background":"triangle"}), json!({"tone":0.5}), json!({"tone":"dark"}), json!({"hue":-0.1}), json!({"hue":360}), json!({"hue":"NaN"}), json!({"motion":1}), json!({"shape":null}), json!({"palette":null}), json!({"palette":{"head":"#fff"}}), json!({"palette":{"head":"#abcdef"}}), json!({"palette":{"eye":"rgb(1,2,3)"}}), json!({"palette":{"bg":"url(image)"}}), json!({"palette":{"body":"#FFFFFF"}}), json!({"palette":{"head":null}}), json!({"css":"color:red"}), json!({"svg":"<svg/>"}), json!({"hue":"() => 3"})] {
        invalid.push(json!({"version":1,"base":base}));
        invalid.push(json!({"version":1,"base":{},"states":{"thinking":base}}));
    }
    for value in invalid {
        for (method, id) in [("bots.create", "invalid"), ("bots.update", "custom")] {
            let result = api::dispatch(&app, method, json!({"id":id,"name":"Must not change","runner_id":runner,"look":value,"avatar":{"path":image,"mime":"image/png"}})).await;
            assert!(result.is_err(), "{method} accepted {value}");
            assert_eq!(serde_json::to_value(app.bot("custom").unwrap()).unwrap(), before_bot);
            assert!(app.bot("invalid").is_none());
            assert_eq!(outbox(&app), before_queue, "{method}: {value} changed encrypted queue");
            assert_eq!(files(&app), before_files, "{method}: {value} stored an image");
            assert!(app.state.lock().unwrap().blob_deletes.is_empty());
        }
    }
}

#[tokio::test]
async fn appearance_accepts_exact_enums_and_sparse_seeded_defaults() {
    let (_scratch, app, runner) = setup();
    assert_eq!(create(&app, &runner, json!({"version":1,"base":{}})).await["look"], json!({"version":1,"base":{}}));
    for (key, values) in [
        ("shape", vec!["round","organic","boxy","capsule","nub","cloud","droplet","hexagon","sun","triangle"]),
        ("expression", vec!["idle","happy","sad","mad","surprised","wink","sleepy","smug","unsure","scared","love","shy","sick","thinking"]),
        ("background", vec!["none","square","circle","squircle"]),
        ("tone", vec!["pastel","pale","mid","deep","bright","ink"]),
    ] {
        for value in values {
            let mut draft = json!({"version":1,"base":{}});
            draft["base"][key] = json!(value);
            let saved = api::dispatch(&app, "bots.update", json!({"id":"custom","look":draft})).await.unwrap();
            assert_eq!(saved["bot"]["look"], draft);
        }
    }
    for hue in [0.0, 359.999999] {
        let draft = json!({"version":1,"base":{"hue":hue},"states":{"idle":{},"thinking":{},"responding":{},"working":{},"waiting":{},"retry":{},"error":{}}});
        assert_eq!(api::dispatch(&app, "bots.update", json!({"id":"custom","look":draft})).await.unwrap()["bot"]["look"], draft);
    }
}

#[tokio::test]
async fn appearance_survives_sqlite_restart_and_encrypted_roster_round_trip() {
    let (scratch, app, runner) = setup();
    let original = look();
    create(&app, &runner, original.clone()).await;
    let queued = app.store.queued_roster().unwrap().unwrap();
    assert!(!queued.ciphertext.windows(7).any(|bytes| bytes == b"#ABCDEF"));
    let roster: RosterBlob = crypto::decrypt_json(&app.dek().unwrap(), "roster", &queued.ciphertext).unwrap();
    let wire = serde_json::to_value(&roster).unwrap();
    assert_eq!(wire["bots"].as_array().unwrap().iter().find(|bot| bot["id"] == "custom").unwrap()["look"], original);
    assert!(crypto::decrypt_json::<RosterBlob>(&[0;32], "roster", &queued.ciphertext).is_err());
    drop(app);
    let restarted = scratch.app();
    assert_eq!(serde_json::to_value(restarted.bot("custom").unwrap()).unwrap()["look"], original);
    assert_eq!(restarted.store.queued_roster().unwrap().unwrap().ciphertext, queued.ciphertext);
    api::dispatch(&restarted, "bots.update", json!({"id":"custom","look":null})).await.unwrap();
    drop(restarted);
    assert!(serde_json::to_value(scratch.app().bot("custom").unwrap()).unwrap().get("look").is_none());
}

#[tokio::test]
async fn legacy_defaults_and_future_appearance_fields_survive_unrelated_edits() {
    let (_scratch, app, runner) = setup();
    let legacy = json!({"id":"legacy","name":"Legacy","description":"","symbol_name":"sparkles","accent":"indigo","runner_id":runner,"provider":"deepseek","created_at":1});
    let bot: Bot = serde_json::from_value(legacy.clone()).unwrap();
    assert!(serde_json::to_value(&bot).unwrap().get("look").is_none());
    app.create_bot_with_dm(bot, None).unwrap();
    let future = json!({"version":1,"base":{"shape":"round","future_color":{"space":"oklab","value":[0.1,0.2,0.3]},"palette":{"head":"#ABCDEF","future_channel":"#010203"}},"states":{"working":{"future_motion":{"speed":2}}},"future_metadata":{"revision":7}});
    let mut wire = legacy;
    wire["id"] = json!("future");
    wire["look"] = future.clone();
    let bot: Bot = serde_json::from_value(wire).unwrap();
    app.create_bot_with_dm(bot, None).unwrap();
    let saved = api::dispatch(&app, "bots.update", json!({"id":"future","name":"Renamed"})).await.unwrap();
    assert_eq!(saved["bot"]["look"], future);
    assert!(api::dispatch(&app, "bots.update", json!({"id":"future","look":future})).await.is_err(), "unknown read fields are preserved, not accepted as authorable traits");
    let stored = app.store.load_state().unwrap();
    assert_eq!(serde_json::to_value(stored.bots.iter().find(|bot| bot.id == "future").unwrap()).unwrap()["look"], future);
}

#[tokio::test]
async fn direct_bot_mutations_reject_invalid_look_without_changing_saved_state() {
    let (_scratch, app, runner) = setup();
    let before = create(&app, &runner, look()).await;
    let before_queue = outbox(&app);
    let invalid = json!({"version":1,"base":{"hue":360}});
    let mut wire = before.clone();
    wire["look"] = invalid;
    wire["name"] = json!("Must not change");
    let invalid_bot: Bot = serde_json::from_value(wire).unwrap();
    assert!(app.update_bot("custom", |bot| *bot = invalid_bot.clone()).is_err());
    assert_eq!(serde_json::to_value(app.bot("custom").unwrap()).unwrap(), before);
    assert_eq!(outbox(&app), before_queue);
    let mut new_bot = invalid_bot;
    new_bot.id = "invalid-direct".into();
    assert!(app.create_bot_with_dm(new_bot, None).is_err());
    assert!(app.bot("invalid-direct").is_none());
    assert_eq!(outbox(&app), before_queue);
}
