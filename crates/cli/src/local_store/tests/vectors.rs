//! Semantic search storage at its edges: spaces per role, the queues and their version guards,
//! pictures arriving late, and backlogs larger than one batch.

use super::*;
use crate::local_store::vectors::{ChunkVector, Embedded, Quantized, Queue, Role};

fn unit(values: &[f32]) -> Quantized {
    Quantized::from_f32(values).unwrap()
}

fn picture_message(id: &str, at: f64, attachments: &[(&str, &str)]) -> Message {
    let mut message = message(id, at);
    message.body = Body::Text {
        text: format!("look at {id}"),
        attachments: attachments
            .iter()
            .map(|(id, mime)| crate::model::Attachment { id: (*id).into(), name: format!("{id}.png"), mime: (*mime).into(), size: 1, width: None, height: None })
            .collect(),
        mentions: vec![],
        reply_to: None,
    };
    message
}

fn pending_ids(store: &LocalStore, queue: Queue) -> Vec<String> {
    store.pending(queue, 100).unwrap().into_iter().map(|p| p.message.id).collect()
}

fn embedded(store: &LocalStore, queue: Queue, role: Role, space: &str, chunks: usize) -> usize {
    let pending = store.pending(queue, 100).unwrap();
    // Chunk numbers are unique per message across roles: pictures number theirs from a base of their own.
    let base = if role == Role::Image { crate::search_index::IMAGE_CHUNK_BASE } else { 0 };
    let work: Vec<Embedded> = pending
        .iter()
        .map(|p| Embedded { message_id: p.message.id.clone(), version: p.version, chunks: (0..chunks as u32).map(|chunk| ChunkVector { chunk: base + chunk, vector: unit(&[1.0, 0.0]) }).collect() })
        .collect();
    store.save_embeddings(role, space, &work).unwrap()
}

#[test]
fn quantization_refuses_what_it_cannot_represent() {
    assert!(Quantized::from_f32(&[]).is_none(), "nothing to scale");
    assert!(Quantized::from_f32(&[0.0; 4]).is_none());
    assert!(Quantized::from_f32(&[f32::INFINITY]).is_none());
    assert!(Quantized::from_f32(&[1.0, f32::NAN]).is_none());
    // So small its scale would not be a normal number: refused rather than stored and ignored.
    assert!(Quantized::from_f32(&[1e-40, -1e-40]).is_none());
    let large = Quantized::from_f32(&[f32::MAX, -f32::MAX, 0.0]).unwrap();
    assert_eq!(large.data, vec![127, -127, 0]);
    let negative = Quantized::from_f32(&[-0.5, -1.0]).unwrap();
    assert_eq!(negative.data, vec![-64, -127]);
    // With a scale of exactly 1, halves round away from zero, and no code leaves -127..=127.
    let ties = Quantized::from_f32(&[127.0, 0.5, -0.5, 126.5, -127.0]).unwrap();
    assert_eq!((ties.scale, ties.data), (1.0, vec![127, 1, -1, 127, -127]));
}

#[test]
fn unit_vectors_score_as_cosines_whatever_their_length() {
    let score = |a: &Quantized, b: &Quantized| a.data.iter().zip(&b.data).map(|(x, y)| i32::from(*x) * i32::from(*y)).sum::<i32>() as f32 * a.scale * b.scale;
    let short = Quantized::unit(&[3.0, 4.0]).unwrap();
    let long = Quantized::unit(&[300.0, 400.0]).unwrap();
    assert_eq!(short.data, Quantized::from_f32(&[3.0, 4.0]).unwrap().data, "the same codes, another scale");
    assert!((score(&short, &long) - 1.0).abs() < 0.01);
    let across = Quantized::unit(&[0.0, 7.0]).unwrap();
    assert!((score(&short, &across) - 0.8).abs() < 0.01);
    let huge = Quantized::unit(&[f32::MAX, f32::MAX]).unwrap();
    assert!((score(&huge, &huge) - 1.0).abs() < 0.01, "no overflow on the way");
    assert!(Quantized::unit(&[0.0, 0.0]).is_none());
    assert!(Quantized::unit(&[f32::NAN]).is_none());
}

#[test]
fn each_role_keeps_its_own_space() {
    let scratch = scratch();
    let store = &scratch.0;
    store.upsert(&picture_message("m", 1.0, &[("att-1", "image/png")])).unwrap();
    assert!(store.set_vector_space(Role::Text, Some("text-a")).unwrap());
    assert!(!store.set_vector_space(Role::Text, Some("text-a")).unwrap(), "the same space is no change");
    assert!(store.set_vector_space(Role::Image, Some("image-a")).unwrap());
    assert_eq!(embedded(store, Queue::Text, Role::Text, "text-a", 2), 1);
    assert_eq!(embedded(store, Queue::Images, Role::Image, "image-a", 1), 1);
    assert_eq!(store.embedding_progress(Role::Text).unwrap(), (1, 0));
    assert_eq!(store.embedding_progress(Role::Image).unwrap(), (1, 0));
    // Another text space drops only text vectors and queues only text again.
    assert!(store.set_vector_space(Role::Text, Some("text-b")).unwrap());
    assert_eq!(store.embedding_progress(Role::Text).unwrap(), (0, 1));
    assert_eq!(store.embedding_progress(Role::Image).unwrap(), (1, 0));
    // Work for a role's old space, or a space no role has, is refused.
    assert_eq!(embedded(store, Queue::Text, Role::Text, "text-a", 1), 0);
    assert_eq!(embedded(store, Queue::Text, Role::Text, "nobody", 1), 0);
    // Off: that role's vectors go, and nothing is queued for a role no one reads.
    assert!(store.set_vector_space(Role::Image, None).unwrap());
    assert_eq!(store.vector_space(Role::Image).unwrap(), None);
    assert_eq!(store.embedding_progress(Role::Image).unwrap(), (0, 0));
}

#[test]
fn saves_are_guarded_by_the_version_they_read() {
    let scratch = scratch();
    let store = &scratch.0;
    store.set_vector_space(Role::Text, Some("space")).unwrap();
    assert!(store.save_embeddings(Role::Text, "space", &[]).unwrap() == 0, "nothing to save is no error");
    let ghost = Embedded { message_id: "no-such-message".into(), version: 0, chunks: vec![] };
    assert_eq!(store.save_embeddings(Role::Text, "space", &[ghost]).unwrap(), 0);
    store.upsert(&message("m", 1.0)).unwrap();
    assert!(store.pending(Queue::Text, 0).unwrap().is_empty(), "a limit of none reads none");
    let read = store.pending(Queue::Text, 10).unwrap();
    store.upsert(&{
        let mut edited = message("m", 1.0);
        edited.body = Body::text("edited while embedding");
        edited
    })
    .unwrap();
    let stale = Embedded { message_id: "m".into(), version: read[0].version, chunks: vec![ChunkVector { chunk: 0, vector: unit(&[1.0]) }] };
    assert_eq!(store.save_embeddings(Role::Text, "space", &[stale]).unwrap(), 0);
    assert_eq!(pending_ids(store, Queue::Text), ["m"], "the edit is still waiting");
    // A message the model found nothing in still counts as done, so it is not read forever.
    let nothing = store.pending(Queue::Text, 10).unwrap();
    let empty = Embedded { message_id: "m".into(), version: nothing[0].version, chunks: vec![] };
    assert_eq!(store.save_embeddings(Role::Text, "space", &[empty]).unwrap(), 1);
    assert!(pending_ids(store, Queue::Text).is_empty());
}

#[test]
fn a_picture_that_arrives_later_is_read_again_and_only_its_messages() {
    let scratch = scratch();
    let store = &scratch.0;
    store.set_vector_space(Role::Image, Some("images")).unwrap();
    store.upsert(&picture_message("one", 1.0, &[("att-1", "image/png")])).unwrap();
    store.upsert(&picture_message("ten", 2.0, &[("att-10", "image/png"), ("att-2", "image/jpeg")])).unwrap();
    store.upsert(&picture_message("doc", 3.0, &[("att-1-doc", "application/pdf")])).unwrap();
    assert_eq!(pending_ids(store, Queue::Images), ["ten", "one"], "only messages with pictures, newest first");
    assert_eq!(embedded(store, Queue::Images, Role::Image, "images", 0), 2);
    assert!(pending_ids(store, Queue::Images).is_empty());
    // `att-1` arrives: its message is read again; `att-10` shares its prefix but is another file.
    assert_eq!(store.requeue_picture("att-1").unwrap(), 1);
    assert_eq!(pending_ids(store, Queue::Images), ["one"]);
    assert_eq!(pending_ids(store, Queue::Descriptions), ["ten", "one"], "descriptions still wait for both");
    assert_eq!(store.requeue_picture("att-2").unwrap(), 1, "a picture after the first in its message");
    assert_eq!(store.requeue_picture("att-1-doc").unwrap(), 0, "not a picture");
    assert_eq!(store.requeue_picture("").unwrap(), 0);
    // A pass that read `one` before the picture arrived cannot take it off the queue.
    let read = store.pending(Queue::Images, 10).unwrap();
    store.requeue_picture("att-1").unwrap();
    let late: Vec<Embedded> = read.iter().map(|p| Embedded { message_id: p.message.id.clone(), version: p.version, chunks: vec![] }).collect();
    assert_eq!(store.save_embeddings(Role::Image, "images", &late).unwrap(), 1, "only `ten`, read after its requeue");
    assert_eq!(pending_ids(store, Queue::Images), ["one"]);
}

#[test]
fn descriptions_keep_the_first_and_finish_only_when_told() {
    let scratch = scratch();
    let store = &scratch.0;
    store.upsert(&picture_message("m", 1.0, &[("att-1", "image/png"), ("att-2", "image/png")])).unwrap();
    let version = store.pending(Queue::Descriptions, 10).unwrap()[0].version;
    // A provider failed after one picture: keep it, and leave the message waiting.
    assert!(!store.save_descriptions("m", version, "vision", &[("att-1".into(), "a cat".into())], false).unwrap());
    assert_eq!(pending_ids(store, Queue::Descriptions), ["m"]);
    assert_eq!(store.describe_backlog().unwrap(), 1);
    // The retry describes the other; a second description of the first does not replace it.
    let both = [("att-1".to_string(), "a dog".to_string()), ("att-2".to_string(), "a boat".to_string())];
    assert!(store.save_descriptions("m", version, "vision", &both, true).unwrap());
    assert_eq!(store.descriptions("m").unwrap(), [("att-1".to_string(), "a cat".to_string()), ("att-2".to_string(), "a boat".to_string())]);
    assert!(pending_ids(store, Queue::Descriptions).is_empty());
    // Finishing with a stale version saves nothing: the pictures changed meanwhile.
    store.upsert(&picture_message("m", 1.0, &[("att-3", "image/png")])).unwrap();
    assert!(!store.save_descriptions("m", version, "vision", &[("att-3".into(), "late".into())], true).unwrap());
    assert!(!store.descriptions("m").unwrap().iter().any(|(id, _)| id == "att-3"));
    // Deleting the message deletes its descriptions; forgetting the account deletes them all.
    store.remove("chat", "m").unwrap();
    assert!(store.descriptions("m").unwrap().is_empty());
}

#[test]
fn a_backlog_larger_than_a_batch_is_derived_whole() {
    let scratch = scratch();
    let store = &scratch.0;
    for i in 0..1300 {
        store.upsert(&message(&format!("m{i:04}"), i as f64)).unwrap();
    }
    // As after an upgrade: every row stale, with text the store must derive again.
    store.connection.lock().unwrap().execute("UPDATE message_search SET stale = 1, text = ''", []).unwrap();
    store.set_vector_space(Role::Text, Some("space")).unwrap();
    assert_eq!(store.pending(Queue::Text, 2000).unwrap().len(), 1300, "three batches and more, all derived");
    let stale: i64 = store.connection.lock().unwrap().query_row("SELECT count(*) FROM message_search WHERE stale = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(stale, 0);
}

#[test]
fn the_copy_proposes_and_the_rows_confirm() {
    let scratch = scratch();
    let store = &scratch.0;
    store.set_vector_space(Role::Text, Some("space")).unwrap();
    for (id, at) in [("near", 1.0), ("far", 2.0)] {
        store.upsert(&message(id, at)).unwrap();
    }
    let pending = store.pending(Queue::Text, 10).unwrap();
    let work: Vec<Embedded> = pending
        .iter()
        .map(|p| {
            let vector = if p.message.id == "near" { unit(&[1.0, 0.0]) } else { unit(&[0.0, 1.0]) };
            Embedded { message_id: p.message.id.clone(), version: p.version, chunks: vec![ChunkVector { chunk: 0, vector }] }
        })
        .collect();
    store.save_embeddings(Role::Text, "space", &work).unwrap();
    let query = unit(&[1.0, 0.1]);
    assert!(store.nearest_vectors("space", 2, &query, 0).unwrap().is_empty(), "asking for none");
    let all = store.nearest_vectors("space", 2, &query, 10).unwrap();
    assert_eq!(all.iter().map(|h| h.message_id.as_str()).collect::<Vec<_>>(), ["near", "far"], "more asked than held");
    assert!(store.nearest_vectors("space", 3, &unit(&[1.0, 0.0, 0.0]), 10).unwrap().is_empty(), "another width is another space");
    // A candidate whose message changed is not confirmed, though the copy still proposes it.
    store.connection.lock().unwrap().execute("UPDATE message_search SET embed_pending = 1 WHERE message_id = 'near'", []).unwrap();
    assert_eq!(store.nearest_vectors("space", 2, &query, 1).unwrap()[0].message_id, "near");
    assert!(store.vector_hit(all[0].vector_id).unwrap().is_none());
    assert_eq!(store.vector_hit(all[1].vector_id).unwrap().unwrap().0.id, "far");
    // A deleted message's rows leave the copy on the next read.
    store.remove("chat", "far").unwrap();
    assert!(store.nearest_vectors("space", 2, &query, 10).unwrap().iter().all(|h| h.message_id != "far"));
}
