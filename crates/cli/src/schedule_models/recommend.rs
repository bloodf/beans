//! Which catalog model suits this computer and these chats: a suggestion the user sees before
//! choosing what to download. Nothing downloads from here. Pure, so every rule is tested.
//!
//! A model fits when this computer can run it and get it: an ONNX or image model needs the ONNX
//! Runtime, a model must take at most a quarter of the memory, its download must leave room on
//! disk, and it must be pinned (or already installed). Among models that fit, a higher catalog
//! `rank` wins; chats in several languages want a multilingual model; an installed model beats
//! an equal one to download; transformers lose a little on a computer with few cores. An image
//! model is suggested only for chats that carry pictures.

use serde::Serialize;

use super::{Catalog, CatalogModel, Engine};

/// What the suggestion knows of this computer. Unknown facts never rule a model out.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Machine {
    pub memory_bytes: Option<u64>,
    pub free_disk_bytes: Option<u64>,
    pub cores: usize,
    pub os: String,
    pub arch: String,
    /// Whether ONNX and image models can run here: the binary can load a runtime, and one is
    /// beside it or pinned to download.
    pub onnx_runtime: bool,
    /// Disk the runtime takes when an ONNX or image model's install downloads it (none is here).
    #[serde(default)]
    pub runtime_download_bytes: u64,
}

/// What the chats on this Runner call for.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Needs {
    /// Messages are written in more than English.
    pub multilingual: bool,
    /// Messages carry pictures.
    pub images: bool,
}

/// One model against this computer: whether it fits, why or why not, and how it ranks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fit {
    pub fits: bool,
    /// Why it does not fit (`needs_onnx_runtime`, `not_enough_memory`, `not_enough_disk`,
    /// `not_downloadable`), then notes (`english_only`, `multilingual`, `slow_here`, `installed`).
    pub reasons: Vec<&'static str>,
    /// About how much memory it takes when loaded.
    pub memory_bytes: u64,
    #[serde(skip)]
    pub score: i64,
}

/// The models to suggest for text and for pictures (only when the chats carry some), if any fits.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Recommended {
    pub text: Option<String>,
    pub image: Option<String>,
}

/// Disk a download must leave free.
const DISK_HEADROOM: u64 = 1_000_000_000;
/// What ONNX Runtime itself takes in memory.
const RUNTIME_MEMORY: u64 = 150_000_000;

/// About how much memory `model` takes loaded: a static table about its size, a transformer
/// its weights and activations, with the runtime.
pub fn memory_bytes(model: &CatalogModel) -> u64 {
    let size = model.size().min(super::MAX_MODEL_BYTES);
    match model.engine {
        Engine::Static => size + size / 5,
        Engine::Onnx | Engine::Clip => size * 5 / 2 + RUNTIME_MEMORY,
    }
}

pub fn fit(model: &CatalogModel, machine: &Machine, needs: &Needs, installed: bool) -> Fit {
    let memory = memory_bytes(model);
    let mut reasons = Vec::new();
    if model.engine != Engine::Static && !machine.onnx_runtime {
        reasons.push("needs_onnx_runtime");
    }
    if machine.memory_bytes.is_some_and(|total| memory > total / 4) {
        reasons.push("not_enough_memory");
    }
    let runtime = if model.engine == Engine::Static { 0 } else { machine.runtime_download_bytes };
    if !installed && machine.free_disk_bytes.is_some_and(|free| model.size().saturating_add(runtime).saturating_add(DISK_HEADROOM) > free) {
        reasons.push("not_enough_disk");
    }
    if !installed && !model.pinned() {
        reasons.push("not_downloadable");
    }
    let fits = reasons.is_empty();
    let mut score = i64::from(model.rank) * 10;
    if model.kind() == "text" && needs.multilingual {
        if model.multilingual {
            score += 25;
        } else {
            score -= 30;
            reasons.push("english_only");
        }
    }
    if model.multilingual {
        reasons.push("multilingual");
    }
    if model.engine != Engine::Static && machine.cores > 0 && machine.cores < 4 {
        score -= 5;
        reasons.push("slow_here");
    }
    if installed {
        score += 3;
        reasons.push("installed");
    }
    Fit { fits, reasons, memory_bytes: memory, score }
}

/// Every model's fit, in catalog order, and the best fitting text and image models: the highest
/// score, the catalog's order breaking ties.
pub fn recommend(catalog: &Catalog, machine: &Machine, needs: &Needs, installed: impl Fn(&str) -> bool) -> (Vec<(String, Fit)>, Recommended) {
    let fits: Vec<(String, Fit)> = catalog.models.iter().map(|m| (m.id.clone(), fit(m, machine, needs, installed(&m.id)))).collect();
    let best = |kind: &str| {
        catalog
            .models
            .iter()
            .zip(&fits)
            .filter(|(model, (_, fit))| model.kind() == kind && fit.fits)
            .fold(None::<(&CatalogModel, &Fit)>, |best, (model, (_, fit))| match best {
                Some((_, kept)) if kept.score >= fit.score => best,
                _ => Some((model, fit)),
            })
            .map(|(model, _)| model.id.clone())
    };
    let recommended = Recommended { text: best("text"), image: if needs.images { best("image") } else { None } };
    (fits, recommended)
}

/// Whether `texts` (recent messages) are written in more than English: over 5% of their letters
/// outside the Latin script, or Latin words that read as Spanish, Portuguese, French, German, or
/// Italian more than half as often as English. A suggestion, so a rough count is enough.
pub fn needs_multilingual(texts: &[String]) -> bool {
    const ENGLISH: &[&str] = &["the", "and", "is", "are", "to", "of", "in", "that", "it", "you", "for", "with", "this", "what", "have", "not", "be", "was", "on", "can"];
    const OTHERS: &[&str] = &[
        "de", "la", "que", "el", "los", "las", "para", "por", "não", "uma", "um", "com", "les", "des", "une", "et", "pas", "der", "die", "und", "das",
        "ist", "nicht", "ein", "eine", "che", "della", "per", "sono", "como", "mais", "muito", "você", "pero", "esto", "está",
    ];
    let (mut latin, mut other, mut english, mut others) = (0usize, 0usize, 0usize, 0usize);
    for text in texts {
        for character in text.chars().filter(|c| c.is_alphabetic()) {
            if character <= '\u{24F}' {
                latin += 1;
            } else {
                other += 1;
            }
        }
        for word in text.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
            let word = word.to_lowercase();
            if ENGLISH.contains(&word.as_str()) {
                english += 1;
            } else if OTHERS.contains(&word.as_str()) {
                others += 1;
            }
        }
    }
    let letters = latin + other;
    letters > 0 && (other * 20 > letters || (others >= 10 && others * 2 > english))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        super::super::catalog().clone()
    }

    /// Pins every model at its listed approximate size, as the pin script would.
    fn pin_all(catalog: &mut Catalog) {
        for model in &mut catalog.models {
            let share = model.approx_bytes.unwrap_or(1_000_000) / model.files().len() as u64;
            for file in [model.files.weights.as_mut(), model.files.model.as_mut(), model.files.image_model.as_mut(), Some(&mut model.files.tokenizer)].into_iter().flatten() {
                file.bytes.get_or_insert(share);
                file.sha256.get_or_insert("a".repeat(64));
            }
        }
    }

    fn roomy() -> Machine {
        Machine { memory_bytes: Some(16 << 30), free_disk_bytes: Some(200 << 30), cores: 8, os: "macos".into(), arch: "aarch64".into(), onnx_runtime: true, runtime_download_bytes: 0 }
    }

    const NONE: fn(&str) -> bool = |_| false;

    #[test]
    fn a_roomy_computer_gets_the_best_ranked_model_that_can_download() {
        let (_, shipped) = recommend(&catalog(), &roomy(), &Needs::default(), NONE);
        // As shipped only the measured static model is pinned: the only one to suggest.
        assert_eq!(shipped.text.as_deref(), Some("wordllama-l2-256"));
        assert_eq!(shipped.image, None, "no image model is pinned yet");
        let mut pinned = catalog();
        pin_all(&mut pinned);
        let pictures = Needs { multilingual: false, images: true };
        let (_, best) = recommend(&pinned, &roomy(), &pictures, NONE);
        assert_eq!(best.text.as_deref(), Some("bge-small-en-v1.5-int8"));
        assert_eq!(best.image.as_deref(), Some("clip-vit-b32-int8"));
        let (fits, words_only) = recommend(&pinned, &roomy(), &Needs::default(), NONE);
        assert_eq!(words_only.image, None, "no pictures in the chats, no picture model to suggest");
        assert!(fits.iter().find(|(id, _)| id == "clip-vit-b32-int8").unwrap().1.fits, "it still fits, for the user to choose");
        let (_, multilingual) = recommend(&pinned, &roomy(), &Needs { multilingual: true, images: true }, NONE);
        assert_eq!(multilingual.text.as_deref(), Some("multilingual-e5-small-int8"));
    }

    #[test]
    fn what_this_computer_cannot_run_or_get_is_never_suggested() {
        let mut pinned = catalog();
        pin_all(&mut pinned);
        // Without ONNX Runtime (a static Linux build): static models only, and no pictures.
        let no_runtime = Machine { onnx_runtime: false, ..roomy() };
        let (fits, best) = recommend(&pinned, &no_runtime, &Needs { multilingual: true, images: true }, NONE);
        assert_eq!(pinned.model(best.text.as_deref().unwrap()).unwrap().engine, Engine::Static);
        assert_eq!(best.image, None);
        let onnx = fits.iter().find(|(id, _)| id == "bge-small-en-v1.5-int8").unwrap();
        assert!(!onnx.1.fits && onnx.1.reasons.contains(&"needs_onnx_runtime"));
        // A small computer: a quarter of its memory rules out the larger models.
        let small = Machine { memory_bytes: Some(1 << 30), ..roomy() };
        let (fits, best) = recommend(&pinned, &small, &Needs::default(), NONE);
        let picked = pinned.model(best.text.as_deref().unwrap()).unwrap();
        assert!(memory_bytes(picked) <= (1 << 30) / 4);
        assert!(fits.iter().any(|(_, fit)| fit.reasons.contains(&"not_enough_memory")));
        // A full disk rules out every download, but not what is installed.
        let full = Machine { free_disk_bytes: Some(500_000_000), ..roomy() };
        let (_, nothing) = recommend(&pinned, &full, &Needs::default(), NONE);
        assert_eq!(nothing, Recommended::default());
        let (_, kept) = recommend(&pinned, &full, &Needs::default(), |id| id == "potion-base-8m");
        assert_eq!(kept.text.as_deref(), Some("potion-base-8m"));
        // A runtime that downloads with the model needs its own room: the disk that holds a
        // transformer alone cannot hold it with its runtime, but a static table needs none.
        let onnx = pinned.model("bge-small-en-v1.5-int8").unwrap();
        let room = onnx.size() + DISK_HEADROOM;
        let tight = Machine { free_disk_bytes: Some(room), runtime_download_bytes: 30_000_000, ..roomy() };
        assert!(fit(onnx, &Machine { runtime_download_bytes: 0, ..tight.clone() }, &Needs::default(), false).fits);
        assert!(fit(onnx, &tight, &Needs::default(), false).reasons.contains(&"not_enough_disk"));
        let table = pinned.model("potion-base-8m").unwrap();
        let exact = Machine { free_disk_bytes: Some(table.size() + DISK_HEADROOM), ..tight };
        assert!(fit(table, &exact, &Needs::default(), false).fits, "exactly enough room fits");
        // Nothing known of the computer rules nothing out.
        let unknown = Machine { memory_bytes: None, free_disk_bytes: None, cores: 0, onnx_runtime: true, ..Default::default() };
        assert!(recommend(&pinned, &unknown, &Needs::default(), NONE).0.iter().all(|(_, fit)| fit.fits));
    }

    #[test]
    fn notes_explain_a_suggestion() {
        let mut pinned = catalog();
        pin_all(&mut pinned);
        let few_cores = Machine { cores: 2, ..roomy() };
        let needs = Needs { multilingual: true, images: false };
        let (fits, _) = recommend(&pinned, &few_cores, &needs, |id| id == "wordllama-l2-256");
        let reasons = |id: &str| fits.iter().find(|(each, _)| each == id).unwrap().1.reasons.clone();
        assert!(reasons("bge-small-en-v1.5-int8").contains(&"english_only"));
        assert!(reasons("bge-small-en-v1.5-int8").contains(&"slow_here"));
        assert!(reasons("multilingual-e5-small-int8").contains(&"multilingual"));
        assert!(reasons("wordllama-l2-256").contains(&"installed"));
        assert!(!reasons("wordllama-l2-256").contains(&"slow_here"), "a static table is fast anywhere");
        // An unpinned model that is already installed fits: it needs no download.
        let shipped = catalog();
        let (fits, best) = recommend(&shipped, &roomy(), &needs, |id| id == "multilingual-e5-small-int8");
        assert!(fits.iter().find(|(id, _)| id == "multilingual-e5-small-int8").unwrap().1.fits);
        assert_eq!(best.text.as_deref(), Some("multilingual-e5-small-int8"));
    }

    #[test]
    fn ties_keep_the_catalog_order_and_estimates_stay_in_bounds() {
        let mut pinned = catalog();
        pin_all(&mut pinned);
        for model in &mut pinned.models {
            model.rank = 1;
        }
        let (_, best) = recommend(&pinned, &roomy(), &Needs::default(), NONE);
        assert_eq!(best.text.as_deref(), Some(pinned.models[0].id.as_str()), "the first of equals");
        let mut huge = pinned.models[0].clone();
        huge.files.weights.as_mut().unwrap().bytes = Some(u64::MAX);
        assert!(memory_bytes(&huge) < u64::MAX / 2, "no overflow on a nonsense size");
        assert!(!fit(&huge, &roomy(), &Needs::default(), false).fits);
    }

    #[test]
    fn languages_are_read_from_the_letters_and_common_words() {
        let say = |texts: &[&str]| needs_multilingual(&texts.iter().map(|t| t.to_string()).collect::<Vec<_>>());
        assert!(!say(&[]));
        assert!(!say(&["   ", "1234 !!!"]), "no letters, nothing to go on");
        assert!(!say(&["Can you deploy the release to staging and check that it is fine?"]));
        assert!(say(&["明天的会议在上海"]));
        assert!(say(&["Please summarize", "これは日本語です"]), "a few non-Latin messages among English ones");
        assert!(!say(&[&format!("{} 上", "the report is ready for review ".repeat(20))]), "one stray character is not a language");
        assert!(say(&["Pode resumir os e-mails de hoje para mim e agende para que isso rode todos os dias, por favor, com os pontos mais importantes e uma lista de tarefas que eu não vi"]));
        assert!(say(&["Привет, как дела?"]));
        assert!(!say(&["Café résumé naïve"]), "accented Latin letters alone are not another language");
    }
}
