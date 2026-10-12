//! Frozen #169 regression, driven from the real System One wire shape.
//!
//! Fixtures are documentation-derived from `https://docs.typesafe.ai/api.md`
//! (read 2026-10-12) and are labelled as such: they pin the contract, they are
//! not adapter acceptance evidence from captured bytes.

use super::{
    validate, validate_request, DecisionAnswer, DecisionRejection, DecisionRequest, DecisionResponse,
};
use serde_json::Value;
use std::collections::BTreeMap;

// Real documented wire.

const CHOICE_REQUEST: &str = r#"{"model":"jev-1.13.0","state":"A user cannot sign in after the SSO migration.","questions":{"route":{"type":"choice","instructions":"Which team should handle this?","criteria":{"billing":"Payments, invoicing, refunds","technical":"Bugs, outages, integrations"}}}}"#;
const CHOICE_RESPONSE: &str = r#"{"model":"jev-1.13.0","answers":{"route":{"type":"choice","choice":"technical","probabilities":{"billing":0.12,"technical":0.88},"confidence":0.88}},"usage":{"input_tokens":210,"output_tokens":12}}"#;
const CHOICE_REQUEST_JSON: &str = r#"{"state":"A user cannot sign in after the SSO migration.","model":"jev-1.13.0","questions":{"route":{"type":"choice","instructions":"Which team should handle this?","criteria":{"billing":"Payments, invoicing, refunds","technical":"Bugs, outages, integrations"}}}}"#;

const SCORE_REQUEST: &str = r#"{"model":"jev-1.13.0","state":"Support transcript: the customer has asked three times for a refund.","questions":{"frustration":{"type":"score","instructions":"How frustrated is the customer?","criteria":["Calm","Frustrated","Very angry"]}}}"#;
const SCORE_RESPONSE: &str = r#"{"model":"jev-1.13.0","answers":{"frustration":{"type":"score","score":1.05,"legend":{"0":"Calm","1":"Frustrated","2":"Very angry"},"probabilities":{"0":0.0,"1":0.95,"2":0.05},"confidence":0.92}},"usage":{"input_tokens":304,"output_tokens":18}}"#;
const SCORE_REQUEST_JSON: &str = r#"{"state":"Support transcript: the customer has asked three times for a refund.","model":"jev-1.13.0","questions":{"frustration":{"type":"score","instructions":"How frustrated is the customer?","criteria":["Calm","Frustrated","Very angry"]}}}"#;

const NOUL_REQUEST: &str = r#"{"model":"jev-1.13.0","state":"A user cannot sign in after the SSO migration.","questions":{"regression":{"type":"noul","instructions":"Does the report describe a regression introduced by the SSO migration?","criteria":{"true":"Introduced by the SSO migration","false":"Unrelated to the SSO migration"}}}}"#;
const NOUL_RESPONSE: &str = r#"{"model":"jev-1.13.0","answers":{"regression":{"type":"noul","noul":0.95}},"usage":{"input_tokens":180,"output_tokens":9}}"#;
const NOUL_REQUEST_JSON: &str = r#"{"state":"A user cannot sign in after the SSO migration.","model":"jev-1.13.0","questions":{"regression":{"type":"noul","instructions":"Does the report describe a regression introduced by the SSO migration?","criteria":{"false":"Unrelated to the SSO migration","true":"Introduced by the SSO migration"}}}}"#;

// Shared outcomes of the three question types.

#[derive(Debug, PartialEq)]
enum CaseResult {
    Serialized(String),
    Accepted(Vec<String>),
    Rejected(DecisionRejection),
    ParseFailed,
}

fn run(request: &str, response: &str) -> CaseResult {
    let request: DecisionRequest = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(_) => return CaseResult::ParseFailed,
    };
    if let Err(rejection) = validate_request(&request) {
        return CaseResult::Rejected(rejection);
    }
    let response: DecisionResponse = match serde_json::from_str(response) {
        Ok(response) => response,
        Err(_) => return CaseResult::ParseFailed,
    };
    match validate(&request, &response) {
        Ok(outcomes) => CaseResult::Accepted(
            outcomes
                .iter()
                .map(|outcome| outcome.question().to_string())
                .collect(),
        ),
        Err(rejection) => CaseResult::Rejected(rejection),
    }
}

/// Rewrites one field of the documented response, for the cases JSON cannot
/// express (`NaN`) and for the ones that need a mutation rather than a fixture.
fn with_choice_probability(response: &str, option: &str, value: f64) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Choice { probabilities, .. } => {
            probabilities.insert(option.to_string(), value);
        }
        other => panic!("expected a choice answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

fn with_choice_confidence(response: &str, value: f64) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Choice { confidence, .. } => *confidence = value,
        other => panic!("expected a choice answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

fn with_score_probability(response: &str, option: &str, value: f64) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Score { probabilities, .. } => {
            probabilities.insert(option.to_string(), value);
        }
        other => panic!("expected a score answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

fn with_score_score(response: &str, value: f64) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Score { score, .. } => *score = value,
        other => panic!("expected a score answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

fn with_score_confidence(response: &str, value: f64) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Score { confidence, .. } => *confidence = value,
        other => panic!("expected a score answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

fn with_score_legend(response: &str, legend: Value) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Score { level_legend, .. } => {
            let legend: BTreeMap<String, Value> = serde_json::from_value(legend).expect("legend");
            *level_legend = legend;
        }
        other => panic!("expected a score answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

/// Drives the public entry point, so a caller cannot reach the response rules
/// with a request the local rules would refuse.
fn validate_public(request: &str, response: &str) -> CaseResult {
    let request: DecisionRequest = serde_json::from_str(request).expect("documented request");
    let response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match validate(&request, &response) {
        Ok(outcomes) => CaseResult::Accepted(outcomes.iter().map(|o| o.question().to_string()).collect()),
        Err(rejection) => CaseResult::Rejected(rejection),
    }
}

fn with_choice_distribution(response: &str, billing: f64, technical: f64) -> CaseResult {
    let mut response: DecisionResponse = serde_json::from_str(response).expect("documented response");
    match response.answers.values_mut().next().expect("one answer") {
        DecisionAnswer::Choice { probabilities, .. } => {
            probabilities.insert("billing".to_string(), billing);
            probabilities.insert("technical".to_string(), technical);
        }
        other => panic!("expected a choice answer, got {other:?}"),
    }
    CaseResult::Rejected(run_validated(&response))
}

fn run_validated(response: &DecisionResponse) -> DecisionRejection {
    let request: DecisionRequest = serde_json::from_str(SCORE_REQUEST).expect("documented request");
    let request = if matches!(
        response.answers.values().next().expect("one answer"),
        DecisionAnswer::Choice { .. }
    ) {
        serde_json::from_str(CHOICE_REQUEST).expect("documented request")
    } else {
        request
    };
    validate(&request, response).expect_err("a mutated documented response must be rejected")
}

// The three question types, then accept, then reject.

fn a_choice_question_serializes_with_its_option_map() -> CaseResult {
    let request: DecisionRequest = serde_json::from_str(CHOICE_REQUEST).expect("documented request");
    CaseResult::Serialized(serde_json::to_string(&request).expect("serialized"))
}

fn a_score_question_serializes_with_ordered_levels() -> CaseResult {
    let request: DecisionRequest = serde_json::from_str(SCORE_REQUEST).expect("documented request");
    CaseResult::Serialized(serde_json::to_string(&request).expect("serialized"))
}

fn a_noul_question_serializes_and_omits_absent_criteria() -> CaseResult {
    let documented: DecisionRequest = serde_json::from_str(NOUL_REQUEST).expect("documented request");
    assert_eq!(
        serde_json::to_string(&documented).expect("serialized"),
        NOUL_REQUEST_JSON
    );
    let bare = r#"{"model":"jev-1.13.0","state":"s","questions":{"regression":{"type":"noul","instructions":"Does the report describe a regression introduced by the SSO migration?"}}}"#;
    let request: DecisionRequest = serde_json::from_str(bare).expect("documented request");
    CaseResult::Serialized(serde_json::to_string(&request).expect("serialized"))
}

fn the_documented_score_response_validates() -> CaseResult {
    let request: DecisionRequest = serde_json::from_str(SCORE_REQUEST).expect("documented request");
    let response: DecisionResponse = serde_json::from_str(SCORE_RESPONSE).expect("documented response");
    let outcomes = match validate(&request, &response) {
        Ok(outcomes) => outcomes,
        Err(rejection) => return CaseResult::Rejected(rejection),
    };
    match &outcomes[..] {
        [super::DecisionOutcome::Score {
            question,
            position,
            probabilities,
            confidence,
        }] => {
            assert_eq!(question, "frustration");
            assert_eq!(*position, 1.05, "a weighted position stays between levels");
            assert_eq!(probabilities["0"], 0.0);
            assert_eq!(probabilities["1"], 0.95);
            assert_eq!(probabilities["2"], 0.05);
            assert_eq!(*confidence, 0.92);
        }
        other => panic!("expected one score outcome, got {other:?}"),
    }
    CaseResult::Accepted(vec![outcomes[0].question().to_string()])
}

fn the_documented_choice_response_validates() -> CaseResult {
    run(CHOICE_REQUEST, CHOICE_RESPONSE)
}

fn the_documented_noul_response_validates() -> CaseResult {
    run(NOUL_REQUEST, NOUL_RESPONSE)
}

fn a_probability_sum_of_1_01_is_accepted() -> CaseResult {
    let response = CHOICE_RESPONSE.replace(r#""billing":0.12,"technical":0.88"#, r#""billing":0.5,"technical":0.51"#);
    assert!(response.contains("0.51"), "the sum-1.01 fixture must be wired");
    run(CHOICE_REQUEST, &response)
}

fn a_probability_sum_of_1_02_is_rejected() -> CaseResult {
    with_choice_probability(CHOICE_RESPONSE, "billing", 0.52)
}

fn a_missing_answer_is_rejected() -> CaseResult {
    run(
        CHOICE_REQUEST,
        r#"{"model":"jev-1.13.0","answers":{},"usage":{"input_tokens":210,"output_tokens":12}}"#,
    )
}

fn an_extra_answer_is_rejected() -> CaseResult {
    run(
        CHOICE_REQUEST,
        r#"{"model":"jev-1.13.0","answers":{"route":{"type":"choice","choice":"technical","probabilities":{"billing":0.12,"technical":0.88},"confidence":0.88},"other":{"type":"noul","noul":0.5}},"usage":{"input_tokens":210,"output_tokens":12}}"#,
    )
}

fn an_answer_of_the_wrong_type_is_rejected() -> CaseResult {
    run(
        CHOICE_REQUEST,
        r#"{"model":"jev-1.13.0","answers":{"route":{"type":"noul","noul":0.9}},"usage":{"input_tokens":210,"output_tokens":12}}"#,
    )
}

fn a_nan_probability_is_rejected() -> CaseResult {
    with_score_probability(SCORE_RESPONSE, "1", f64::NAN)
}

fn a_probability_above_one_is_rejected() -> CaseResult {
    with_choice_probability(CHOICE_RESPONSE, "billing", 1.5)
}

fn an_unrequested_option_is_rejected() -> CaseResult {
    with_choice_probability(CHOICE_RESPONSE, "legal", 0.5)
}

fn a_legend_that_differs_from_the_requested_levels_is_rejected() -> CaseResult {
    with_score_legend(
        SCORE_RESPONSE,
        serde_json::json!({"0": "Calm", "1": "Frustrated", "2": "Furious"}),
    )
}

fn a_score_past_the_last_level_is_rejected() -> CaseResult {
    with_score_score(SCORE_RESPONSE, 3.0)
}

fn a_noul_above_one_is_rejected() -> CaseResult {
    run(
        NOUL_REQUEST,
        r#"{"model":"jev-1.13.0","answers":{"regression":{"type":"noul","noul":1.4}},"usage":{"input_tokens":180,"output_tokens":9}}"#,
    )
}

fn a_confidence_above_one_is_rejected() -> CaseResult {
    with_choice_confidence(CHOICE_RESPONSE, 1.2)
}

fn a_non_finite_score_confidence_is_rejected() -> CaseResult {
    with_score_confidence(SCORE_RESPONSE, f64::INFINITY)
}

fn a_served_model_other_than_the_pinned_id_is_rejected() -> CaseResult {
    run(
        CHOICE_REQUEST,
        &CHOICE_RESPONSE.replace("jev-1.13.0", "jev-1.14.0"),
    )
}

fn a_response_without_usage_fails_to_parse() -> CaseResult {
    run(
        CHOICE_REQUEST,
        r#"{"model":"jev-1.13.0","answers":{"route":{"type":"choice","choice":"technical","probabilities":{"billing":0.12,"technical":0.88},"confidence":0.88}}}"#,
    )
}

fn a_refusal_answer_fails_to_parse() -> CaseResult {
    run(
        CHOICE_REQUEST,
        r#"{"model":"jev-1.13.0","answers":{"route":{"type":"refusal","refusal":"unsupported"}},"usage":{"input_tokens":210,"output_tokens":12}}"#,
    )
}

fn an_invalid_request_is_refused_by_validate() -> CaseResult {
    let alias = CHOICE_REQUEST.replace("jev-1.13.0", "jev-latest");
    validate_public(&alias, CHOICE_RESPONSE)
}

fn an_empty_score_question_is_refused_by_validate() -> CaseResult {
    let empty = r#"{"state":"s","model":"jev-1.13.0","questions":{"frustration":{"type":"score","instructions":"How frustrated is the customer?","criteria":[]}}}"#;
    validate_public(empty, SCORE_RESPONSE)
}

fn a_choice_that_is_not_the_highest_probability_is_rejected() -> CaseResult {
    with_choice_distribution(CHOICE_RESPONSE, 0.6, 0.4)
}

fn a_tied_choice_is_accepted() -> CaseResult {
    let tied = CHOICE_RESPONSE.replace(
        r#""probabilities":{"billing":0.12,"technical":0.88},"confidence":0.88"#,
        r#""probabilities":{"billing":0.5,"technical":0.5},"confidence":0.5"#,
    );
    run(CHOICE_REQUEST, &tied)
}

fn a_negative_score_is_rejected() -> CaseResult {
    with_score_score(SCORE_RESPONSE, -0.005)
}

fn a_score_just_past_the_last_level_is_rejected() -> CaseResult {
    with_score_score(SCORE_RESPONSE, 2.005)
}

fn an_inconsistent_weighted_position_is_rejected() -> CaseResult {
    with_score_score(SCORE_RESPONSE, 1.5)
}

fn an_unknown_request_field_is_rejected() -> CaseResult {
    run(
        &CHOICE_REQUEST.replace(r#""type":"choice","#, r#""type":"choice","temperature":0.2,"#),
        CHOICE_RESPONSE,
    )
}

#[test]
fn the_frozen_decision_contract_regression() {
    type Check = fn() -> CaseResult;
    let cases: [(&str, Check, CaseResult); 30] = [
        (
            "a_choice_question_serializes_with_its_option_map",
            a_choice_question_serializes_with_its_option_map,
            CaseResult::Serialized(CHOICE_REQUEST_JSON.to_string()),
        ),
        (
            "a_score_question_serializes_with_ordered_levels",
            a_score_question_serializes_with_ordered_levels,
            CaseResult::Serialized(SCORE_REQUEST_JSON.to_string()),
        ),
        (
            "a_noul_question_serializes_and_omits_absent_criteria",
            a_noul_question_serializes_and_omits_absent_criteria,
            CaseResult::Serialized(
                r#"{"state":"s","model":"jev-1.13.0","questions":{"regression":{"type":"noul","instructions":"Does the report describe a regression introduced by the SSO migration?"}}}"#
                    .to_string(),
            ),
        ),
        (
            "the_documented_score_response_validates",
            the_documented_score_response_validates,
            CaseResult::Accepted(vec!["frustration".to_string()]),
        ),
        (
            "the_documented_choice_response_validates",
            the_documented_choice_response_validates,
            CaseResult::Accepted(vec!["route".to_string()]),
        ),
        (
            "the_documented_noul_response_validates",
            the_documented_noul_response_validates,
            CaseResult::Accepted(vec!["regression".to_string()]),
        ),
        (
            "a_probability_sum_of_1_01_is_accepted",
            a_probability_sum_of_1_01_is_accepted,
            CaseResult::Accepted(vec!["route".to_string()]),
        ),
        (
            "a_probability_sum_of_1_02_is_rejected",
            a_probability_sum_of_1_02_is_rejected,
            CaseResult::Rejected(DecisionRejection::ProbabilitySum {
                question: "route".to_string(),
            }),
        ),
        (
            "a_missing_answer_is_rejected",
            a_missing_answer_is_rejected,
            CaseResult::Rejected(DecisionRejection::AnswerMissing {
                question: "route".to_string(),
            }),
        ),
        (
            "an_extra_answer_is_rejected",
            an_extra_answer_is_rejected,
            CaseResult::Rejected(DecisionRejection::AnswerUnexpected {
                question: "other".to_string(),
            }),
        ),
        (
            "an_answer_of_the_wrong_type_is_rejected",
            an_answer_of_the_wrong_type_is_rejected,
            CaseResult::Rejected(DecisionRejection::AnswerTypeMismatch {
                question: "route".to_string(),
                asked: super::DecisionQuestionType::Choice,
                answered: "noul",
            }),
        ),
        (
            "a_nan_probability_is_rejected",
            a_nan_probability_is_rejected,
            CaseResult::Rejected(DecisionRejection::ProbabilityNotFinite {
                question: "frustration".to_string(),
                option: "1".to_string(),
            }),
        ),
        (
            "a_probability_above_one_is_rejected",
            a_probability_above_one_is_rejected,
            CaseResult::Rejected(DecisionRejection::ProbabilityOutOfRange {
                question: "route".to_string(),
                option: "billing".to_string(),
                value: 1.5,
            }),
        ),
        (
            "an_unrequested_option_is_rejected",
            an_unrequested_option_is_rejected,
            CaseResult::Rejected(DecisionRejection::ProbabilityUnexpected {
                question: "route".to_string(),
                option: "legal".to_string(),
            }),
        ),
        (
            "a_legend_that_differs_from_the_requested_levels_is_rejected",
            a_legend_that_differs_from_the_requested_levels_is_rejected,
            CaseResult::Rejected(DecisionRejection::LegendMismatch {
                question: "frustration".to_string(),
            }),
        ),
        (
            "a_score_past_the_last_level_is_rejected",
            a_score_past_the_last_level_is_rejected,
            CaseResult::Rejected(DecisionRejection::ScoreOutsideLevels {
                question: "frustration".to_string(),
                position: 3.0,
                levels: 3,
            }),
        ),
        (
            "a_noul_above_one_is_rejected",
            a_noul_above_one_is_rejected,
            CaseResult::Rejected(DecisionRejection::NoulOutOfRange {
                question: "regression".to_string(),
                value: 1.4,
            }),
        ),
        (
            "a_confidence_above_one_is_rejected",
            a_confidence_above_one_is_rejected,
            CaseResult::Rejected(DecisionRejection::ConfidenceOutOfRange {
                question: "route".to_string(),
                value: 1.2,
            }),
        ),
        (
            "a_non_finite_score_confidence_is_rejected",
            a_non_finite_score_confidence_is_rejected,
            CaseResult::Rejected(DecisionRejection::ConfidenceNotFinite {
                question: "frustration".to_string(),
                value: f64::INFINITY,
            }),
        ),
        (
            "a_served_model_other_than_the_pinned_id_is_rejected",
            a_served_model_other_than_the_pinned_id_is_rejected,
            CaseResult::Rejected(DecisionRejection::ModelMismatch {
                requested: "jev-1.13.0".to_string(),
                served: "jev-1.14.0".to_string(),
            }),
        ),
        (
            "a_response_without_usage_fails_to_parse",
            a_response_without_usage_fails_to_parse,
            CaseResult::ParseFailed,
        ),
        (
            "a_refusal_answer_fails_to_parse",
            a_refusal_answer_fails_to_parse,
            CaseResult::ParseFailed,
        ),
        (
            "an_invalid_request_is_refused_by_validate",
            an_invalid_request_is_refused_by_validate,
            CaseResult::Rejected(DecisionRejection::ModelNotPinned {
                model: "jev-latest".to_string(),
            }),
        ),
        (
            "an_empty_score_question_is_refused_by_validate",
            an_empty_score_question_is_refused_by_validate,
            CaseResult::Rejected(DecisionRejection::CriteriaBounds {
                question: "frustration".to_string(),
                count: 0,
            }),
        ),
        (
            "a_choice_that_is_not_the_highest_probability_is_rejected",
            a_choice_that_is_not_the_highest_probability_is_rejected,
            CaseResult::Rejected(DecisionRejection::ChoiceNotMaximum {
                question: "route".to_string(),
                selected: "technical".to_string(),
                probability: 0.4,
            }),
        ),
        (
            "a_tied_choice_is_accepted",
            a_tied_choice_is_accepted,
            CaseResult::Accepted(vec!["route".to_string()]),
        ),
        (
            "a_negative_score_is_rejected",
            a_negative_score_is_rejected,
            CaseResult::Rejected(DecisionRejection::ScoreOutsideLevels {
                question: "frustration".to_string(),
                position: -0.005,
                levels: 3,
            }),
        ),
        (
            "a_score_just_past_the_last_level_is_rejected",
            a_score_just_past_the_last_level_is_rejected,
            CaseResult::Rejected(DecisionRejection::ScoreOutsideLevels {
                question: "frustration".to_string(),
                position: 2.005,
                levels: 3,
            }),
        ),
        (
            "an_inconsistent_weighted_position_is_rejected",
            an_inconsistent_weighted_position_is_rejected,
            CaseResult::Rejected(DecisionRejection::ScoreWeightedMeanMismatch {
                question: "frustration".to_string(),
                position: 1.5,
                weighted: 1.05,
            }),
        ),
        (
            "an_unknown_request_field_is_rejected",
            an_unknown_request_field_is_rejected,
            CaseResult::ParseFailed,
        ),
    ];

    for (name, check, expected) in cases {
        let actual = check();
        let matched = match (&actual, &expected) {
            (CaseResult::Serialized(actual), CaseResult::Serialized(expected)) => actual == expected,
            (CaseResult::Accepted(actual), CaseResult::Accepted(expected)) => actual == expected,
            (CaseResult::Rejected(actual), CaseResult::Rejected(expected)) => actual == expected,
            (CaseResult::ParseFailed, CaseResult::ParseFailed) => true,
            _ => false,
        };
        assert!(matched, "{name} produced {actual:?}, expected {expected:?}");
    }
}

#[test]
fn the_local_request_rules_pin_models_and_value_shapes() {
    let alias = CHOICE_REQUEST.replace("jev-1.13.0", "jev-latest");
    assert_eq!(
        run(&alias, CHOICE_RESPONSE),
        CaseResult::Rejected(DecisionRejection::ModelNotPinned {
            model: "jev-latest".to_string(),
        }),
        "a moving alias is not a pinned versioned id"
    );

    let numeric_instructions = CHOICE_REQUEST.replace(
        r#""instructions":"Which team should handle this?""#,
        r#""instructions":7"#,
    );
    assert_eq!(
        run(&numeric_instructions, CHOICE_RESPONSE),
        CaseResult::Rejected(DecisionRejection::InstructionsShape {
            question: "route".to_string(),
        })
    );

    let null_score_level = SCORE_REQUEST.replace(r#""Very angry""#, "null");
    assert_eq!(
        run(&null_score_level, SCORE_RESPONSE),
        CaseResult::Rejected(DecisionRejection::CriteriaValueShape {
            question: "frustration".to_string(),
        })
    );

    let null_choice_option = CHOICE_REQUEST.replace(r#""technical":"Bugs, outages, integrations""#, r#""technical":null"#);
    let request: DecisionRequest = serde_json::from_str(&null_choice_option).expect("a null option value is legal on the wire");
    assert!(
        validate_request(&request).is_ok(),
        "a null option value is the documented way to leave an option undescribed"
    );
}

#[test]
fn a_duplicate_json_key_is_rejected_where_it_can_reach_a_struct() {
    // serde_json keeps the last occurrence of a repeated key, so an untyped
    // `Value` cannot see a duplicate. Into `DecisionRequest` it errors, and
    // Beans builds the request in code, so a duplicate never reaches
    // `validate` either way.
    let body = r#"{"model":"jev-1.13.0","model":"jev-9.9.9","questions":{}}"#;
    let parsed: Value = serde_json::from_str(body).expect("parses");
    assert_eq!(parsed["model"], "jev-9.9.9", "an untyped parse is last-wins, not duplicate-safe");
    assert!(
        serde_json::from_str::<DecisionRequest>(body).is_err(),
        "the typed parse must reject the duplicate"
    );
}