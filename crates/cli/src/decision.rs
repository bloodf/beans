//! Typed-decision contract for the TypeSafe System One wire (#169).
//!
//! Pure validation: the request and response shapes, the local request rules,
//! and the fail-closed checks that run before any answer is used. No HTTP, no
//! credential, no cost, no repair. A decision outcome is advisory and may only
//! rank candidates the caller was already permitted to choose.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Documented probability-sum tolerance, inclusive: `1.01` is accepted, `1.02`
/// is not. The same tolerance absorbs the weighted rounding of a score.
pub const PROBABILITY_SUM_TOLERANCE: f64 = 0.01;
/// Binary floating point cannot hold the boundary exactly: `0.5 + 0.51` lands a
/// few ULP above `1.01`. This is representation slack only, never a wider
/// contract tolerance, so `1.02` is still rejected.
const PROBABILITY_SUM_ROUNDING: f64 = 1e-9;
pub const MIN_SCORE_LEVELS: usize = 2;
pub const MAX_SCORE_LEVELS: usize = 10;
pub const MAX_CHOICE_OPTIONS: usize = 255;

/// Moving aliases the vendor resolves for us today; naming one would let a
/// release move the answered model under a pinned request.
pub const MODEL_ALIASES: [&str; 2] = ["jev-latest", "jev-preview"];

/// A pinned versioned id such as `jev-1.13.0`: three dot-separated numbers
/// behind the model family prefix, and never one of [`MODEL_ALIASES`].
fn is_pinned_model(model: &str) -> bool {
    if MODEL_ALIASES.contains(&model) {
        return false;
    }
    let Some(version) = model.strip_prefix("jev-") else {
        return false;
    };
    let mut parts = version.split('.');
    let numeric = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    });
    numeric && parts.next().is_none()
}

/// `instructions` and score levels accept the documented structured forms.
fn is_text_or_structure(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_))
}

/// A choice option may also be `null`, the documented way to leave it undescribed.
fn is_option_value(value: &Value) -> bool {
    value.is_null() || is_text_or_structure(value)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub state: Value,
    pub model: String,
    pub questions: BTreeMap<String, DecisionQuestion>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum DecisionQuestion {
    Noul {
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        // JSON keys are strings on the wire, so the documented `true` and
        // `false` arrive quoted; `validate_request` checks they are exactly those.
        criteria: Option<BTreeMap<String, Value>>,
    },
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionQuestionType {
    Noul,
    Choice,
    Score,
}

impl DecisionQuestion {
    pub fn question_type(&self) -> DecisionQuestionType {
        match self {
            DecisionQuestion::Noul { .. } => DecisionQuestionType::Noul,
            DecisionQuestion::Choice { .. } => DecisionQuestionType::Choice,
            DecisionQuestion::Score { .. } => DecisionQuestionType::Score,
        }
    }

    fn instructions(&self) -> &Value {
        match self {
            DecisionQuestion::Noul { instructions, .. }
            | DecisionQuestion::Choice { instructions, .. }
            | DecisionQuestion::Score { instructions, .. } => instructions,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionResponse {
    /// The versioned id that answered.
    pub model: String,
    pub answers: BTreeMap<String, DecisionAnswer>,
    pub usage: DecisionUsage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// No `deny_unknown_fields`: vendor extras such as `routing`, `id` or
/// `provider` are ignored, while an unknown `type` fails the whole parse.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        #[serde(rename = "legend")]
        level_legend: BTreeMap<String, Value>,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

impl DecisionAnswer {
    fn answer_type(&self) -> DecisionQuestionType {
        match self {
            DecisionAnswer::Noul { .. } => DecisionQuestionType::Noul,
            DecisionAnswer::Choice { .. } => DecisionQuestionType::Choice,
            DecisionAnswer::Score { .. } => DecisionQuestionType::Score,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionOutcome {
    Noul {
        question: String,
        probability: f64,
    },
    Choice {
        question: String,
        selected: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        question: String,
        /// The probability-weighted position, which may land between levels.
        position: f64,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

impl DecisionOutcome {
    pub fn question(&self) -> &str {
        match self {
            DecisionOutcome::Noul { question, .. }
            | DecisionOutcome::Choice { question, .. }
            | DecisionOutcome::Score { question, .. } => question,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionRejection {
    ModelNotPinned { model: String },
    QuestionIdBlank { question: String },
    StateShape,
    InstructionsShape { question: String },
    CriteriaValueShape { question: String },
    CriteriaBounds { question: String, count: usize },
    NoulCriteriaKeys { question: String },
    AnswerMissing { question: String },
    AnswerUnexpected { question: String },
    AnswerTypeMismatch {
        question: String,
        asked: DecisionQuestionType,
        answered: &'static str,
    },
    ProbabilityNotFinite {
        question: String,
        option: String,
    },
    ProbabilityOutOfRange {
        question: String,
        option: String,
        value: f64,
    },
    ProbabilityMissing {
        question: String,
        option: String,
    },
    ProbabilityUnexpected {
        question: String,
        option: String,
    },
    ProbabilitySum { question: String },
    NoulNotFinite {
        question: String,
        value: f64,
    },
    NoulOutOfRange {
        question: String,
        value: f64,
    },
    ConfidenceNotFinite {
        question: String,
        value: f64,
    },
    ConfidenceOutOfRange {
        question: String,
        value: f64,
    },
    ScoreNotFinite {
        question: String,
        value: f64,
    },
    ScoreOutsideLevels {
        question: String,
        position: f64,
        levels: usize,
    },
    ChoiceNotMaximum {
        question: String,
        selected: String,
        probability: f64,
    },
    ScoreWeightedMeanMismatch {
        question: String,
        position: f64,
        weighted: f64,
    },
    LegendMismatch { question: String },
    ModelMismatch {
        requested: String,
        served: String,
    },
}

/// Local request rules, run before any bytes leave the Runner.
pub fn validate_request(request: &DecisionRequest) -> Result<(), DecisionRejection> {
    if !is_pinned_model(&request.model) {
        return Err(DecisionRejection::ModelNotPinned {
            model: request.model.clone(),
        });
    }
    if !is_text_or_structure(&request.state) {
        return Err(DecisionRejection::StateShape);
    }
    for (id, question) in &request.questions {
        if id.trim().is_empty() {
            return Err(DecisionRejection::QuestionIdBlank {
                question: id.clone(),
            });
        }
        if !is_text_or_structure(question.instructions()) {
            return Err(DecisionRejection::InstructionsShape {
                question: id.clone(),
            });
        }
        match question {
            DecisionQuestion::Choice { criteria, .. } => {
                if criteria.is_empty() || criteria.len() > MAX_CHOICE_OPTIONS {
                    return Err(DecisionRejection::CriteriaBounds {
                        question: id.clone(),
                        count: criteria.len(),
                    });
                }
                if !criteria.values().all(is_option_value) {
                    return Err(DecisionRejection::CriteriaValueShape {
                        question: id.clone(),
                    });
                }
            }
            DecisionQuestion::Score { criteria, .. } => {
                if !(MIN_SCORE_LEVELS..=MAX_SCORE_LEVELS).contains(&criteria.len()) {
                    return Err(DecisionRejection::CriteriaBounds {
                        question: id.clone(),
                        count: criteria.len(),
                    });
                }
                if !criteria.iter().all(is_text_or_structure) {
                    return Err(DecisionRejection::CriteriaValueShape {
                        question: id.clone(),
                    });
                }
            }
            DecisionQuestion::Noul { criteria, .. } => {
                if let Some(criteria) = criteria {
                    let keys: Vec<&str> = criteria.keys().map(|key| key.as_str()).collect();
                    if keys != ["false", "true"] || !criteria.values().all(is_text_or_structure) {
                        return Err(DecisionRejection::NoulCriteriaKeys {
                            question: id.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Response rules. Fail-closed, no repair, no default, no skip.
pub fn validate(
    request: &DecisionRequest,
    response: &DecisionResponse,
) -> Result<Vec<DecisionOutcome>, DecisionRejection> {
    validate_request(request)?;
    if response.model != request.model {
        return Err(DecisionRejection::ModelMismatch {
            requested: request.model.clone(),
            served: response.model.clone(),
        });
    }
    for id in response.answers.keys() {
        if !request.questions.contains_key(id) {
            return Err(DecisionRejection::AnswerUnexpected {
                question: id.clone(),
            });
        }
    }

    let mut outcomes = Vec::with_capacity(request.questions.len());
    for (id, question) in &request.questions {
        let Some(answer) = response.answers.get(id) else {
            return Err(DecisionRejection::AnswerMissing {
                question: id.clone(),
            });
        };
        let asked = question.question_type();
        let answered = answer.answer_type();
        if asked != answered {
            return Err(DecisionRejection::AnswerTypeMismatch {
                question: id.clone(),
                asked,
                answered: match answered {
                    DecisionQuestionType::Noul => "noul",
                    DecisionQuestionType::Choice => "choice",
                    DecisionQuestionType::Score => "score",
                },
            });
        }
        outcomes.push(outcome(id, question, answer)?);
    }
    Ok(outcomes)
}

fn outcome(
    id: &str,
    question: &DecisionQuestion,
    answer: &DecisionAnswer,
) -> Result<DecisionOutcome, DecisionRejection> {
    match (question, answer) {
        (DecisionQuestion::Noul { .. }, DecisionAnswer::Noul { noul }) => {
            if !noul.is_finite() {
                return Err(DecisionRejection::NoulNotFinite {
                    question: id.to_string(),
                    value: *noul,
                });
            }
            if !(0.0..=1.0).contains(noul) {
                return Err(DecisionRejection::NoulOutOfRange {
                    question: id.to_string(),
                    value: *noul,
                });
            }
            Ok(DecisionOutcome::Noul {
                question: id.to_string(),
                probability: *noul,
            })
        }
        (DecisionQuestion::Choice { criteria, .. }, DecisionAnswer::Choice { choice, probabilities, confidence }) => {
            if !criteria.contains_key(choice) {
                return Err(DecisionRejection::ProbabilityMissing {
                    question: id.to_string(),
                    option: choice.clone(),
                });
            }
            check_probabilities(id, &criteria.keys().cloned().collect(), probabilities)?;
            check_confidence(id, *confidence)?;
            // The selected option must carry the highest probability. A tie is a
            // legal selection, so only a strictly better option rejects.
            let best = probabilities[choice];
            if probabilities
                .iter()
                .any(|(option, value)| option != choice && *value > best)
            {
                return Err(DecisionRejection::ChoiceNotMaximum {
                    question: id.to_string(),
                    selected: choice.clone(),
                    probability: best,
                });
            }
            Ok(DecisionOutcome::Choice {
                question: id.to_string(),
                selected: choice.clone(),
                probabilities: probabilities.clone(),
                confidence: *confidence,
            })
        }
        (
            DecisionQuestion::Score { criteria, .. },
            DecisionAnswer::Score { score, level_legend, probabilities, confidence },
        ) => {
            let levels = criteria.len();
            let indices: BTreeSet<String> = (0..levels).map(|index| index.to_string()).collect();
            check_probabilities(id, &indices, probabilities)?;
            let legend: BTreeMap<String, Value> = (0..levels)
                .map(|index| (index.to_string(), criteria[index].clone()))
                .collect();
            if *level_legend != legend {
                return Err(DecisionRejection::LegendMismatch {
                    question: id.to_string(),
                });
            }
            if !score.is_finite() {
                return Err(DecisionRejection::ScoreNotFinite {
                    question: id.to_string(),
                    value: *score,
                });
            }
            // The position is a level index and nothing else: no slack, so a
            // value outside the levels is a rejection and never a rounded one.
            let last = (levels - 1) as f64;
            if !(0.0..=last).contains(score) {
                return Err(DecisionRejection::ScoreOutsideLevels {
                    question: id.to_string(),
                    position: *score,
                    levels,
                });
            }
            // The position must also be the probability-weighted mean of the
            // levels, within the documented rounding tolerance. This never
            // repairs a position; it only accepts one the probabilities justify.
            let weighted: f64 = probabilities
                .iter()
                .map(|(index, value)| index.parse::<f64>().unwrap_or(f64::NAN) * value)
                .sum();
            if (score - weighted).abs() > PROBABILITY_SUM_TOLERANCE + PROBABILITY_SUM_ROUNDING {
                return Err(DecisionRejection::ScoreWeightedMeanMismatch {
                    question: id.to_string(),
                    position: *score,
                    weighted,
                });
            }
            check_confidence(id, *confidence)?;
            Ok(DecisionOutcome::Score {
                question: id.to_string(),
                position: *score,
                probabilities: probabilities.clone(),
                confidence: *confidence,
            })
        }
        _ => Err(DecisionRejection::AnswerTypeMismatch {
            question: id.to_string(),
            asked: question.question_type(),
            answered: "other",
        }),
    }
}

fn check_probabilities(
    question: &str,
    expected_keys: &BTreeSet<String>,
    probabilities: &BTreeMap<String, f64>,
) -> Result<(), DecisionRejection> {
    for option in expected_keys {
        if !probabilities.contains_key(option) {
            return Err(DecisionRejection::ProbabilityMissing {
                question: question.to_string(),
                option: option.clone(),
            });
        }
    }
    let mut sum = 0.0;
    for (option, value) in probabilities {
        if !expected_keys.contains(option) {
            return Err(DecisionRejection::ProbabilityUnexpected {
                question: question.to_string(),
                option: option.clone(),
            });
        }
        if !value.is_finite() {
            return Err(DecisionRejection::ProbabilityNotFinite {
                question: question.to_string(),
                option: option.clone(),
            });
        }
        if !(0.0..=1.0).contains(value) {
            return Err(DecisionRejection::ProbabilityOutOfRange {
                question: question.to_string(),
                option: option.clone(),
                value: *value,
            });
        }
        sum += *value;
    }
    if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE + PROBABILITY_SUM_ROUNDING {
        return Err(DecisionRejection::ProbabilitySum {
            question: question.to_string(),
        });
    }
    Ok(())
}

fn check_confidence(question: &str, confidence: f64) -> Result<(), DecisionRejection> {
    if !confidence.is_finite() {
        return Err(DecisionRejection::ConfidenceNotFinite {
            question: question.to_string(),
            value: confidence,
        });
    }
    if !(0.0..=1.0).contains(&confidence) {
        return Err(DecisionRejection::ConfidenceOutOfRange {
            question: question.to_string(),
            value: confidence,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;