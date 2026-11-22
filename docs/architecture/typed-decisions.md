# Typed decisions

`crates/cli/src/decision.rs` is a pure request, response and capability validator for typed decisions: a model is asked a fixed set of keyed questions and must answer in a closed shape. It introduces no dispatch, provider call or authorization caller; code that asks a model for a decision validates the request before sending it and the response before using it. The tests live in `crates/cli/src/decision/tests.rs`.

## Requests

The current wire profile accepts pinned `jev-MAJOR.MINOR.PATCH` model ids and keyed Choice, Score and Noul questions. Choice accepts 1–255 options; Score accepts 2–10 ordered levels; Noul criteria are absent or exactly `false` and `true`. State and instructions accept text, objects or arrays. Choice option descriptions additionally accept null. These are this profile's structural rules, not verified capabilities of other models.

## Responses

Responses must name the requested model and return exactly the requested question ids and types. Deserialize directly from raw JSON: duplicate keys in `answers`, `probabilities` or `legend` reject the response, including identical entries and escaped spellings of the same key. Parsing through an untyped JSON value first loses this evidence. Probabilities and confidence are finite and within [0,1]. Probability sums have an inclusive 0.01 tolerance plus 1e-9 floating-point representation slack; validation never clamps or normalizes. Choice selects a maximum-probability option (ties allowed). Score matches the requested legend, stays within level indices and agrees with the probability-weighted position within the same tolerance. Noul returns a finite probability of yes.

## Capability preflight

`DecisionCapabilities` carries optional support flags for each question type and optional question, option and level limits. Missing or null facts remain unknown; false means unsupported and zero is a known limit. `preflight_request` validates structure first, rejects known unsupported types or exceeded limits without truncation, and returns `Unverified` whenever a relevant fact is unknown. `Verified` means only that these supplied facts passed this local preflight; it proves no provider availability or permission.
