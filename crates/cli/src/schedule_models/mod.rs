//! Local models that notice a request to repeat something without spending the user's tokens.
//!
//! The user picks one from a fixed catalog (`catalog.json`): small embedding models, at most
//! 100 MB each, that run on one CPU thread. A Runner installs one through the same one-use,
//! consent-bound preview and apply as local memory embeddings (`crate::embeddings::assets`),
//! either downloaded from Hugging Face (only for files whose size and SHA-256 the catalog pins)
//! or from files the user downloaded themselves, which are hashed locally and shown before
//! approval. Once installed, the Runner trains a small classifier for that model from
//! `training.json` and keeps it.
//!
//! How a Runner reads messages for a schedule is its `Detection` mode: the small model of the
//! bot's provider as before (`reader`, the default), the local model alone (`local`, no
//! tokens), the local model first and the small model only for what it flags
//! (`local_then_reader`), or nothing (`off`).
//!
//! The API (`schedule_models.*`) is for the user's apps; no model tool reaches it.

pub mod classifier;
#[cfg(feature = "embedding-local")]
pub mod engine;
#[cfg(feature = "runner")]
pub mod runner;

use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::embeddings::{Pooling, TensorNames};

/// The catalog the Runner offers, built in.
pub const CATALOG_JSON: &str = include_str!("catalog.json");
/// The most a model may be, its tokenizer included, in bytes.
pub const MAX_MODEL_BYTES: u64 = 100_000_000;
/// Hugging Face, where catalog files download from.
pub const HUGGING_FACE: &str = "https://huggingface.co";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub note: String,
    pub models: Vec<CatalogModel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// A token table averaged per text: no native runtime.
    Static,
    /// A transformer through ONNX Runtime, which the user supplies.
    Onnx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogFile {
    /// The file's path in the repository.
    pub path: String,
    /// Pinned size and SHA-256, or both null until pinned.
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticSpec {
    pub tensor: String,
    pub dimensions: usize,
    pub add_special_tokens: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnnxSpec {
    pub dimensions: usize,
    pub max_tokens: usize,
    pub pooling: Pooling,
    pub prefix: String,
    pub tensors: TensorNames,
    pub add_special_tokens: bool,
    pub pad_id: u32,
    pub pad_type_id: u32,
    pub pad_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogFiles {
    /// A static model's table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<CatalogFile>,
    /// An ONNX model's graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<CatalogFile>,
    pub tokenizer: CatalogFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub languages: String,
    pub license: String,
    /// The Hugging Face repository, `owner/name`.
    pub repo: String,
    /// The branch, tag, or commit files download from.
    pub revision: String,
    pub files: CatalogFiles,
    /// Roughly how big, for a model not pinned yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approx_bytes: Option<u64>,
    #[serde(default, rename = "static", skip_serializing_if = "Option::is_none")]
    pub static_spec: Option<StaticSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onnx: Option<OnnxSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_from: Option<String>,
    /// Other ways to get the same files.
    #[serde(default)]
    pub other_sources: Vec<String>,
    /// How it did on `held_out.json`, when measured.
    pub measured: Option<String>,
}

impl CatalogModel {
    /// The model's files by role: `weights` or `model`, then `tokenizer`.
    pub fn files(&self) -> Vec<(&'static str, &CatalogFile)> {
        let graph = match self.engine {
            Engine::Static => self.files.weights.as_ref().map(|f| ("weights", f)),
            Engine::Onnx => self.files.model.as_ref().map(|f| ("model", f)),
        };
        graph.into_iter().chain([("tokenizer", &self.files.tokenizer)]).collect()
    }

    /// Where a file downloads from.
    pub fn url(&self, file: &CatalogFile) -> String {
        format!("{HUGGING_FACE}/{}/resolve/{}/{}", self.repo, self.revision, file.path)
    }

    /// Whether every file's size and SHA-256 are pinned, so it may download.
    pub fn pinned(&self) -> bool {
        self.files().iter().all(|(_, f)| f.bytes.is_some() && f.sha256.is_some())
    }

    /// The pinned total size, or the approximate one.
    pub fn size(&self) -> u64 {
        if self.pinned() {
            self.files().iter().filter_map(|(_, f)| f.bytes).sum()
        } else {
            self.approx_bytes.unwrap_or(u64::MAX)
        }
    }

    pub fn dimensions(&self) -> usize {
        self.static_spec.as_ref().map(|s| s.dimensions).or(self.onnx.as_ref().map(|o| o.dimensions)).unwrap_or(0)
    }

    /// The catalog entry as the apps show it: what it is, its state, and how to get its files.
    pub fn listing(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        value["pinned"] = self.pinned().into();
        value["bytes"] = self.size().into();
        value["needs_onnx_runtime"] = (self.engine == Engine::Onnx).into();
        value["downloads"] = self
            .files()
            .iter()
            .map(|(role, file)| serde_json::json!({ "role": role, "url": self.url(file), "path": file.path, "bytes": file.bytes, "sha256": file.sha256 }))
            .collect::<Vec<_>>()
            .into();
        value
    }

    fn validate(&self) -> Result<(), String> {
        let slug = |text: &str| !text.is_empty() && text.len() <= 96 && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.'));
        let segment = |text: &str| !text.is_empty() && text.len() <= 96 && text != "." && text != ".." && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        let id = &self.id;
        if !slug(id) {
            return Err(format!("{id}: the id is lowercase letters, digits, dashes, and dots"));
        }
        if self.name.trim().is_empty() || self.languages.trim().is_empty() || self.license.trim().is_empty() {
            return Err(format!("{id}: a model needs a name, its languages, and its license"));
        }
        let repo: Vec<&str> = self.repo.split('/').collect();
        if repo.len() != 2 || !repo.iter().all(|part| segment(part)) || !segment(&self.revision) {
            return Err(format!("{id}: the repository is owner/name and the revision one segment"));
        }
        match (self.engine, &self.static_spec, &self.onnx, &self.files.weights, &self.files.model) {
            (Engine::Static, Some(spec), None, Some(_), None) if !spec.tensor.is_empty() => {}
            (Engine::Onnx, None, Some(spec), None, Some(_)) if spec.max_tokens > 0 && spec.max_tokens <= 512 => {}
            _ => return Err(format!("{id}: a static model has weights and a static spec; an ONNX model has a model file and an onnx spec")),
        }
        if !(1..=4096).contains(&self.dimensions()) {
            return Err(format!("{id}: dimensions out of range"));
        }
        for (role, file) in self.files() {
            if !file.path.split('/').all(segment) {
                return Err(format!("{id}: the {role} path has an unsafe segment"));
            }
            match (file.bytes, &file.sha256) {
                (None, None) => {}
                (Some(bytes), Some(sha256)) if bytes > 0 && sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)) => {}
                _ => return Err(format!("{id}: the {role} file pins both its size and a lowercase SHA-256, or neither")),
            }
        }
        if !self.pinned() && self.approx_bytes.is_none() {
            return Err(format!("{id}: a model not pinned yet gives its approximate size"));
        }
        if self.size() > MAX_MODEL_BYTES {
            return Err(format!("{id}: more than {MAX_MODEL_BYTES} bytes"));
        }
        Ok(())
    }
}

impl Catalog {
    fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for model in &self.models {
            model.validate()?;
            if !ids.insert(&model.id) {
                return Err(format!("{}: listed twice", model.id));
            }
        }
        Ok(())
    }

    pub fn model(&self, id: &str) -> Option<&CatalogModel> {
        self.models.iter().find(|m| m.id == id)
    }
}

/// The catalog, read once. The tests hold `catalog.json` to `Catalog::validate`.
pub fn catalog() -> &'static Catalog {
    static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
        let catalog: Catalog = serde_json::from_str(CATALOG_JSON).expect("catalog.json parses");
        catalog.validate().expect("catalog.json is valid");
        catalog
    });
    &CATALOG
}

/// How a Runner reads a user's message for a request to repeat something.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detection {
    /// Not at all; the bot still reads the message itself.
    Off,
    /// The small model of the bot's provider, which costs tokens.
    #[default]
    Reader,
    /// The selected local model alone: no tokens.
    Local,
    /// The local model first, then the small model only for a message it flags.
    LocalThenReader,
}

/// The API's verbs. All of them run on the Runner the request names.
pub fn is_method(method: &str) -> bool {
    matches!(
        method,
        "schedule_models.list" | "schedule_models.status" | "schedule_models.preview" | "schedule_models.apply" | "schedule_models.select" | "schedule_models.remove"
    )
}

/// Routes a `schedule_models.*` call from the user's apps to the Runner it names.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if !is_method(method) {
        return Err("unknown_schedule_models_method".into());
    }
    let runner_id = params["runner_id"].as_str().ok_or("runner_id_required")?.to_owned();
    let runner = app.device(&runner_id).or_else(|| app.local_device().filter(|d| d.id == runner_id)).ok_or("runner_unknown")?;
    if !runner.is_runner() {
        return Err("runner_required".into());
    }
    let mut body = params;
    body.as_object_mut().ok_or("invalid_request")?.remove("runner_id");
    if app.this_device_id().as_deref() != Some(&runner_id) {
        // An install downloads up to 100 MB and trains a classifier before it answers.
        return crate::requests::ask_within(app, &runner_id, method, body, std::time::Duration::from_secs(900)).await;
    }
    serve_as(app, method, body, &app.this_device_id().ok_or("identity_required")?).await
}

/// Serves a call on this Runner for `actor`, this Device or a paired one.
pub async fn serve_as(app: &Arc<App>, method: &str, params: Value, actor: &str) -> Result<Value, String> {
    #[cfg(feature = "runner")]
    return runner::serve(app, method, params, actor).await;
    #[cfg(not(feature = "runner"))]
    {
        let _ = (app, method, params, actor);
        Err("runner_required".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_valid_and_every_model_fits() {
        let catalog = catalog();
        assert!(catalog.models.len() >= 5);
        for model in &catalog.models {
            assert!(model.size() <= MAX_MODEL_BYTES, "{}", model.id);
            for (_, file) in model.files() {
                let url = model.url(file);
                assert!(url.starts_with(&format!("{HUGGING_FACE}/{}/resolve/", model.repo)), "{url}");
                assert!(reqwest::Url::parse(&url).is_ok(), "{url}");
            }
            let listing = model.listing();
            assert_eq!(listing["id"], model.id.as_str());
            assert_eq!(listing["downloads"].as_array().unwrap().len(), 2);
        }
        // The pinned model is the one measured, pinned from the PyPI wheel that ships it.
        let measured = catalog.model("wordllama-l2-256").unwrap();
        assert!(measured.pinned() && measured.measured.is_some());
        assert_eq!(measured.size(), 16_384_096 + 1_842_796);
        assert_eq!(
            measured.url(measured.files.weights.as_ref().unwrap()),
            format!("https://huggingface.co/dleemiller/word-llama-l2-supercat/resolve/{}/l2_supercat_256.safetensors", measured.revision)
        );
        assert!(catalog.models.iter().any(|m| m.engine == Engine::Onnx) && catalog.models.iter().any(|m| !m.pinned()));
    }

    #[test]
    fn invalid_entries_are_refused() {
        let good = catalog().model("wordllama-l2-256").unwrap().clone();
        let broken = |change: &dyn Fn(&mut CatalogModel)| {
            let mut model = good.clone();
            change(&mut model);
            model.validate().unwrap_err()
        };
        assert!(broken(&|m| m.id = "Bad Id".into()).contains("id"));
        assert!(broken(&|m| m.repo = "../etc".into()).contains("owner/name"));
        assert!(broken(&|m| m.revision = "main/../x".into()).contains("revision"));
        assert!(broken(&|m| m.files.tokenizer.path = "../../secrets".into()).contains("unsafe"));
        assert!(broken(&|m| m.files.tokenizer.sha256 = None).contains("both"));
        assert!(broken(&|m| m.files.tokenizer.sha256 = Some("ABC".into())).contains("both"));
        assert!(broken(&|m| m.files.weights.as_mut().unwrap().bytes = Some(MAX_MODEL_BYTES)).contains("more than"));
        assert!(broken(&|m| m.engine = Engine::Onnx).contains("static model"));
        assert!(broken(&|m| {
            m.files.weights.as_mut().unwrap().bytes = None;
            m.files.weights.as_mut().unwrap().sha256 = None;
        })
        .contains("approximate"));
        let mut twice = catalog().clone();
        twice.models.push(good.clone());
        assert!(twice.validate().unwrap_err().contains("twice"));
    }

    #[test]
    fn detection_reads_as_the_api_writes_it() {
        assert_eq!(Detection::default(), Detection::Reader);
        for (mode, text) in [
            (Detection::Off, "off"),
            (Detection::Reader, "reader"),
            (Detection::Local, "local"),
            (Detection::LocalThenReader, "local_then_reader"),
        ] {
            assert_eq!(serde_json::to_value(mode).unwrap(), text);
            assert_eq!(serde_json::from_value::<Detection>(text.into()).unwrap(), mode);
        }
        assert!(is_method("schedule_models.apply") && !is_method("schedule_models.delete_everything"));
    }
}
