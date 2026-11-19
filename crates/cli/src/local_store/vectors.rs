//! Semantic search storage (`crate::search_index`): int8 chunk vectors beside the messages they
//! come from, one vector space per role, and the queues of messages waiting for work.
//!
//! The SQLite rows are the truth. A per-space in-memory copy scans them exactly by brute force
//! (about 75 ms and 384 MB per million 384-dimension chunks on one core) and only proposes
//! candidates: it reads rows past the last id it holds and applies the deletion log, and every
//! candidate is checked against current rows before it is returned, so a stale copy costs
//! recall, never correctness.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use super::*;

/// What a vector space embeds: message text and attachment names (and image descriptions), or
/// images. Each role has its own space; a text query is embedded once per role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Text,
    Image,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Text => "text",
            Role::Image => "image",
        }
    }
}

/// Messages waiting for work: embedding their text, embedding their images, or describing
/// their images with a vision model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Queue {
    Text,
    Images,
    Descriptions,
}

/// A vector as int8 codes and one scale: `value[i] ≈ data[i] * scale`.
#[derive(Clone, Debug, PartialEq)]
pub struct Quantized {
    pub scale: f32,
    pub data: Vec<i8>,
}

impl Quantized {
    /// Symmetric per-vector quantization. `None` for a vector with a value that is not finite,
    /// or whose scale would not be a normal number (all zeros, or too small to divide by).
    pub fn from_f32(vector: &[f32]) -> Option<Quantized> {
        let max = vector.iter().try_fold(0f32, |max, v| v.is_finite().then(|| max.max(v.abs())))?;
        let scale = max / 127.0;
        if !scale.is_normal() {
            return None;
        }
        Some(Quantized { scale, data: vector.iter().map(|v| (v / scale).round().clamp(-127.0, 127.0) as i8).collect() })
    }

    /// `vector` scaled to unit length, then quantized, so the score of two is their cosine
    /// whatever the model returned: not every model or provider normalizes its output.
    pub fn unit(vector: &[f32]) -> Option<Quantized> {
        let mut quantized = Quantized::from_f32(vector)?;
        let norm = vector.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
        quantized.scale = (f64::from(quantized.scale) / norm) as f32;
        quantized.scale.is_normal().then_some(quantized)
    }

    fn from_row(scale: f64, blob: &[u8], dimensions: usize) -> Option<Quantized> {
        let scale = scale as f32;
        (blob.len() == dimensions && scale.is_finite() && scale > 0.0).then(|| Quantized { scale, data: blob.iter().map(|b| *b as i8).collect() })
    }

    fn blob(&self) -> Vec<u8> {
        self.data.iter().map(|v| *v as u8).collect()
    }
}

/// A message waiting in a queue, with the version the work must still match when it saves.
pub struct Pending {
    pub message: Message,
    pub version: i64,
    /// For the text queue: what vision models said the message's images show, by attachment.
    pub descriptions: Vec<(String, String)>,
}

/// One chunk of a message and its vector. Chunk numbers are the chunker's (`search_index`).
pub struct ChunkVector {
    pub chunk: u32,
    pub vector: Quantized,
}

/// What the indexer embedded for one `Pending` message.
pub struct Embedded {
    pub message_id: String,
    pub version: i64,
    pub chunks: Vec<ChunkVector>,
}

/// A candidate chunk, best first.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorHit {
    pub vector_id: i64,
    pub message_id: String,
    pub chunk: u32,
    pub score: f32,
}

/// The in-memory copies, one per active space.
#[derive(Default)]
pub struct VectorCaches(HashMap<String, VectorCache>);

/// The in-memory copy of one space's vectors. A row whose scale is 0 is deleted.
#[derive(Default)]
struct VectorCache {
    dimensions: usize,
    max_id: i64,
    deletion_seq: i64,
    ids: Vec<i64>,
    message_ids: Vec<String>,
    chunks: Vec<u32>,
    scales: Vec<f32>,
    data: Vec<i8>,
    slots: HashMap<i64, usize>,
    dead: usize,
}

impl VectorCache {
    fn new(connection: &Connection, dimensions: usize) -> anyhow::Result<VectorCache> {
        // Earlier deletions concern rows this fresh copy never reads.
        let deletion_seq = connection.query_row(sql!("vectors/last_deletion"), [], |r| r.get(0))?;
        Ok(VectorCache { dimensions, deletion_seq, ..Default::default() })
    }

    #[cfg(test)]
    fn live(&self) -> usize {
        self.ids.len() - self.dead
    }

    fn push(&mut self, id: i64, message_id: String, chunk: u32, vector: Quantized) {
        self.slots.insert(id, self.ids.len());
        self.ids.push(id);
        self.message_ids.push(message_id);
        self.chunks.push(chunk);
        self.scales.push(vector.scale);
        self.data.extend_from_slice(&vector.data);
    }

    /// Brings the copy up to the rows of `space`: deletions first, then rows past `max_id`.
    fn sync(&mut self, connection: &Connection, space: &str) -> anyhow::Result<()> {
        let mut deletions = connection.prepare_cached(sql!("vectors/deletions_since"))?;
        let mut rows = deletions.query([self.deletion_seq])?;
        while let Some(row) = rows.next()? {
            self.deletion_seq = row.get(0)?;
            if let Some(slot) = self.slots.remove(&row.get::<_, i64>(1)?) {
                self.scales[slot] = 0.0;
                self.dead += 1;
            }
        }
        let mut added = connection.prepare_cached(sql!("vectors/rows_since"))?;
        let mut rows = added.query(params![self.max_id, space])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            self.max_id = id;
            if let Some(vector) = Quantized::from_row(row.get(3)?, row.get_ref(4)?.as_blob()?, self.dimensions) {
                self.push(id, row.get(1)?, row.get(2)?, vector);
            }
        }
        if self.dead > 1024 && self.dead * 4 > self.ids.len() {
            self.compact();
        }
        Ok(())
    }

    fn compact(&mut self) {
        let mut next = VectorCache { dimensions: self.dimensions, max_id: self.max_id, deletion_seq: self.deletion_seq, ..Default::default() };
        for slot in (0..self.ids.len()).filter(|&slot| self.scales[slot] != 0.0) {
            let data = self.data[slot * self.dimensions..(slot + 1) * self.dimensions].to_vec();
            let message_id = std::mem::take(&mut self.message_ids[slot]);
            next.push(self.ids[slot], message_id, self.chunks[slot], Quantized { scale: self.scales[slot], data });
        }
        *self = next;
    }

    /// The `k` live rows with the highest dot product with `query`, best first.
    fn nearest(&self, query: &Quantized, k: usize) -> Vec<VectorHit> {
        /// A kept row, ordered worst first so the heap's top is the one to drop.
        #[derive(PartialEq)]
        struct Kept(f32, usize);
        impl Eq for Kept {}
        impl PartialOrd for Kept {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Kept {
            fn cmp(&self, other: &Self) -> Ordering {
                other.0.total_cmp(&self.0).then(self.1.cmp(&other.1))
            }
        }
        if k == 0 || query.data.len() != self.dimensions {
            return Vec::new();
        }
        let mut kept = BinaryHeap::with_capacity(k + 1);
        for (slot, row) in self.data.chunks_exact(self.dimensions).enumerate() {
            if self.scales[slot] == 0.0 {
                continue;
            }
            let dot: i32 = row.iter().zip(&query.data).map(|(a, b)| *a as i32 * *b as i32).sum();
            let score = dot as f32 * self.scales[slot] * query.scale;
            if kept.len() < k {
                kept.push(Kept(score, slot));
            } else if kept.peek().is_some_and(|worst: &Kept| score > worst.0) {
                kept.pop();
                kept.push(Kept(score, slot));
            }
        }
        let mut hits: Vec<VectorHit> = kept
            .into_iter()
            .map(|Kept(score, slot)| VectorHit { vector_id: self.ids[slot], message_id: self.message_ids[slot].clone(), chunk: self.chunks[slot], score })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.vector_id.cmp(&b.vector_id)));
        hits
    }
}

/// The role of a queue's vectors, and its SQL.
fn queue_sql(queue: Queue) -> (&'static str, &'static str) {
    match queue {
        Queue::Text => (sql!("vectors/queue_text"), sql!("vectors/finish_text")),
        Queue::Images => (sql!("vectors/queue_images"), sql!("vectors/finish_images")),
        Queue::Descriptions => (sql!("vectors/queue_descriptions"), sql!("vectors/finish_descriptions")),
    }
}

impl LocalStore {
    /// The space `role` vectors are kept for now, if any.
    pub fn vector_space(&self, role: Role) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        Ok(connection.query_row(sql!("vectors/space"), [role.as_str()], |r| r.get(0)).optional()?)
    }

    /// Keeps `role` vectors for `space` from now on; `None` keeps none. A change drops that
    /// role's vectors and queues every message again. Returns whether the space changed.
    pub fn set_vector_space(&self, role: Role, space: Option<&str>) -> anyhow::Result<bool> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: Option<String> = tx.query_row(sql!("vectors/space"), [role.as_str()], |r| r.get(0)).optional()?;
        if current.as_deref() == space {
            return Ok(false);
        }
        tx.execute(sql!("vectors/drop_role"), [role.as_str()])?;
        match space {
            Some(space) => {
                tx.execute(if role == Role::Text { sql!("vectors/requeue_text") } else { sql!("vectors/requeue_images") }, [])?;
                tx.execute(sql!("vectors/set_space"), params![role.as_str(), space])?
            }
            None => tx.execute(sql!("vectors/clear_space"), [role.as_str()])?,
        };
        tx.commit()?;
        Ok(true)
    }

    /// Up to `limit` finished messages waiting in `queue`, newest first. Only messages with
    /// something for the queue are ever in it.
    pub fn pending(&self, queue: Queue, limit: usize) -> anyhow::Result<Vec<Pending>> {
        self.refresh_search()?;
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(queue_sql(queue).0)?;
        let rows: Vec<(String, i64, String)> = statement.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut descriptions = connection.prepare_cached(sql!("vectors/descriptions_of"))?;
        rows.into_iter()
            .map(|(id, version, json)| {
                let descriptions = match queue {
                    Queue::Text => descriptions.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?,
                    _ => Vec::new(),
                };
                Ok(Pending { message: serde_json::from_str(&json).context("decoding a queued message")?, version, descriptions })
            })
            .collect()
    }

    /// Stores `role` vectors for `space`, for each message still at the version it was read
    /// at. Returns how many messages were stored; none when the space changed meanwhile.
    pub fn save_embeddings(&self, role: Role, space: &str, embedded: &[Embedded]) -> anyhow::Result<usize> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: Option<String> = tx.query_row(sql!("vectors/space"), [role.as_str()], |r| r.get(0)).optional()?;
        if current.as_deref() != Some(space) {
            return Ok(0);
        }
        let mut saved = 0;
        {
            let mut finish = tx.prepare(queue_sql(if role == Role::Text { Queue::Text } else { Queue::Images }).1)?;
            let mut clear = tx.prepare(sql!("vectors/drop_message_role"))?;
            let mut insert = tx.prepare(sql!("vectors/insert"))?;
            for message in embedded {
                if finish.execute(params![message.message_id, message.version])? != 1 {
                    continue;
                }
                clear.execute(params![message.message_id, role.as_str()])?;
                for chunk in &message.chunks {
                    insert.execute(params![message.message_id, role.as_str(), chunk.chunk, space, chunk.vector.scale as f64, chunk.vector.blob()])?;
                }
                saved += 1;
            }
        }
        tx.commit()?;
        Ok(saved)
    }

    /// Keeps what `model` said each image shows (the text queue picks the message up), and when
    /// `finished`, takes the message off the describe queue if its images are still the ones
    /// described (`version`). Returns whether it was taken off.
    pub fn save_descriptions(&self, message_id: &str, version: i64, model: &str, descriptions: &[(String, String)], finished: bool) -> anyhow::Result<bool> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let taken = finished && tx.execute(queue_sql(Queue::Descriptions).1, params![message_id, version])? == 1;
        if finished && !taken {
            return Ok(false);
        }
        for (attachment_id, text) in descriptions {
            tx.execute(sql!("vectors/add_description"), params![message_id, attachment_id, model, text])?;
        }
        tx.commit()?;
        Ok(taken)
    }

    /// A picture's bytes arrived on this Runner (fetched for a turn, after indexing skipped it):
    /// the messages showing it are read again. Returns how many.
    pub fn requeue_picture(&self, attachment_id: &str) -> anyhow::Result<usize> {
        Ok(self.connection.lock().unwrap().execute(sql!("vectors/requeue_picture"), [attachment_id])?)
    }

    /// What vision models said a message's images show, by attachment.
    pub fn descriptions(&self, message_id: &str) -> anyhow::Result<Vec<(String, String)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare_cached(sql!("vectors/descriptions_of"))?;
        let descriptions = statement.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(descriptions)
    }

    /// Messages whose images wait for a description.
    pub fn describe_backlog(&self) -> anyhow::Result<u64> {
        let connection = self.connection.lock().unwrap();
        Ok(connection.query_row(sql!("vectors/describe_backlog"), [], |r| r.get::<_, i64>(0))? as u64)
    }

    /// Messages with `role` vectors, and messages waiting for them.
    pub fn embedding_progress(&self, role: Role) -> anyhow::Result<(u64, u64)> {
        let connection = self.connection.lock().unwrap();
        Ok(connection.query_row(sql!("vectors/progress"), [role.as_str()], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)))?)
    }

    /// The `k` stored chunks of `space` nearest `query`, best first: candidates only, for
    /// `vector_hit` to confirm.
    pub fn nearest_vectors(&self, space: &str, dimensions: usize, query: &Quantized, k: usize) -> anyhow::Result<Vec<VectorHit>> {
        // Changed messages drop their vectors as their projection is derived again.
        self.refresh_search()?;
        // Lock order: the copies, then the connection, released before the scan.
        let mut caches = self.vectors.lock().unwrap();
        {
            let connection = self.connection.lock().unwrap();
            let active: Vec<String> = connection.prepare_cached(sql!("vectors/active_spaces"))?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            caches.0.retain(|kept, _| active.contains(kept));
            if caches.0.get(space).is_none_or(|cache| cache.dimensions != dimensions) {
                caches.0.insert(space.to_string(), VectorCache::new(&connection, dimensions)?);
            }
            for (space, cache) in caches.0.iter_mut() {
                cache.sync(&connection, space)?;
            }
            // Only these copies read the log; a second store on the same file (tests) loses
            // recall of deletions, which `vector_hit` covers.
            if let Some(applied) = caches.0.values().map(|cache| cache.deletion_seq).min() {
                connection.execute(sql!("vectors/prune_deletions"), [applied])?;
            }
        }
        Ok(caches.0[space].nearest(query, k))
    }

    /// The message and chunk of a candidate, if the chunk is still stored and its message is
    /// current and not waiting to be embedded again.
    pub fn vector_hit(&self, vector_id: i64) -> anyhow::Result<Option<(Message, u32)>> {
        let connection = self.connection.lock().unwrap();
        let row: Option<(String, u32)> = connection.query_row(sql!("vectors/hit"), [vector_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        row.map(|(json, chunk)| Ok((serde_json::from_str(&json).context("decoding search result")?, chunk))).transpose()
    }

    /// How many rows the in-memory copies hold, live, for tests.
    #[cfg(test)]
    pub(crate) fn cached_vectors(&self) -> usize {
        self.vectors.lock().unwrap().0.values().map(VectorCache::live).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_keeps_direction_and_refuses_empty_vectors() {
        let q = Quantized::from_f32(&[0.5, -1.0, 0.25, 0.0]).unwrap();
        assert_eq!(q.data, vec![64, -127, 32, 0]);
        assert!((q.scale - 1.0 / 127.0).abs() < 1e-7);
        assert!(Quantized::from_f32(&[0.0, 0.0]).is_none());
        assert!(Quantized::from_f32(&[f32::NAN, 1.0]).is_none());
        assert_eq!(Quantized::from_row(q.scale as f64, &q.blob(), 4).unwrap(), q);
        assert!(Quantized::from_row(q.scale as f64, &q.blob(), 5).is_none());
    }

    #[test]
    fn the_copy_ranks_exactly_and_follows_deletions() {
        let mut cache = VectorCache { dimensions: 2, ..Default::default() };
        for (id, v) in [(1, [1.0, 0.0]), (2, [0.7, 0.7]), (3, [0.0, 1.0]), (4, [-1.0, 0.0])] {
            cache.push(id, format!("m{id}"), 0, Quantized::from_f32(&v).unwrap());
        }
        let query = Quantized::from_f32(&[1.0, 0.1]).unwrap();
        let order = |cache: &VectorCache, k| cache.nearest(&query, k).into_iter().map(|h| h.vector_id).collect::<Vec<_>>();
        assert_eq!(order(&cache, 4), [1, 2, 3, 4]);
        assert_eq!(order(&cache, 2), [1, 2]);
        let slot = cache.slots.remove(&1).unwrap();
        cache.scales[slot] = 0.0;
        cache.dead += 1;
        assert_eq!(order(&cache, 2), [2, 3]);
        cache.compact();
        assert_eq!((cache.ids.len(), cache.live()), (3, 3));
        assert_eq!(order(&cache, 3), [2, 3, 4]);
        assert!(cache.nearest(&Quantized::from_f32(&[1.0, 0.0, 0.0]).unwrap(), 3).is_empty());
    }
}
