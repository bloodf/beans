use super::super::mock_hub::{MockHub, Serve};
use super::super::CatalogModel;
use super::*;
use crate::config::Config;

struct Scratch(Arc<App>, PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

/// An app with an identity, whose own Device is a Runner on the machines these tests run on,
/// offering the built-in catalog.
fn shipped_runner() -> Scratch {
    let home = std::env::temp_dir().join(format!("beans-schedule-models-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&home).unwrap();
    let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
    crate::identity::create(&app, Some("Runner".into())).unwrap();
    assert!(app.local_device().unwrap().is_runner());
    Scratch(app, home)
}

/// As `shipped_runner`, offering `fixture_catalog()`.
fn runner() -> Scratch {
    let scratch = shipped_runner();
    scratch.0.schedule_models.set_catalog(fixture_catalog());
    scratch
}

/// Models whose pinning these tests decide, built from shipped entries: a static model not
/// pinned, the same model pinned to `supplied_files`, and an ONNX model not pinned.
fn fixture_catalog() -> Catalog {
    let shipped = super::super::catalog();
    let unpin = |model: &mut CatalogModel| {
        for file in [model.files.weights.as_mut(), model.files.model.as_mut(), Some(&mut model.files.tokenizer)].into_iter().flatten() {
            file.bytes = None;
            file.sha256 = None;
        }
        model.approx_bytes = Some(1_000_000);
    };
    let mut unpinned = shipped.models.iter().find(|m| m.engine == Engine::Static).unwrap().clone();
    unpin(&mut unpinned);
    unpinned.id = "fixture-static".into();
    unpinned.static_spec = Some(super::super::StaticSpec {
        tensor: "embedding.weight".into(),
        dimensions: 128,
        add_special_tokens: false,
    });
    let mut pinned = unpinned.clone();
    pinned.id = "fixture-static-pinned".into();
    let (weights, tokenizer) = fixture_bytes();
    let pin = |bytes: &[u8]| (Some(bytes.len() as u64), Some(format!("{:x}", Sha256::digest(bytes))));
    let weights_file = pinned.files.weights.as_mut().unwrap();
    (weights_file.bytes, weights_file.sha256) = pin(&weights);
    (pinned.files.tokenizer.bytes, pinned.files.tokenizer.sha256) = pin(&tokenizer);
    let mut onnx = shipped.models.iter().find(|m| m.engine == Engine::Onnx && m.onnx.as_ref().unwrap().dimensions == 384).unwrap().clone();
    unpin(&mut onnx);
    onnx.id = "fixture-onnx".into();
    let catalog = Catalog {
        version: 1,
        note: "fixtures".into(),
        models: vec![unpinned, pinned, onnx],
    };
    catalog.validate().unwrap();
    catalog
}

fn me(app: &App) -> String {
    app.this_device_id().unwrap()
}

async fn call(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    serve(app, method, params, &me(app)).await
}

/// Safetensors bytes with one `rows × columns` F32 tensor.
fn safetensors(name: &str, rows: usize, columns: usize, value: impl Fn(usize, usize) -> f32) -> Vec<u8> {
    let data: Vec<u8> = (0..rows * columns).flat_map(|i| value(i / columns, i % columns).to_le_bytes()).collect();
    let header = json!({ name: { "dtype": "F32", "shape": [rows, columns], "data_offsets": [0, data.len()] } }).to_string();
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(&data);
    bytes
}

/// A small table of the fixture models' width, and the fixture tokenizer.
fn fixture_bytes() -> (Vec<u8>, Vec<u8>) {
    let weights = safetensors("embedding.weight", 16, 128, |row, column| ((row * 7 + column * 3) % 11) as f32 / 11.0 - 0.5);
    (weights, include_bytes!("../embeddings/fixtures/tiny-tokenizer.json").to_vec())
}

/// `fixture_bytes` as files in `dir`.
fn supplied_files(dir: &Path) -> (PathBuf, PathBuf) {
    let (weights_bytes, tokenizer_bytes) = fixture_bytes();
    let weights = dir.join("weights.safetensors");
    std::fs::write(&weights, weights_bytes).unwrap();
    let tokenizer = dir.join("tokenizer.json");
    std::fs::write(&tokenizer, tokenizer_bytes).unwrap();
    (weights, tokenizer)
}

fn supplied(weights: &Path, tokenizer: &Path) -> Value {
    json!({ "kind": "supplied", "files": { "weights": weights, "tokenizer": tokenizer } })
}

#[tokio::test]
async fn the_catalog_lists_with_its_downloads_and_nothing_installed() {
    let scratch = runner();
    let app = &scratch.0;
    let list = call(app, "schedule_models.list", json!({})).await.unwrap();
    assert_eq!(list["mode"], "reader", "the small-model read stays the default");
    assert_eq!(list["selected"], Value::Null);
    assert_eq!(list["max_bytes"], MAX_MODEL_BYTES);
    let models = list["models"].as_array().unwrap();
    assert_eq!(models.len(), 3);
    for model in models {
        assert_eq!(model["installed"], false);
        assert!(model["bytes"].as_u64().unwrap() <= MAX_MODEL_BYTES);
        for download in model["downloads"].as_array().unwrap() {
            assert!(download["url"].as_str().unwrap().starts_with("https://huggingface.co/"));
        }
    }
    assert_eq!(list["runs_models"], cfg!(feature = "embedding-local"));
    let status = call(app, "schedule_models.status", json!({})).await.unwrap();
    assert_eq!((status["mode"].as_str(), status["state"].as_str()), (Some("reader"), Some("none_selected")));
    assert_eq!(call(app, "schedule_models.delete_everything", json!({})).await.unwrap_err(), "unknown_schedule_models_method");
}

/// A download is only ever of pinned files, from the catalog's URLs; preview does no I/O.
#[tokio::test]
async fn only_pinned_models_download() {
    let scratch = runner();
    let app = &scratch.0;
    let unpinned = call(app, "schedule_models.preview", json!({ "model_id": "fixture-static", "source": { "kind": "download" } })).await;
    assert_eq!(unpinned.unwrap_err(), "model_not_pinned");
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "fixture-static-pinned", "source": { "kind": "download" } }))
        .await
        .unwrap();
    assert_eq!((preview["pinned"].as_bool(), preview["source"].as_str(), preview["hub"].as_str()), (Some(true), Some("download"), Some("Hugging Face")));
    let (weights, tokenizer) = fixture_bytes();
    assert_eq!(preview["total_bytes"], (weights.len() + tokenizer.len()) as u64);
    let urls: Vec<&str> = preview["assets"].as_array().unwrap().iter().map(|a| a["source"]["url"].as_str().unwrap()).collect();
    let model = catalog(app).model("fixture-static-pinned").unwrap();
    let (weights_path, tokenizer_path) = (&model.files.weights.as_ref().unwrap().path, &model.files.tokenizer.path);
    assert_eq!(
        urls,
        [
            format!("https://huggingface.co/{}/resolve/{}/{weights_path}", model.repo, model.revision),
            format!("https://huggingface.co/{}/resolve/{}/{tokenizer_path}", model.repo, model.revision)
        ]
    );
    assert!(!app.config.home.join(ASSET_DIR).exists(), "a preview downloads nothing");
    assert_eq!(
        call(app, "schedule_models.preview", json!({ "model_id": "nope", "source": { "kind": "download" } })).await.unwrap_err(),
        "unknown_model"
    );
    assert_eq!(
        call(app, "schedule_models.preview", json!({ "model_id": "fixture-static-pinned", "source": { "kind": "torrent" } }))
            .await
            .unwrap_err(),
        "invalid_request"
    );
}

/// Supplied files are hashed as they are, and a pinned model's must be the pinned files.
#[tokio::test]
async fn supplied_files_are_checked_before_approval() {
    let scratch = runner();
    let app = &scratch.0;
    let (weights, tokenizer) = supplied_files(&scratch.1);
    let preview = |model: &str, source: Value, runtime: Option<&Path>| {
        let mut params = json!({ "model_id": model, "source": source });
        if let Some(runtime) = runtime {
            params["runtime"] = json!(runtime);
        }
        call(app, "schedule_models.preview", params)
    };
    // A pinned model's files must be exactly the pinned ones.
    let other = scratch.1.join("other.safetensors");
    std::fs::write(&other, safetensors("embedding.weight", 16, 128, |_, _| 0.25)).unwrap();
    assert_eq!(preview("fixture-static-pinned", supplied(&other, &tokenizer), None).await.unwrap_err(), "asset_mismatch");
    assert_eq!(preview("fixture-static-pinned", supplied(&weights, &tokenizer), None).await.unwrap()["pinned"], true);
    assert_eq!(
        preview("fixture-static", supplied(Path::new("weights.safetensors"), &tokenizer), None).await.unwrap_err(),
        "absolute_path_required"
    );
    assert_eq!(preview("fixture-static", supplied(&scratch.1.join("missing"), &tokenizer), None).await.unwrap_err(), "file_unavailable");
    assert_eq!(
        preview("fixture-static", json!({ "kind": "supplied", "files": { "weights": weights } }), None).await.unwrap_err(),
        "file_required"
    );
    let extra = json!({ "kind": "supplied", "files": { "weights": weights, "tokenizer": tokenizer, "model": weights } });
    assert_eq!(preview("fixture-static", extra, None).await.unwrap_err(), "unknown_file_role");
    assert_eq!(preview("fixture-static", supplied(&weights, &tokenizer), Some(&tokenizer)).await.unwrap_err(), "runtime_not_needed");
    // An ONNX model needs a runtime: with none beside the binary and none to download, the
    // user must name one.
    app.schedule_models.set_runtime_supply(RuntimeSupply::default());
    let onnx = json!({ "kind": "supplied", "files": { "model": weights, "tokenizer": tokenizer } });
    assert_eq!(preview("fixture-onnx", onnx.clone(), None).await.unwrap_err(), "runtime_required");
    let with_runtime = preview("fixture-onnx", onnx, Some(&weights)).await.unwrap();
    let kinds: Vec<&str> = with_runtime["assets"].as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["model", "tokenizer", "runtime"]);

    let preview = preview("fixture-static", supplied(&weights, &tokenizer), None).await.unwrap();
    assert_eq!((preview["pinned"].as_bool(), preview["source"].as_str()), (Some(false), Some("supplied")));
    assert!(preview["hub"].is_null(), "supplied files come from no hub");
    let weights_asset = &preview["assets"][0];
    assert_eq!(weights_asset["kind"], "weights");
    assert_eq!(weights_asset["bytes"], std::fs::metadata(&weights).unwrap().len());
    assert_eq!(weights_asset["sha256"], format!("{:x}", Sha256::digest(std::fs::read(&weights).unwrap())));
}

/// Applying takes the one preview it names, once, with the user's confirmation, and installs
/// exactly the previewed bytes. Selecting it loads it and trains its classifier once.
#[tokio::test]
async fn an_approved_install_selects_reads_and_removes() {
    let scratch = runner();
    let app = &scratch.0;
    let (weights, tokenizer) = supplied_files(&scratch.1);
    let preview_params = json!({ "model_id": "fixture-static", "source": supplied(&weights, &tokenizer) });
    let preview = call(app, "schedule_models.preview", preview_params.clone()).await.unwrap();
    let apply = |preview: &Value, digest: &str, confirm: bool| {
        call(
            app,
            "schedule_models.apply",
            json!({ "preview_token": preview["preview_token"], "preview_digest": digest, "confirm": confirm }),
        )
    };
    let digest = preview["preview_digest"].as_str().unwrap().to_string();
    assert_eq!(apply(&preview, &digest, false).await.unwrap_err(), "confirmation_required");
    assert_eq!(apply(&preview, "another", true).await.unwrap_err(), "approval_stale");
    assert_eq!(apply(&preview, &digest, true).await.unwrap_err(), "approval_not_found", "a refused approval is spent");

    // Files changed after the preview are not installed.
    let preview = call(app, "schedule_models.preview", preview_params.clone()).await.unwrap();
    let original = std::fs::read(&weights).unwrap();
    std::fs::write(&weights, b"changed after approval").unwrap();
    let digest = preview["preview_digest"].as_str().unwrap().to_string();
    assert_eq!(apply(&preview, &digest, true).await.unwrap_err(), "asset_install_failed");
    assert_eq!(call(app, "schedule_models.status", json!({})).await.unwrap()["installed"], json!([]));
    std::fs::write(&weights, &original).unwrap();

    let preview = call(app, "schedule_models.preview", preview_params).await.unwrap();
    let digest = preview["preview_digest"].as_str().unwrap().to_string();
    let installed = apply(&preview, &digest, true).await.unwrap();
    let loaded_state = if cfg!(feature = "embedding-local") { "ready" } else { "runtime_unavailable" };
    assert_eq!((installed["installed"].as_bool(), installed["state"].as_str()), (Some(true), Some(loaded_state)));
    let saved = saved(app).unwrap();
    let install = &saved.installs["fixture-static"];
    assert_eq!(std::fs::read(&install.assets.get(AssetKind::Weights).unwrap().path).unwrap(), original);
    assert_eq!(install.classifier.is_some(), cfg!(feature = "embedding-local"), "trained once, and kept");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(app.config.home.join(Saved::PATH)).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let list = call(app, "schedule_models.list", json!({})).await.unwrap();
    let listed = list["models"].as_array().unwrap().iter().find(|m| m["id"] == "fixture-static").unwrap();
    assert_eq!((listed["installed"].as_bool(), listed["selected"].as_bool()), (Some(true), Some(false)));

    // Local modes name an installed model.
    let select = |params: Value| call(app, "schedule_models.select", params);
    assert_eq!(select(json!({ "mode": "local" })).await.unwrap_err(), "model_required");
    assert_eq!(select(json!({ "mode": "local", "model_id": "fixture-static-pinned" })).await.unwrap_err(), "model_not_installed");
    assert_eq!(select(json!({ "mode": "sometimes" })).await.unwrap_err(), "invalid_request");
    let selected = select(json!({ "mode": "local", "model_id": "fixture-static" })).await.unwrap();
    assert_eq!((selected["mode"].as_str(), selected["state"].as_str()), (Some("local"), Some(loaded_state)));
    assert_eq!(mode(app), Detection::Local);
    let status = call(app, "schedule_models.status", json!({})).await.unwrap();
    assert_eq!(status["state"], loaded_state);
    let found = read(app, "Summarize my inbox, and run this daily", &CancellationToken::new()).await;
    #[cfg(feature = "embedding-local")]
    {
        let (words, score) = found.unwrap();
        assert!(score.is_finite());
        assert!(crate::schedule_models::classifier::parts("Summarize my inbox, and run this daily").contains(&words));
    }
    #[cfg(not(feature = "embedding-local"))]
    assert_eq!(found, None);

    // Removing the selected model leaves nothing selected, and a local mode reads nothing.
    let directory = install.assets.get(AssetKind::Weights).unwrap().path.parent().unwrap().to_path_buf();
    let removed = call(app, "schedule_models.remove", json!({ "model_id": "fixture-static" })).await.unwrap();
    assert_eq!((removed["mode"].as_str(), removed["selected"].as_str()), (Some("local"), None));
    assert!(!directory.exists());
    assert_eq!(read(app, "run this daily", &CancellationToken::new()).await, None);
    assert_eq!(call(app, "schedule_models.remove", json!({ "model_id": "fixture-static" })).await.unwrap_err(), "model_not_installed");

    // Back to the small model, or off.
    assert_eq!(select(json!({ "mode": "reader" })).await.unwrap()["state"], "none_selected");
    assert_eq!(mode(app), Detection::Reader);
    select(json!({ "mode": "off" })).await.unwrap();
    assert_eq!(mode(app), Detection::Off);
}

/// An approval belongs to the Device that asked for it and the account it was asked in, and a
/// Device no longer paired, or a paused account, can do nothing.
#[tokio::test]
async fn approvals_belong_to_their_requester_and_account() {
    let scratch = runner();
    let app = &scratch.0;
    let (weights, tokenizer) = supplied_files(&scratch.1);
    app.state.lock().unwrap().listed_machines.insert("phone".into(), 1);
    let params = json!({ "model_id": "fixture-static", "source": supplied(&weights, &tokenizer) });
    let preview = serve(app, "schedule_models.preview", params.clone(), "phone").await.unwrap();
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply.clone()).await.unwrap_err(), "approval_stale", "the Runner did not ask for it");

    assert_eq!(serve(app, "schedule_models.list", json!({}), "stranger").await.unwrap_err(), "requester_revoked");
    let preview = serve(app, "schedule_models.preview", params.clone(), "phone").await.unwrap();
    app.state.lock().unwrap().listed_machines.remove("phone");
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(serve(app, "schedule_models.apply", apply, "phone").await.unwrap_err(), "requester_revoked");

    app.state.lock().unwrap().paused = true;
    assert_eq!(call(app, "schedule_models.list", json!({})).await.unwrap_err(), "account_paused");
    app.state.lock().unwrap().paused = false;

    // Pausing or forgetting the account drops every approval.
    let preview = call(app, "schedule_models.preview", params).await.unwrap();
    app.schedule_models.clear();
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap_err(), "approval_not_found");
}

/// Another account's saved state reads as nothing saved.
#[tokio::test]
async fn saved_state_is_the_accounts_own() {
    let scratch = runner();
    let app = &scratch.0;
    edit(
        app,
        || Ok(()),
        |saved| {
            saved.mode = Detection::Off;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(mode(app), Detection::Off);
    let path = app.config.home.join(Saved::PATH);
    let mut other: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    other["account"] = "someone else".into();
    crate::config::write_json_private(&path, &other).unwrap();
    app.schedule_models.clear();
    assert_eq!(mode(app), Detection::Reader);
}

/// The measured model's real files download as the catalog pins them, through a redirect to
/// a CDN host, exactly as from Hugging Face; a file that is not the pinned one installs nothing.
/// Set `BEANS_SCHEDULE_MODEL_DIR` as for the test below and run with `--ignored`.
#[tokio::test]
#[ignore = "needs BEANS_SCHEDULE_MODEL_DIR with the wordllama-l2-256 files"]
async fn the_measured_model_downloads_through_a_redirect() {
    let dir = PathBuf::from(std::env::var("BEANS_SCHEDULE_MODEL_DIR").expect("BEANS_SCHEDULE_MODEL_DIR"));
    let read_file = |name: &str| (name.to_string(), std::fs::read(dir.join(name)).unwrap());
    let files = [read_file("l2_supercat_256.safetensors"), read_file("l2_supercat_tokenizer_config.json")];
    let scratch = shipped_runner();
    let app = &scratch.0;
    let hub = MockHub::start(files.iter().map(|(name, bytes)| (name.as_str(), Serve::Bytes(bytes.clone())))).await;
    let base = hub.hub.base.clone();
    app.schedule_models.set_hub(hub.hub.clone());
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "wordllama-l2-256", "source": { "kind": "download" } }))
        .await
        .unwrap();
    let revision = &catalog(app).model("wordllama-l2-256").unwrap().revision;
    assert!(preview["assets"][0]["source"]["url"]
        .as_str()
        .unwrap()
        .starts_with(&format!("{base}/dleemiller/word-llama-l2-supercat/resolve/{revision}/")));
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    let installed = call(app, "schedule_models.apply", apply).await.unwrap();
    assert_eq!(installed["state"], if cfg!(feature = "embedding-local") { "ready" } else { "runtime_unavailable" });
    let first = saved(app).unwrap();
    let weights = first.installs["wordllama-l2-256"].assets.get(AssetKind::Weights).unwrap();
    assert_eq!(std::fs::read(&weights.path).unwrap(), files[0].1);

    // The same download with a changed file fails its checksum and installs nothing new.
    let mut changed = files[1].1.clone();
    changed.push(b' ');
    hub.serve(&files[1].0, Serve::Bytes(changed));
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "wordllama-l2-256", "source": { "kind": "download" } }))
        .await
        .unwrap();
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap_err(), "asset_install_failed");
    assert_eq!(saved(app).unwrap().installs["wordllama-l2-256"], first.installs["wordllama-l2-256"], "the first install stays");
}

/// The model the catalog measured, from its real files: set `BEANS_SCHEDULE_MODEL_DIR` to a
/// directory holding `l2_supercat_256.safetensors` and `l2_supercat_tokenizer_config.json`
/// (from Hugging Face, or from the `wordllama==0.4.0.post1` wheel) and run with `--ignored`.
#[cfg(feature = "embedding-local")]
#[tokio::test]
#[ignore = "needs BEANS_SCHEDULE_MODEL_DIR with the wordllama-l2-256 files"]
async fn the_measured_model_reads_the_held_out_messages() {
    let dir = PathBuf::from(std::env::var("BEANS_SCHEDULE_MODEL_DIR").expect("BEANS_SCHEDULE_MODEL_DIR"));
    let scratch = shipped_runner();
    let app = &scratch.0;
    let files = supplied(&dir.join("l2_supercat_256.safetensors"), &dir.join("l2_supercat_tokenizer_config.json"));
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "wordllama-l2-256", "source": files })).await.unwrap();
    assert_eq!(preview["pinned"], true);
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap()["state"], "ready");
    reads_the_held_out_messages(app).await;
}

/// Selects the installed `wordllama-l2-256` and reads the held-out messages with it, as the
/// catalog measured them.
#[cfg(feature = "embedding-local")]
async fn reads_the_held_out_messages(app: &Arc<App>) {
    call(app, "schedule_models.select", json!({ "mode": "local", "model_id": "wordllama-l2-256" })).await.unwrap();
    let cancel = CancellationToken::new();
    let (mut found, mut false_alarms) = (0, 0);
    for message in crate::schedule_models::classifier::held_out_messages() {
        let (words, score) = read(app, &message.text, &cancel).await.unwrap();
        let flagged = score > LOCAL_THRESHOLD;
        println!("{} {:>5} {score:+.2} {:?} <- {}", message.language, message.recurring, words, message.text);
        match (message.recurring, flagged) {
            (true, true) => found += 1,
            (false, true) => false_alarms += 1,
            _ => {}
        }
    }
    println!("held out: {found}/20 requests found, {false_alarms}/20 false alarms");
    // The Python harness measured 17/20 and 3/20 with the same data and method.
    assert!(found >= 15, "{found}/20 found");
    assert!(false_alarms <= 4, "{false_alarms}/20 false alarms");
}

/// An ONNX catalog model through the ONNX Runtime the user supplies: installed, loaded on the
/// CPU, trained, and reading messages. Set `BEANS_SCHEDULE_ONNX_DIR` to a directory holding
/// `model.onnx` (any graph with all-MiniLM-L6-v2's tensors: int64 `input_ids`, `attention_mask`,
/// `token_type_ids` in, float `last_hidden_state` [batch, tokens, 384] out), `tokenizer.json`,
/// and the runtime (`libonnxruntime.so`, `libonnxruntime.dylib`, or `onnxruntime.dll`, 1.27 or
/// newer), and run with `--ignored`. The runtime binds once per process, so run it on its own.
#[cfg(feature = "embedding-local")]
#[tokio::test]
#[ignore = "needs BEANS_SCHEDULE_ONNX_DIR with model.onnx, tokenizer.json, and an ONNX Runtime library"]
async fn an_onnx_model_runs_on_the_supplied_runtime() {
    let dir = PathBuf::from(std::env::var("BEANS_SCHEDULE_ONNX_DIR").expect("BEANS_SCHEDULE_ONNX_DIR"));
    let runtime = crate::embeddings::assets::AssetKind::Runtime.filename();
    let scratch = runner();
    let app = &scratch.0;
    let files = json!({ "kind": "supplied", "files": { "model": dir.join("model.onnx"), "tokenizer": dir.join("tokenizer.json") } });
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "fixture-onnx", "source": files, "runtime": dir.join(runtime) }))
        .await
        .unwrap();
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap()["state"], "ready");
    assert!(saved(app).unwrap().installs["fixture-onnx"].classifier.as_ref().is_some_and(|c| c.weights.len() == 384));
    let selected = call(app, "schedule_models.select", json!({ "mode": "local_then_reader", "model_id": "fixture-onnx" })).await.unwrap();
    assert_eq!(selected["state"], "ready");
    let (words, score) = read(app, "a b, c d", &CancellationToken::new()).await.unwrap();
    assert!(score.is_finite() && ["a b, c d", "a b", "c d"].contains(&words.as_str()), "{words} {score}");
}

/// A model installed here can also embed messages for semantic search: the real static engine,
/// the approved files, and the indexer end to end. Removing the install leaves search unable to
/// load it rather than reading files nobody approved.
#[cfg(feature = "embedding-local")]
#[tokio::test]
async fn an_installed_model_embeds_messages_for_semantic_search() {
    let scratch = runner();
    let app = &scratch.0;
    let (weights, tokenizer) = supplied_files(&scratch.1);
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "fixture-static", "source": supplied(&weights, &tokenizer) })).await.unwrap();
    call(app, "schedule_models.apply", json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true }))
        .await
        .unwrap();
    let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
    // The fixture tokenizer knows `a` to `d`; each word is one row of the fixture table.
    for (id, text) in [("first", "a b"), ("second", "c d d")] {
        let mut message = crate::model::Message::new(&chat, crate::model::Author::You, crate::model::Body::text(text));
        message.id = id.into();
        app.store.upsert(&message).unwrap();
    }
    let me = me(app);
    let text = json!({ "text": { "kind": "local", "model_id": "fixture-static" } });
    let select = crate::search_index::serve_as(app, "search_index.select", text, &me).await.unwrap();
    assert_eq!(select["text"]["choice"]["model_id"], "fixture-static");
    let mut state = Value::Null;
    for _ in 0..500 {
        state = crate::search_index::serve_as(app, "search_index.status", json!({}), &me).await.unwrap()["text"]["state"].clone();
        if state != "loading" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state, "ready");
    while crate::search_index::runner::index_once(app).await.unwrap() {}
    let status = crate::search_index::serve_as(app, "search_index.status", json!({}), &me).await.unwrap();
    assert_eq!((status["text"]["indexed"].as_u64(), status["text"]["pending"].as_u64()), (Some(2), Some(0)));
    // An image model cannot read text search, nor a text model images.
    let wrong = crate::search_index::serve_as(app, "search_index.select", json!({ "image_model_id": "fixture-static" }), &me).await;
    assert_eq!(wrong.unwrap_err(), "not_an_image_model");
    // The same words are the same vector: the message is its own nearest neighbour.
    let found = crate::api::dispatch(app, "chats.search_semantic", json!({ "query": "a b" })).await.unwrap();
    assert_eq!(found["state"], "ready");
    assert_eq!(found["related"][0]["message_id"], "first");
    assert!((found["related"][0]["score"].as_f64().unwrap() - 1.0).abs() < 0.02);

    call(app, "schedule_models.remove", json!({ "model_id": "fixture-static" })).await.unwrap();
    app.search_index.clear();
    let gone = crate::api::dispatch(app, "chats.search_semantic", json!({ "query": "a b" })).await.unwrap();
    assert_ne!(gone["state"], "ready");
    assert!(gone["related"].as_array().unwrap().is_empty());
}

/// The real ONNX Runtime (`BEANS_ONNXRUNTIME`, which `scripts/test-rust.ts` fetches and pins): an
/// image model installed from supplied files through the approved preview, its vision tower
/// reading pictures and its text tower reading queries, end to end. The preview names no
/// runtime, so the shipped one is found and shown like the model's own files. The tiny models
/// (`embeddings/fixtures/make_onnx.py`) make `a` and a red picture point the same way.
#[cfg(feature = "embedding-local")]
#[tokio::test]
#[ignore = "needs BEANS_ONNXRUNTIME; scripts/test-rust.ts provides it"]
async fn onnx_runtime_finds_pictures_with_an_installed_image_model() {
    let runtime = std::env::var_os("BEANS_ONNXRUNTIME").map(PathBuf::from).expect("BEANS_ONNXRUNTIME names the ONNX Runtime library");
    assert!(runtime.is_file());
    let scratch = shipped_runner();
    installs_and_finds_pictures(&scratch.0, "fixture-clip", "supplied").await;
}

/// As above on a Runner with no runtime beside it (a CLI installed on its own): the preview
/// brings the runtime as the pinned wheel, here the real library zipped and served by a mock
/// hub, and the library unpacked from it runs the model.
#[cfg(feature = "embedding-local")]
#[tokio::test]
#[ignore = "needs BEANS_ONNXRUNTIME; scripts/test-rust.ts provides it"]
async fn onnx_runtime_downloads_its_runtime_when_none_ships_beside_the_cli() {
    let runtime = std::env::var_os("BEANS_ONNXRUNTIME").map(PathBuf::from).expect("BEANS_ONNXRUNTIME names the ONNX Runtime library");
    let library = std::fs::read(&runtime).unwrap();
    let scratch = shipped_runner();
    let app = &scratch.0;
    let (hub, pin) = served_runtime(&library, "onnxruntime/capi/libonnxruntime.so").await;
    app.schedule_models.set_hub(hub.hub.clone());
    app.schedule_models.set_runtime_supply(RuntimeSupply { beside: None, download: Some(pin) });
    installs_and_finds_pictures(app, "fixture-clip", "archive").await;
    assert_eq!(hub.requests().iter().filter(|r| r.starts_with("origin ")).count(), 1, "the wheel, once");
    // A second model, with its own copy of the same runtime, loads beside the first in this
    // process: one runtime, whichever copy loaded it.
    app.schedule_models.set_runtime_supply(RuntimeSupply { beside: Some(runtime), download: None });
    installs_and_finds_pictures(app, "fixture-clip-again", "supplied").await;
}

/// Installs a tiny image model from supplied files, its runtime shown in the preview as coming
/// from `runtime_source`, then finds pictures by colour with it.
#[cfg(feature = "embedding-local")]
async fn installs_and_finds_pictures(app: &Arc<App>, id: &str, runtime_source: &str) {
    let mut clip = super::super::catalog().models.iter().find(|m| m.engine == Engine::Clip).unwrap().clone();
    clip.id = "fixture-clip".into();
    for file in [clip.files.model.as_mut(), clip.files.image_model.as_mut(), Some(&mut clip.files.tokenizer)].into_iter().flatten() {
        (file.bytes, file.sha256) = (None, None);
    }
    clip.approx_bytes = Some(10_000);
    let text = clip.onnx.as_mut().unwrap();
    (text.dimensions, text.max_tokens, text.add_special_tokens, text.pad_id, text.pad_token) = (4, 8, false, 1, "[PAD]".into());
    let image = clip.image.as_mut().unwrap();
    (image.size, image.mean, image.std) = (16, [0.5; 3], [0.5; 3]);
    let again = CatalogModel { id: "fixture-clip-again".into(), ..clip.clone() };
    app.schedule_models.set_catalog(Catalog { version: 1, note: "fixtures".into(), models: vec![clip, again] });

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/embeddings/fixtures");
    let files = json!({
        "model": fixtures.join("tiny-text.onnx"), "image_model": fixtures.join("tiny-vision.onnx"), "tokenizer": fixtures.join("tiny-tokenizer.json"),
    });
    let preview = call(app, "schedule_models.preview", json!({ "model_id": id, "source": { "kind": "supplied", "files": files } })).await.unwrap();
    let shown: Vec<&str> = preview["assets"].as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap()).collect();
    assert_eq!(shown, ["model", "image_model", "tokenizer", "runtime"], "the runtime is shown for approval");
    assert_eq!(preview["assets"][3]["source"]["kind"], runtime_source);
    let installed = call(app, "schedule_models.apply", json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true })).await.unwrap();
    assert_eq!(installed["state"], "installed", "an image model reads no schedules");

    let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
    std::fs::create_dir_all(app.config.files_dir()).unwrap();
    for (id, colour) in [("red", [240, 0, 0]), ("blue", [0, 0, 240])] {
        let png = {
            let image = image::RgbImage::from_pixel(8, 8, image::Rgb(colour));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            bytes.into_inner()
        };
        let attachment = crate::model::Attachment { id: format!("att-{id}"), name: format!("{id}.png"), mime: "image/png".into(), size: png.len() as u64, width: Some(8), height: Some(8) };
        std::fs::write(crate::files::local_path(app, &attachment.id).unwrap(), png).unwrap();
        let body = crate::model::Body::Text { text: "a picture".into(), attachments: vec![attachment], mentions: vec![], reply_to: None };
        let mut message = crate::model::Message::new(&chat, crate::model::Author::You, body);
        message.id = id.into();
        app.store.upsert(&message).unwrap();
    }
    let me = me(app);
    crate::search_index::serve_as(app, "search_index.select", json!({ "image_model_id": id }), &me).await.unwrap();
    let mut status = Value::Null;
    for _ in 0..500 {
        status = crate::search_index::serve_as(app, "search_index.status", json!({}), &me).await.unwrap();
        if status["image"]["state"] != "loading" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(status["image"]["state"], "ready", "{status}");
    while crate::search_index::runner::index_once(app).await.unwrap() {}
    for (query, first) in [("a", "red"), ("c", "blue")] {
        let found = crate::api::dispatch(app, "chats.search_semantic", json!({ "query": query })).await.unwrap();
        assert_eq!(found["image_state"], "ready");
        assert_eq!(found["images"][0]["message_id"], first, "{query}: {found}");
        assert!((found["images"][0]["score"].as_f64().unwrap() - 1.0).abs() < 0.05, "the same direction: {found}");
    }
}

/// A roomy computer that cannot run ONNX models, as the suggestion reads it.
fn static_only_machine() -> super::super::recommend::Machine {
    super::super::recommend::Machine {
        memory_bytes: Some(8 << 30),
        free_disk_bytes: Some(100 << 30),
        cores: 8,
        os: "linux".into(),
        arch: "x86_64".into(),
        onnx_runtime: false,
        runtime_download_bytes: 0,
    }
}

/// The pinned fixture model's files, by the names the hub serves them under.
fn hub_files() -> Vec<(String, Vec<u8>)> {
    let model = catalog_fixture_pinned();
    let (weights, tokenizer) = fixture_bytes();
    vec![(file_name(&model, "weights"), weights), (file_name(&model, "tokenizer"), tokenizer)]
}

fn catalog_fixture_pinned() -> CatalogModel {
    fixture_catalog().model("fixture-static-pinned").unwrap().clone()
}

fn file_name(model: &CatalogModel, role: &str) -> String {
    let (_, file) = model.files().into_iter().find(|(r, _)| *r == role).unwrap();
    file.path.rsplit('/').next().unwrap().to_string()
}

async fn approve(app: &Arc<App>, model_id: &str) -> Result<Value, String> {
    let preview = call(app, "schedule_models.preview", json!({ "model_id": model_id, "source": { "kind": "download" } })).await?;
    call(app, "schedule_models.apply", json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true })).await
}

/// The whole process against a mock hub: the list suggests what fits this computer, nothing
/// downloads until the user approves a preview, the approved files download through the hub's
/// redirect, and the installed model then searches messages.
#[tokio::test]
async fn a_suggested_model_downloads_only_once_approved_and_then_searches() {
    let scratch = runner();
    let app = &scratch.0;
    app.schedule_models.set_machine(static_only_machine());
    let hub = MockHub::start(hub_files().iter().map(|(name, bytes)| (name.as_str(), Serve::Bytes(bytes.clone())))).await;
    app.schedule_models.set_hub(hub.hub.clone());

    let list = call(app, "schedule_models.list", json!({})).await.unwrap();
    assert_eq!(list["recommended"]["text"], "fixture-static-pinned", "the one model this computer can run and get");
    assert_eq!(list["recommended"]["image"], Value::Null);
    assert_eq!(list["machine"]["onnx_runtime"], false);
    let fit = |id: &str| list["models"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap()["fit"].clone();
    assert_eq!(fit("fixture-static")["reasons"], json!(["not_downloadable"]));
    assert_eq!(fit("fixture-onnx")["fits"], false);
    assert!(fit("fixture-onnx")["reasons"].as_array().unwrap().contains(&json!("needs_onnx_runtime")));
    let url = list["models"][1]["downloads"][0]["url"].as_str().unwrap();
    assert!(url.starts_with(&hub.hub.base), "listings name the hub downloads come from: {url}");

    let preview = call(app, "schedule_models.preview", json!({ "model_id": "fixture-static-pinned", "source": { "kind": "download" } })).await.unwrap();
    assert!(hub.requests().is_empty(), "listing and previewing download nothing: {:?}", hub.requests());
    assert_eq!(preview["hub"], "127.0.0.1", "the approval names the hub in use");
    assert_eq!(preview["total_bytes"], hub_files().iter().map(|(_, b)| b.len()).sum::<usize>());
    let refused = call(app, "schedule_models.apply", json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": false })).await;
    assert_eq!(refused.unwrap_err(), "confirmation_required");
    assert!(hub.requests().is_empty(), "a refusal downloads nothing");

    approve(app, "fixture-static-pinned").await.unwrap();
    let requested = hub.requests();
    assert_eq!(requested.iter().filter(|r| r.starts_with("origin ")).count(), 2, "each approved file once: {requested:?}");
    assert_eq!(requested.iter().filter(|r| r.starts_with("cdn ")).count(), 2, "through the CDN redirect: {requested:?}");
    let list = call(app, "schedule_models.list", json!({})).await.unwrap();
    let installed = list["models"].as_array().unwrap().iter().find(|m| m["id"] == "fixture-static-pinned").unwrap().clone();
    assert_eq!(installed["installed"], true);
    assert!(installed["fit"]["reasons"].as_array().unwrap().contains(&json!("installed")));

    #[cfg(feature = "embedding-local")]
    {
        let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
        let mut message = crate::model::Message::new(&chat, crate::model::Author::You, crate::model::Body::text("a b"));
        message.id = "found".into();
        app.store.upsert(&message).unwrap();
        let me = me(app);
        crate::search_index::serve_as(app, "search_index.select", json!({ "text": { "kind": "local", "model_id": "fixture-static-pinned" } }), &me).await.unwrap();
        for _ in 0..500 {
            if crate::search_index::serve_as(app, "search_index.status", json!({}), &me).await.unwrap()["text"]["state"] == "ready" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while crate::search_index::runner::index_once(app).await.unwrap() {}
        let found = crate::api::dispatch(app, "chats.search_semantic", json!({ "query": "a b" })).await.unwrap();
        assert_eq!(found["related"][0]["message_id"], "found");
    }
}

/// Every way a hub download goes wrong installs nothing, keeps the install already there, spends
/// the approval, and leaves no partial files; serving the right bytes again recovers.
#[tokio::test]
async fn a_download_that_goes_wrong_installs_nothing_and_keeps_what_was_there() {
    let scratch = runner();
    let app = &scratch.0;
    let files = hub_files();
    let hub = MockHub::start(files.iter().map(|(name, bytes)| (name.as_str(), Serve::Bytes(bytes.clone())))).await;
    app.schedule_models.set_hub(hub.hub.clone());
    approve(app, "fixture-static-pinned").await.unwrap();
    let first = saved(app).unwrap().installs["fixture-static-pinned"].clone();
    let directory = first.assets.assets[0].1.path.parent().unwrap().parent().unwrap().to_path_buf();
    let entries = || std::fs::read_dir(&directory).unwrap().count();
    let before = entries();

    let (name, bytes) = files[0].clone();
    let mut wrong = bytes.clone();
    wrong[0] ^= 0xff;
    let mut longer = bytes.clone();
    longer.extend_from_slice(b"more");
    for (case, serve) in [
        ("not found", Serve::Status(404)),
        ("server error", Serve::Status(500)),
        ("rate limited", Serve::Status(429)),
        ("cut short", Serve::Truncated(bytes.clone(), bytes.len() / 2)),
        ("other bytes of the same size", Serve::Bytes(wrong)),
        ("more bytes than pinned", Serve::Bytes(longer)),
        ("empty", Serve::Bytes(Vec::new())),
        ("a redirect to a host the hub does not allow", Serve::ForeignRedirect),
        ("a redirect loop", Serve::RedirectLoop),
    ] {
        hub.serve(&name, serve);
        let preview = call(app, "schedule_models.preview", json!({ "model_id": "fixture-static-pinned", "source": { "kind": "download" } })).await.unwrap();
        let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
        assert_eq!(call(app, "schedule_models.apply", apply.clone()).await.unwrap_err(), "asset_install_failed", "{case}");
        assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap_err(), "approval_not_found", "{case}: the approval is spent");
        assert_eq!(saved(app).unwrap().installs["fixture-static-pinned"], first, "{case}: the install already there stays");
        assert!(first.assets.assets.iter().all(|(_, asset)| asset.path.is_file()), "{case}: its files too");
        assert_eq!(entries(), before, "{case}: nothing partial is left behind");
    }
    hub.serve(&name, Serve::Bytes(bytes));
    approve(app, "fixture-static-pinned").await.unwrap();
    assert_ne!(saved(app).unwrap().installs["fixture-static-pinned"].installed_at, first.installed_at, "the right bytes install again");
}

/// A hub that cannot be reached fails the install the same way, and never blocks a preview.
#[tokio::test]
async fn an_unreachable_hub_fails_the_install_cleanly() {
    let scratch = runner();
    let app = &scratch.0;
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    app.schedule_models.set_hub(super::super::hub::Hub::parse(Some(&format!("http://127.0.0.1:{port}"))).unwrap());
    assert_eq!(approve(app, "fixture-static-pinned").await.unwrap_err(), "asset_install_failed");
    assert!(saved(app).unwrap().installs.is_empty());
}

/// State an earlier release saved, before settings files were shared (`runner_state`), reads the
/// same: the account inside, beside the mode, selection, and installs.
#[tokio::test]
async fn state_saved_by_an_earlier_release_still_reads() {
    let scratch = runner();
    let app = &scratch.0;
    let path = app.config.home.join(Saved::PATH);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let earlier = json!({ "account": account(app).unwrap(), "mode": "local_then_reader", "selected": null, "installs": {} });
    std::fs::write(&path, serde_json::to_vec(&earlier).unwrap()).unwrap();
    app.schedule_models.clear();
    assert_eq!(mode(app), Detection::LocalThenReader);
    assert_eq!(call(app, "schedule_models.status", json!({})).await.unwrap()["mode"], "local_then_reader");
}

/// A wheel holding `library` at `member`, served by a new mock hub, and its pin.
async fn served_runtime(library: &[u8], member: &str) -> (MockHub, Pinned) {
    let wheel = crate::embeddings::archive::tests::zip(&[("onnxruntime/__init__.py", b"# python"), (member, library)], None);
    let hub = MockHub::start([("onnxruntime-test.whl", Serve::Bytes(wheel.clone()))]).await;
    let pin = Pinned {
        url: format!("{}/packages/onnxruntime-test.whl", hub.hub.base),
        bytes: wheel.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&wheel)),
        member: member.into(),
        library_bytes: library.len() as u64,
        library_sha256: format!("{:x}", Sha256::digest(library)),
    };
    (hub, pin)
}

/// A Runner with no runtime beside it (a CLI installed on its own) downloads the pinned one with
/// an ONNX model: shown in the preview, fetched only once approved, checked as a whole wheel and
/// as the library in it, which alone is kept. A runtime beside the binary is used instead.
#[tokio::test]
async fn an_onnx_model_brings_its_runtime_when_none_is_here() {
    let scratch = runner();
    let app = &scratch.0;
    let (weights, tokenizer) = supplied_files(&scratch.1);
    let library = b"\x7fELF not really a runtime, ".repeat(200);
    let member = "onnxruntime/capi/libonnxruntime.so";
    let (hub, pin) = served_runtime(&library, member).await;
    app.schedule_models.set_hub(hub.hub.clone());
    app.schedule_models.set_runtime_supply(RuntimeSupply { beside: None, download: Some(pin.clone()) });
    let onnx = json!({ "model_id": "fixture-onnx", "source": { "kind": "supplied", "files": { "model": weights, "tokenizer": tokenizer } } });

    let preview = call(app, "schedule_models.preview", onnx.clone()).await.unwrap();
    let runtime = preview["assets"].as_array().unwrap().iter().find(|a| a["kind"] == "runtime").unwrap().clone();
    assert_eq!(runtime["source"], json!({ "kind": "archive", "url": pin.url, "bytes": pin.bytes, "sha256": pin.sha256, "member": member }));
    assert_eq!((runtime["bytes"].as_u64(), runtime["sha256"].as_str()), (Some(pin.library_bytes), Some(pin.library_sha256.as_str())), "the library is what installs");
    assert_eq!(preview["runtime_download"], json!({ "version": runtime_download::version(), "bytes": pin.bytes, "from": "127.0.0.1" }));
    assert!(hub.requests().is_empty(), "a preview downloads nothing");
    call(app, "schedule_models.apply", json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true })).await.unwrap();
    let install = saved(app).unwrap().installs["fixture-onnx"].clone();
    let installed = install.assets.get(AssetKind::Runtime).unwrap();
    assert_eq!(std::fs::read(&installed.path).unwrap(), library);
    let mut kept: Vec<String> = std::fs::read_dir(installed.path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    kept.sort();
    let mut expected = [AssetKind::Runtime.filename(), "model.onnx", "tokenizer.json"];
    expected.sort();
    assert_eq!(kept, expected, "only the library is kept, never the wheel");

    // A wheel that is not the pinned one, or a pinned wheel without the library, installs nothing.
    let wheel = crate::embeddings::archive::tests::zip(&[("onnxruntime/__init__.py", b"# python"), (member, &library)], None);
    let mut changed = wheel.clone();
    let last = changed.len() - 30;
    changed[last] ^= 0xff;
    for (case, serve) in [
        ("another wheel of the same size", Serve::Bytes(changed)),
        ("cut short", Serve::Truncated(wheel.clone(), wheel.len() / 2)),
        ("not found", Serve::Status(404)),
    ] {
        hub.serve("onnxruntime-test.whl", serve);
        let preview = call(app, "schedule_models.preview", onnx.clone()).await.unwrap();
        let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
        assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap_err(), "asset_install_failed", "{case}");
        assert_eq!(saved(app).unwrap().installs["fixture-onnx"], install, "{case}: the install already there stays");
    }
    let (empty_hub, mut without) = served_runtime(b"a different file", "onnxruntime/capi/other.so").await;
    without.member = member.into();
    app.schedule_models.set_hub(empty_hub.hub.clone());
    app.schedule_models.set_runtime_supply(RuntimeSupply { beside: None, download: Some(without) });
    let preview = call(app, "schedule_models.preview", onnx.clone()).await.unwrap();
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap_err(), "asset_install_failed", "the pinned wheel without the library");

    // A runtime beside the binary is used, and nothing is downloaded for it.
    let beside = scratch.1.join("beside-libonnxruntime");
    std::fs::write(&beside, &library).unwrap();
    app.schedule_models.set_runtime_supply(RuntimeSupply { beside: Some(beside.clone()), download: Some(pin) });
    let preview = call(app, "schedule_models.preview", onnx).await.unwrap();
    let runtime = preview["assets"].as_array().unwrap().iter().find(|a| a["kind"] == "runtime").unwrap().clone();
    assert_eq!(runtime["source"], json!({ "kind": "supplied", "path": beside }));
    assert!(preview["runtime_download"].is_null());
}

/// The size probe the live checks use, against the mock hub: through the CDN redirect, by the
/// whole size a first-byte answer gives; a missing file or a redirect away is an error.
#[tokio::test]
async fn a_pinned_file_is_sized_on_the_hub_without_downloading_it() {
    let body = b"0123456789".repeat(100);
    let hub = MockHub::start([
        ("file.bin", Serve::Bytes(body)),
        ("empty.bin", Serve::Bytes(Vec::new())),
        ("gone.bin", Serve::Status(404)),
        ("away.bin", Serve::ForeignRedirect),
    ])
    .await;
    let url = |name: &str| format!("{}/owner/repo/resolve/main/{name}", hub.hub.base);
    assert_eq!(hub.hub.served_size(&url("file.bin")).await, Ok(1000));
    assert_eq!(hub.hub.served_size(&url("empty.bin")).await, Ok(0), "nothing to range over: the whole empty answer");
    assert_eq!(hub.hub.served_size(&url("gone.bin")).await, Err("HTTP 404".into()));
    assert_eq!(hub.hub.served_size(&url("away.bin")).await, Err("HTTP 302".into()), "a redirect away is not followed");
}

/// The live hub, where it is reachable (`pnpm run test:rust --live-hub`): every pinned file of
/// the shipped catalog, and every pinned ONNX Runtime wheel, served at its pinned size. A
/// changed or missing file means the catalog needs pinning again (`pnpm run models:pin`).
#[tokio::test]
#[ignore = "reaches the model hub (Hugging Face unless BEANS_MODEL_HUB) and PyPI: pnpm run test:rust --live-hub"]
async fn live_hub_serves_every_pinned_file_at_its_pinned_size() {
    let hub = Hub::from_env();
    let (mut checked, mut wrong) = (0, Vec::new());
    let mut expect = |what: String, served: Result<u64, String>, pinned: u64| match served {
        Ok(size) if size == pinned => checked += 1,
        other => wrong.push(format!("{what}: {other:?}, pinned {pinned}")),
    };
    for model in super::super::catalog().models.iter().filter(|m| m.pinned()) {
        for (role, file) in model.files() {
            let url = hub.file_url(model, file);
            expect(format!("{} {role} {url}", model.id), hub.served_size(&url).await, file.bytes.unwrap());
        }
    }
    for target in ["darwin-arm64", "linux-arm64", "linux-x64", "windows-arm64", "windows-x64"] {
        let pin = runtime_download::pinned_for(target).unwrap();
        expect(format!("runtime {target} {}", pin.url), Hub::hugging_face().served_size(&pin.url).await, pin.bytes);
    }
    assert!(wrong.is_empty(), "not as pinned:\n{}", wrong.join("\n"));
    assert!(checked > 5, "{checked} files checked");
}

/// The live hub end to end: the measured model downloads through the real hub with the user's
/// approval, checked against its pins, and then reads schedules as the catalog measured.
#[cfg(feature = "embedding-local")]
#[tokio::test]
#[ignore = "reaches the model hub (Hugging Face unless BEANS_MODEL_HUB): pnpm run test:rust --live-hub"]
async fn live_hub_installs_the_measured_model_and_it_reads_the_held_out_messages() {
    let scratch = shipped_runner();
    let app = &scratch.0;
    let installed = approve(app, "wordllama-l2-256").await.unwrap();
    assert_eq!(installed["state"], "ready", "{installed}");
    reads_the_held_out_messages(app).await;
}

/// Issue #581: an install's size bounds it, not a fixed five minutes.
#[test]
fn an_install_may_take_as_long_as_its_size_needs() {
    use crate::embeddings::assets::transfer_cap;
    assert!(transfer_cap(0) >= Duration::from_secs(600));
    // 1 GB at 1 MB/s takes about 17 minutes; the cap leaves room for much slower links.
    assert!(transfer_cap(1_000_000_000) > Duration::from_secs(2 * 3600), "{:?}", transfer_cap(1_000_000_000));
}

/// Issue #581: a slow download reports its bytes as they arrive and finishes however long it
/// takes, as long as bytes keep coming; a cancel stops it and installs nothing, and the next
/// approved download installs.
#[tokio::test]
async fn a_slow_download_shows_progress_and_a_cancel_installs_nothing() {
    let scratch = runner();
    let app = &scratch.0;
    let files = hub_files();
    let total: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();
    let slow = |bytes: &Vec<u8>| Serve::Slow(bytes.clone(), 512, Duration::from_millis(150));
    let hub = MockHub::start(files.iter().map(|(name, bytes)| (name.as_str(), slow(bytes)))).await;
    app.schedule_models.set_hub(hub.hub.clone());
    assert_eq!(call(app, "schedule_models.progress", json!({})).await.unwrap(), json!({ "installing": null }));

    let installing = tokio::spawn({
        let app = app.clone();
        async move { approve(&app, "fixture-static-pinned").await }
    });
    let mut seen = Value::Null;
    for _ in 0..400 {
        seen = call(app, "schedule_models.progress", json!({})).await.unwrap();
        if seen["installing"]["done_bytes"].as_u64().is_some_and(|done| done > 0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(seen["installing"]["model_id"], "fixture-static-pinned", "{seen}");
    assert_eq!(seen["installing"]["total_bytes"], total);
    assert!(seen["installing"]["done_bytes"].as_u64().unwrap() < total as u64, "{seen}");
    assert_eq!(call(app, "schedule_models.cancel", json!({ "model_id": "other" })).await.unwrap(), json!({ "cancelled": false }));
    assert_eq!(call(app, "schedule_models.cancel", json!({})).await.unwrap(), json!({ "cancelled": true }));
    assert_eq!(installing.await.unwrap().unwrap_err(), "install_cancelled");
    assert_eq!(call(app, "schedule_models.progress", json!({})).await.unwrap(), json!({ "installing": null }));
    assert_eq!(call(app, "schedule_models.status", json!({})).await.unwrap()["installed"], json!([]));
    fn staging(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir).map(|entries| entries.flatten().flat_map(|entry| {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with(".staging-") { vec![path] } else if path.is_dir() { staging(&path) } else { vec![] }
        }).collect()).unwrap_or_default()
    }
    assert_eq!(staging(&app.config.home.join(ASSET_DIR)), Vec::<PathBuf>::new(), "nothing partial is left");

    // Slower than the idle timeout in all, but never idle that long: it finishes.
    let started = std::time::Instant::now();
    approve(app, "fixture-static-pinned").await.unwrap();
    assert!(started.elapsed() > crate::embeddings::assets::IDLE_TIMEOUT, "the download took {:?}", started.elapsed());
    assert_eq!(call(app, "schedule_models.progress", json!({})).await.unwrap(), json!({ "installing": null }));
}

/// Issue #581: a download that stops sending stops after the idle timeout, installing nothing.
#[tokio::test]
async fn a_stalled_download_stops_after_the_idle_timeout() {
    let scratch = runner();
    let app = &scratch.0;
    let files = hub_files();
    let hub = MockHub::start(files.iter().map(|(name, bytes)| (name.as_str(), Serve::Stall(bytes.clone(), 100)))).await;
    app.schedule_models.set_hub(hub.hub.clone());
    let started = std::time::Instant::now();
    assert_eq!(approve(app, "fixture-static-pinned").await.unwrap_err(), "asset_install_failed");
    assert!(started.elapsed() < crate::embeddings::assets::IDLE_TIMEOUT * 5, "{:?}", started.elapsed());
    assert_eq!(call(app, "schedule_models.status", json!({})).await.unwrap()["installed"], json!([]));
}
