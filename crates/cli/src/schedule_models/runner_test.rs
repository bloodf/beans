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
    assert_eq!((preview["pinned"].as_bool(), preview["source"].as_str()), (Some(true), Some("download")));
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
    // An ONNX model needs the ONNX Runtime the user supplies.
    let onnx = json!({ "kind": "supplied", "files": { "model": weights, "tokenizer": tokenizer } });
    assert_eq!(preview("fixture-onnx", onnx.clone(), None).await.unwrap_err(), "runtime_required");
    let with_runtime = preview("fixture-onnx", onnx, Some(&weights)).await.unwrap();
    let kinds: Vec<&str> = with_runtime["assets"].as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["model", "tokenizer", "runtime"]);

    let preview = preview("fixture-static", supplied(&weights, &tokenizer), None).await.unwrap();
    assert_eq!((preview["pinned"].as_bool(), preview["source"].as_str()), (Some(false), Some("supplied")));
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
        assert_eq!(std::fs::metadata(state_path(app)).unwrap().permissions().mode() & 0o777, 0o600);
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
    let mut other = saved(app).unwrap();
    other.account = "someone else".into();
    crate::config::write_json_private(&state_path(app), &other).unwrap();
    app.schedule_models.clear();
    assert_eq!(mode(app), Detection::Reader);
}

/// Serves `files` by name over plain HTTP until dropped: the origin answers each path with a
/// redirect to the second listener, as Hugging Face sends downloads on to its CDN.
async fn hugging_face_and_cdn(files: Vec<(String, Vec<u8>)>) -> (String, u16, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cdn = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (origin_port, cdn_port) = (origin.local_addr().unwrap().port(), cdn.local_addr().unwrap().port());
    let files = Arc::new(files);
    let task = tokio::spawn(async move {
        let request_path = |buffer: &[u8]| String::from_utf8_lossy(buffer).split_whitespace().nth(1).unwrap_or("").to_string();
        loop {
            tokio::select! {
                Ok((mut stream, _)) = origin.accept() => {
                    let mut buffer = vec![0u8; 8192];
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    let path = request_path(&buffer[..n]);
                    let name = path.rsplit('/').next().unwrap_or("").to_string();
                    let reply = format!("HTTP/1.1 302 Found\r\nlocation: http://localhost:{cdn_port}/blobs/{name}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                    let _ = stream.write_all(reply.as_bytes()).await;
                }
                Ok((mut stream, _)) = cdn.accept() => {
                    let files = files.clone();
                    tokio::spawn(async move {
                        let mut buffer = vec![0u8; 8192];
                        let n = stream.read(&mut buffer).await.unwrap_or(0);
                        let path = request_path(&buffer[..n]);
                        let body = files.iter().find(|(name, _)| path == format!("/blobs/{name}")).map(|(_, b)| b.clone());
                        let reply = match body {
                            Some(body) => [format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).into_bytes(), body].concat(),
                            None => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                        };
                        let _ = stream.write_all(&reply).await;
                    });
                }
            }
        }
    });
    (format!("http://127.0.0.1:{origin_port}"), cdn_port, task)
}

/// The measured model's real files download as the catalog pins them, through a redirect to
/// a CDN host, exactly as from Hugging Face; a file that is not the pinned one installs nothing.
/// Set `BEANS_SCHEDULE_MODEL_DIR` as for the test below and run with `--ignored`.
#[tokio::test]
#[ignore = "needs BEANS_SCHEDULE_MODEL_DIR with the wordllama-l2-256 files"]
async fn the_measured_model_downloads_through_a_redirect() {
    let dir = PathBuf::from(std::env::var("BEANS_SCHEDULE_MODEL_DIR").expect("BEANS_SCHEDULE_MODEL_DIR"));
    let read_file = |name: &str| (name.to_string(), std::fs::read(dir.join(name)).unwrap());
    let files = vec![read_file("l2_supercat_256.safetensors"), read_file("l2_supercat_tokenizer_config.json")];
    let scratch = shipped_runner();
    let app = &scratch.0;
    let (base, _, server) = hugging_face_and_cdn(files.clone()).await;
    let redirects = RedirectHosts {
        exact: ["localhost".to_string()].into(),
        parents: Default::default(),
    };
    app.schedule_models.set_policy(&base, true, redirects.clone());
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
    server.abort();

    // The same download with a changed file fails its checksum and installs nothing new.
    let mut changed = files.clone();
    changed[1].1.push(b' ');
    let (base, _, server) = hugging_face_and_cdn(changed).await;
    app.schedule_models.set_policy(&base, true, redirects);
    let preview = call(app, "schedule_models.preview", json!({ "model_id": "wordllama-l2-256", "source": { "kind": "download" } }))
        .await
        .unwrap();
    let apply = json!({ "preview_token": preview["preview_token"], "preview_digest": preview["preview_digest"], "confirm": true });
    assert_eq!(call(app, "schedule_models.apply", apply).await.unwrap_err(), "asset_install_failed");
    assert_eq!(saved(app).unwrap().installs["wordllama-l2-256"], first.installs["wordllama-l2-256"], "the first install stays");
    server.abort();
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
