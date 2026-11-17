//! Telling a request to repeat something from everything else with any embedding model: a
//! logistic regression over the model's sentence vectors, trained on the Runner from
//! `training.json` once per installed model and then kept. `scripts/schedule-intent-eval` does
//! the same in Python, from the same files, to measure a model before it is offered.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Template messages in nine languages, expanded by `training_messages`.
pub const TRAINING_JSON: &str = include_str!("training.json");
/// Hand-written messages no model trains on, for measuring one.
pub const HELD_OUT_JSON: &str = include_str!("held_out.json");

/// Regularization as scikit-learn's `C`: the value the evaluation measured.
const C: f64 = 3.0;
const EPOCHS: usize = 1500;
const LEARNING_RATE: f64 = 0.05;
/// Below this in every component, the gradient counts as zero.
const CONVERGED: f64 = 1e-6;

/// The most clauses, and the most characters of each, one message is read as.
const MAX_PARTS: usize = 48;
const MAX_PART_CHARS: usize = 600;

/// One labelled message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub language: String,
    pub text: String,
    pub recurring: bool,
}

#[derive(Deserialize)]
struct Training {
    languages: BTreeMap<String, Language>,
}

#[derive(Deserialize)]
struct Language {
    tasks: Vec<String>,
    rec: Vec<String>,
    once: Vec<String>,
    habit: Vec<String>,
    hv: Vec<String>,
    ask: Vec<String>,
    q: Vec<String>,
}

/// Requests to repeat something (every task with every recurring phrase in every way of asking),
/// and what is not one: the same tasks once or at a time, bare tasks, habits, and questions. The
/// same expansion as `generate` in scripts/schedule-intent-eval/training.py.
pub fn training_messages() -> Vec<Example> {
    let training: Training = serde_json::from_str(TRAINING_JSON).expect("training.json is checked by the tests");
    let mut examples = Vec::new();
    for (language, l) in &training.languages {
        let mut push = |text: String, recurring: bool| {
            examples.push(Example {
                language: language.clone(),
                text,
                recurring,
            })
        };
        for task in &l.tasks {
            for when in &l.rec {
                for ask in &l.ask {
                    push(fill(ask, &[("t", task), ("r", when)]), true);
                }
            }
        }
        for task in &l.tasks {
            for when in &l.once {
                for ask in &l.ask {
                    push(fill(ask, &[("t", task), ("r", when)]), false);
                }
            }
        }
        for task in &l.tasks {
            push(task.clone(), false);
        }
        for habit in &l.habit {
            for verb in &l.hv {
                push(fill(habit, &[("h", verb)]), false);
            }
        }
        for question in &l.q {
            for task in l.tasks.iter().take(2) {
                push(fill(question, &[("t", task)]), false);
            }
        }
    }
    examples
}

/// The hand-written messages in `held_out.json`.
pub fn held_out_messages() -> Vec<Example> {
    #[derive(Deserialize)]
    struct HeldOut {
        messages: Vec<Message>,
    }
    #[derive(Deserialize)]
    struct Message {
        language: String,
        recurring: bool,
        text: String,
    }
    let held_out: HeldOut = serde_json::from_str(HELD_OUT_JSON).expect("held_out.json is checked by the tests");
    held_out
        .messages
        .into_iter()
        .map(|m| Example {
            language: m.language,
            text: m.text,
            recurring: m.recurring,
        })
        .collect()
}

/// What a trained classifier was trained on, so a changed training set retrains it.
pub fn training_digest() -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("beans-schedule-intent-v1\0{C}\0{EPOCHS}\0{LEARNING_RATE}\0{CONVERGED}\0{TRAINING_JSON}").as_bytes())
    )
}

/// Python's `str.format` for the `{name}` fields the templates use.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.to_string();
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// A message as the classifier reads it: whole, then each clause between punctuation, so an
/// ask in the middle of a sentence is read on its own too.
pub fn parts(message: &str) -> Vec<String> {
    let clip = |text: &str| text.chars().take(MAX_PART_CHARS).collect::<String>();
    let mut parts = vec![clip(message.trim())];
    for part in message.split([',', '.', ';', ':', '!', '?', '，', '。', '！', '？', '、', '\n']) {
        let part = part.trim();
        if !part.is_empty() && parts.len() < MAX_PARTS && !parts.iter().any(|p| p == part) {
            parts.push(clip(part));
        }
    }
    parts
}

/// A logistic regression over one model's sentence vectors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classifier {
    pub weights: Vec<f32>,
    pub bias: f32,
    /// `training_digest()` when it was trained.
    pub training: String,
}

impl Classifier {
    /// Fits the weights that minimize `C` times the class-balanced log loss plus half the squared
    /// weights, as scikit-learn's `LogisticRegression(C=3, class_weight="balanced")` does, with
    /// full-batch Adam: deterministic, and quick for a few thousand vectors.
    pub fn train(features: &[Vec<f32>], labels: &[bool]) -> Option<Classifier> {
        let n = features.len();
        let d = features.first()?.len();
        let positives = labels.iter().filter(|l| **l).count();
        if n != labels.len() || d == 0 || positives == 0 || positives == n || features.iter().any(|f| f.len() != d) {
            return None;
        }
        let weight_of = |label: bool| n as f64 / (2.0 * if label { positives } else { n - positives } as f64);
        // Each example once as f64, with its label's target and class weight folded in.
        let rows: Vec<(Vec<f64>, f64, f64)> = features
            .iter()
            .zip(labels)
            .map(|(x, &label)| (x.iter().map(|v| *v as f64).collect(), if label { 1.0 } else { 0.0 }, weight_of(label) / n as f64))
            .collect();
        // The objective divided by C·n, so the step size does not depend on how many examples there are.
        let l2 = 1.0 / (C * n as f64);
        // The weights, then the bias last.
        let mut w = vec![0f64; d + 1];
        let (mut m, mut v) = (vec![0f64; d + 1], vec![0f64; d + 1]);
        let (beta1, beta2, epsilon) = (0.9f64, 0.999f64, 1e-8f64);
        let mut gradient = vec![0f64; d + 1];
        for step in 1..=EPOCHS {
            gradient.iter_mut().for_each(|g| *g = 0.0);
            for (x, target, weight) in &rows {
                let z = w[d] + x.iter().zip(&w).map(|(a, b)| a * b).sum::<f64>();
                let error = (sigmoid(z) - target) * weight;
                for (g, a) in gradient.iter_mut().zip(x) {
                    *g += error * a;
                }
                gradient[d] += error;
            }
            for (g, weight) in gradient.iter_mut().zip(&w).take(d) {
                *g += l2 * weight;
            }
            // At the optimum the gradient vanishes; stop once it has.
            if gradient.iter().all(|g| g.abs() < CONVERGED) {
                break;
            }
            for i in 0..=d {
                m[i] = beta1 * m[i] + (1.0 - beta1) * gradient[i];
                v[i] = beta2 * v[i] + (1.0 - beta2) * gradient[i] * gradient[i];
                let m_hat = m[i] / (1.0 - beta1.powi(step as i32));
                let v_hat = v[i] / (1.0 - beta2.powi(step as i32));
                w[i] -= LEARNING_RATE * m_hat / (v_hat.sqrt() + epsilon);
            }
        }
        let weights = w[..d].iter().map(|w| *w as f32).collect();
        Some(Classifier {
            weights,
            bias: w[d] as f32,
            training: training_digest(),
        })
    }

    /// The log-odds that a vector is a request to repeat something; above 0 is more likely than not.
    pub fn score(&self, vector: &[f32]) -> f32 {
        self.bias + vector.iter().zip(&self.weights).map(|(a, b)| a * b).sum::<f32>()
    }

    /// The best-scoring part of a message and its score, from the vectors of `parts(message)`.
    pub fn best(&self, vectors: &[Vec<f32>]) -> Option<(usize, f32)> {
        vectors.iter().map(|v| self.score(v)).enumerate().max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Whether it was trained for `dimensions`-wide vectors on today's training set.
    pub fn fits(&self, dimensions: usize) -> bool {
        self.weights.len() == dimensions && self.training == training_digest() && self.weights.iter().all(|w| w.is_finite()) && self.bias.is_finite()
    }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_training_set_expands_as_the_python_harness_does() {
        let examples = training_messages();
        // scripts/schedule-intent-eval/training.py generates 2831 messages, 1554 of them requests.
        assert_eq!(examples.len(), 2831);
        assert_eq!(examples.iter().filter(|e| e.recurring).count(), 1554);
        assert_eq!(examples.iter().map(|e| e.language.as_str()).collect::<std::collections::BTreeSet<_>>().len(), 9);
        assert!(examples.iter().any(|e| e.recurring && e.text == "agende para resuma meus e-mails todos os dias"));
        assert!(examples.iter().any(|e| !e.recurring && e.text == "I usually go running every morning"));
        assert!(examples.iter().all(|e| !e.text.contains('{')), "every field filled");
        let held_out = held_out_messages();
        assert_eq!((held_out.len(), held_out.iter().filter(|e| e.recurring).count()), (40, 20));
        let trained: std::collections::BTreeSet<_> = examples.iter().map(|e| e.text.as_str()).collect();
        assert!(held_out.iter().all(|e| !trained.contains(e.text.as_str())), "no held-out message is trained on");
    }

    #[test]
    fn a_message_is_read_whole_and_by_clause() {
        assert_eq!(
            parts("Summarize my inbox, and run this daily. Thanks!"),
            ["Summarize my inbox, and run this daily. Thanks!", "Summarize my inbox", "and run this daily", "Thanks"]
        );
        assert_eq!(
            parts("メールを要約して、これを毎日実行して。"),
            ["メールを要約して、これを毎日実行して。", "メールを要約して", "これを毎日実行して"]
        );
        assert_eq!(parts("   "), [""]);
        let long = "x,".repeat(500);
        assert!(parts(&long).len() <= MAX_PARTS);
        assert!(parts(&"y".repeat(5000))[0].chars().count() == MAX_PART_CHARS);
    }

    /// Two separable clouds: the fit separates them and is the same every time.
    #[test]
    fn training_separates_and_repeats_exactly() {
        let mut features = Vec::new();
        let mut labels = Vec::new();
        for i in 0..60 {
            let jitter = (i % 7) as f32 * 0.01;
            features.push(vec![1.0 + jitter, 0.2, -jitter]);
            labels.push(true);
            features.push(vec![-0.3, 1.0 - jitter, jitter]);
            labels.push(false);
        }
        // Three times as many of one class: balancing keeps the boundary between the clouds.
        for _ in 0..120 {
            features.push(vec![-0.3, 1.0, 0.0]);
            labels.push(false);
        }
        let classifier = Classifier::train(&features, &labels).unwrap();
        assert_eq!(Classifier::train(&features, &labels).unwrap(), classifier, "deterministic");
        assert!(classifier.score(&[1.0, 0.2, 0.0]) > 2.0);
        assert!(classifier.score(&[-0.3, 1.0, 0.0]) < -2.0);
        assert_eq!(classifier.best(&[vec![-0.3, 1.0, 0.0], vec![1.0, 0.2, 0.0]]).unwrap().0, 1);
        assert!(classifier.fits(3) && !classifier.fits(4));
        let stale = Classifier {
            training: "old".into(),
            ..classifier.clone()
        };
        assert!(!stale.fits(3));
        // Nothing to learn from.
        assert!(Classifier::train(&[], &[]).is_none());
        assert!(Classifier::train(&[vec![1.0]], &[true]).is_none());
        assert!(Classifier::train(&[vec![1.0], vec![1.0, 2.0]], &[true, false]).is_none());
    }
}
