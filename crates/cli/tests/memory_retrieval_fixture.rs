// Synthetic corpus/scorer contract only; this target does not invoke retrieval backends.
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq)]
struct Outcome<'a> {
    ordered_ids: Vec<&'a str>,
    relevant_rank: Option<usize>,
    hit_at_1: bool,
    hit_at_5: bool,
}

fn score<'a>(
    relevant_id: &str,
    returned: Result<&[&'a str], &'a str>,
) -> Result<Outcome<'a>, &'a str> {
    let ids = returned?;
    let rank = ids.iter().position(|id| *id == relevant_id).map(|i| i + 1);
    Ok(Outcome {
        ordered_ids: ids.to_vec(),
        relevant_rank: rank,
        hit_at_1: rank == Some(1),
        hit_at_5: rank.is_some_and(|r| r <= 5),
    })
}

// Kept independent of corpus orders and labels: these are scorer boundary inputs.
#[test]
fn scorer_distinguishes_rank_five_six_absent_and_service_error() {
    let fifth = score("answer", Ok(&["a", "b", "c", "d", "answer", "e"])).unwrap();
    assert_eq!(fifth.relevant_rank, Some(5));
    assert!(!fifth.hit_at_1);
    assert!(fifth.hit_at_5);
    let sixth = score("answer", Ok(&["a", "b", "c", "d", "e", "answer"])).unwrap();
    assert_eq!(sixth.relevant_rank, Some(6));
    assert!(!sixth.hit_at_5);
    let absent = score("answer", Ok(&["a", "b", "c", "d", "e", "f"])).unwrap();
    assert_eq!(absent.relevant_rank, None);
    assert!(!absent.hit_at_1 && !absent.hit_at_5);
    assert_eq!(
        score("answer", Err("service unavailable")),
        Err("service unavailable")
    );
    let empty = score("answer", Ok(&[])).unwrap();
    assert_eq!(empty.relevant_rank, None);
    assert!(!empty.hit_at_5);
    assert!(score("answer", Ok(&["answer"])).unwrap().hit_at_1);
}

mod corpus {
    use super::*;
    use serde_json::Value;

    fn rows<'a>(data: &'a Value, key: &str) -> &'a [Value] {
        data[key].as_array().unwrap().as_slice()
    }
    fn text<'a>(row: &'a Value, key: &str) -> &'a str {
        row[key].as_str().unwrap()
    }
    fn vector(row: &Value) -> Vec<f64> {
        let values: Vec<_> = row["vector"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|v| v.is_finite()));
        assert!(values.iter().any(|v| *v != 0.0));
        values
    }
    fn cosine_distance(a: &[f64], b: &[f64]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm = |v: &[f64]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
        1.0 - dot / (norm(a) * norm(b))
    }
    fn ids(row: &Value) -> Vec<&str> {
        row["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect()
    }
    fn run<'a>(data: &'a Value, key: &str) -> (Vec<(&'a str, Outcome<'a>)>, [usize; 3]) {
        let mut outcomes = Vec::new();
        let mut table = [0, 0, 0]; // hit@1 count, hit@5 count, successful queries
        for order in rows(data, key) {
            let query = text(order, "query_id");
            let label = rows(data, "relevance_labels")
                .iter()
                .find(|label| text(label, "query_id") == query)
                .unwrap();
            let outcome = score(text(label, "relevant_id"), Ok(&ids(order))).unwrap();
            table[0] += usize::from(outcome.hit_at_1);
            table[1] += usize::from(outcome.hit_at_5);
            table[2] += 1;
            outcomes.push((query, outcome));
        }
        (outcomes, table)
    }

    #[test]
    fn corpus_references_scope_and_untied_vectors_are_consistent() {
        let data: Value =
            serde_json::from_str(include_str!("fixtures/memory_retrieval.json")).unwrap();
        let documents = rows(&data, "documents");
        let queries = rows(&data, "queries");
        let labels = rows(&data, "relevance_labels");
        let unique = |rows: &[Value], key: &str| {
            let mut seen = BTreeSet::new();
            for row in rows {
                assert!(seen.insert(text(row, key)), "duplicate {key}");
            }
        };
        unique(documents, "id");
        unique(queries, "id");
        unique(labels, "query_id");
        let query_ids: BTreeSet<_> = queries.iter().map(|q| text(q, "id")).collect();
        assert_eq!(
            query_ids,
            labels.iter().map(|l| text(l, "query_id")).collect()
        );
        let scopes = rows(&data, "scopes");
        unique(scopes, "id");
        let mut timestamps = BTreeSet::new();
        for doc in documents {
            let scope = scopes
                .iter()
                .find(|s| text(s, "id") == text(doc, "scope"))
                .unwrap();
            assert!(
                scope["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m == text(scope, "bot_id"))
            );
            let stamp = text(doc, "timestamp");
            assert!(stamp.starts_with("2026-01-") && stamp.ends_with("T12:00:00Z"));
            assert!(timestamps.insert(stamp));
            assert!(!text(doc, "text").is_empty());
            vector(doc);
        }
        for label in labels {
            let doc = documents
                .iter()
                .find(|d| text(d, "id") == text(label, "relevant_id"))
                .unwrap();
            assert_eq!(text(doc, "scope"), "own");
            assert!(!text(label, "reason").is_empty());
        }
        let foreign = documents
            .iter()
            .find(|d| text(d, "id") == "foreign-canary")
            .unwrap();
        assert_eq!(text(foreign, "scope"), "foreign-private");
        assert!(text(foreign, "text").contains("FOREIGN_BOT_CANARY"));
        let private = scopes
            .iter()
            .find(|s| text(s, "id") == "foreign-private")
            .unwrap();
        assert!(
            !private["members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m == "atlas")
        );
        let shared = documents
            .iter()
            .find(|d| text(d, "id") == "shared-speaker")
            .unwrap();
        assert_eq!(text(shared, "scope"), "own");
        let own = scopes.iter().find(|s| text(s, "id") == "own").unwrap();
        assert!(
            own["members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m == text(shared, "speaker"))
        );
        assert_ne!(text(private, "chat_id"), text(own, "chat_id"));
        assert_ne!(text(private, "bot_id"), text(own, "bot_id"));

        for key in ["vector_orders", "scripted_remote_orders"] {
            let orders = rows(&data, key);
            unique(orders, "query_id");
            assert_eq!(
                query_ids,
                orders.iter().map(|o| text(o, "query_id")).collect()
            );
            for order in orders {
                let ranked = ids(order);
                assert_eq!(ranked.len(), ranked.iter().collect::<BTreeSet<_>>().len());
                for id in ranked {
                    let doc = documents.iter().find(|d| text(d, "id") == id).unwrap();
                    assert_eq!(text(doc, "scope"), "own");
                }
            }
        }
        for query in queries {
            let qv = vector(query);
            assert!(!text(query, "text").is_empty());
            let mut distances: Vec<_> = documents
                .iter()
                .map(|d| (text(d, "id"), cosine_distance(&qv, &vector(d))))
                .collect();
            distances.sort_by(|a, b| a.1.total_cmp(&b.1));
            for pair in distances.windows(2) {
                assert!(
                    pair[1].1 - pair[0].1 > 1e-5,
                    "near-tied distances for {}",
                    text(query, "id")
                );
            }
            assert_eq!(
                distances[0].0, "foreign-canary",
                "canary must challenge scope filtering"
            );
            let expected = rows(&data, "vector_orders")
                .iter()
                .find(|o| text(o, "query_id") == text(query, "id"))
                .unwrap();
            let ranked: Vec<_> = distances
                .iter()
                .filter(|(id, _)| *id != "foreign-canary")
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(ranked, ids(expected));
        }
        let attempt = &data["foreign_attempt"];
        assert!(query_ids.contains(text(attempt, "query_id")));
        for id in ids(attempt) {
            assert!(documents.iter().any(|d| text(d, "id") == id));
        }
        assert!(ids(attempt).contains(&"foreign-canary"));
        assert_eq!(text(attempt, "expected"), "reject_foreign_scope");
    }

    #[test]
    fn declared_orders_repeat_with_independent_hit_tables() {
        let data: Value =
            serde_json::from_str(include_str!("fixtures/memory_retrieval.json")).unwrap();
        for (key, expected) in [
            ("vector_orders", [1, 5, 6]),
            ("scripted_remote_orders", [1, 4, 6]),
        ] {
            let first = run(&data, key);
            let second = run(&data, key);
            assert_eq!(first, second); // Includes query order and every ordered returned ID.
            assert_eq!(first.1, expected);
            println!(
                "synthetic scorer only: {key}: {:?}; counts {:?}",
                first.0, first.1
            );
        }
    }
}
