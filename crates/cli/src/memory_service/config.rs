//! Portable account configuration; independent of inference credentials.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use super::types::*;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Record<T> { pub revision: Revision, pub value: Option<T>, #[serde(flatten)] pub extra: Extra }
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub schema_version: u32,
    #[serde(default)] pub clock: u64,
    #[serde(default)] pub connections: BTreeMap<String, Record<Connection>>,
    #[serde(default)] pub embeddings: BTreeMap<String, Record<EmbeddingProfile>>,
    #[serde(default)] pub bots: BTreeMap<String, Record<BotMemory>>,
    #[serde(flatten)] pub extra: Extra,
}
impl Default for MemoryConfig {
    fn default() -> Self { Self { schema_version: 1, clock: 0, connections: BTreeMap::new(),
        embeddings: BTreeMap::new(), bots: BTreeMap::new(), extra: Extra::new() } }
}

// Unknown JSON keys are recursively unioned. Known values always come from the winning
// logical revision. Unknown conflicting values have a deterministic lexical JSON winner.
fn preserve_unknown(winner: &mut Value, loser: &Value) {
    if let (Some(to), Some(from)) = (winner.as_object_mut(), loser.as_object()) {
        for (key, value) in from {
            match to.get_mut(key) {
                Some(current) if current.is_object() && value.is_object() => preserve_unknown(current, value),
                None => { to.insert(key.clone(), value.clone()); },
                _ => {},
            }
        }
    }
}
fn merge_extra(a: &mut Extra, b: &Extra) {
    for (key, incoming) in b {
        let entry = a.entry(key.clone()).or_insert_with(|| incoming.clone());
        if entry.is_object() && incoming.is_object() {
            let mut merged = if entry.to_string() >= incoming.to_string() { entry.clone() } else { incoming.clone() };
            preserve_unknown(&mut merged, if entry.to_string() >= incoming.to_string() { incoming } else { entry });
            *entry = merged;
        } else if incoming.to_string() > entry.to_string() { *entry = incoming.clone(); }
    }
}
fn record_extra(winner: &mut Extra, loser: &Extra, equal_revision: bool) {
    if equal_revision { merge_extra(winner,loser); }
    else { for (key,value) in loser { winner.entry(key.clone()).or_insert_with(||value.clone()); } }
}

trait ConfigValue: Clone + PartialEq {
    fn extra(&self) -> &Extra;
    fn extra_mut(&mut self) -> &mut Extra;
}
impl ConfigValue for Connection {
    fn extra(&self) -> &Extra { &self.extra }
    fn extra_mut(&mut self) -> &mut Extra { &mut self.extra }
}
impl ConfigValue for EmbeddingProfile {
    fn extra(&self) -> &Extra { &self.extra }
    fn extra_mut(&mut self) -> &mut Extra { &mut self.extra }
}
impl ConfigValue for BotMemory {
    fn extra(&self) -> &Extra { &self.extra }
    fn extra_mut(&mut self) -> &mut Extra { &mut self.extra }
}
fn merge_records<T: ConfigValue>(
    local: &mut BTreeMap<String, Record<T>>, remote: &BTreeMap<String, Record<T>>,
) -> Result<(), MemoryError> {
    for (id, incoming) in remote {
        match local.get_mut(id) {
            None => { local.insert(id.clone(), incoming.clone()); },
            Some(existing) => {
                if existing.revision == incoming.revision {
                    let mut a = existing.value.clone(); let mut b = incoming.value.clone();
                    if let Some(value) = &mut a { value.extra_mut().clear(); }
                    if let Some(value) = &mut b { value.extra_mut().clear(); }
                    if a != b { return Err(MemoryError::new("revision_conflict")); }
                }
                let (mut winner, loser) = if incoming.revision > existing.revision {
                    (incoming.clone(), existing.clone())
                } else { (existing.clone(), incoming.clone()) };
                let equal_revision=winner.revision==loser.revision;
                record_extra(&mut winner.extra, &loser.extra,equal_revision);
                if let (Some(value), Some(previous)) = (&mut winner.value, &loser.value) {
                    record_extra(value.extra_mut(), previous.extra(),equal_revision);
                }
                *existing = winner;
            }
        }
    }
    Ok(())
}
impl MemoryConfig {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema_version != 1 || self.connections.len() > 128 || self.bots.len() > 10000 || self.embeddings.len() > 128 {
            return Err(MemoryError::new("unsupported_config"));
        }
        let mut identities: BTreeMap<(&str, &str), &str> = BTreeMap::new();
        let mut user_keys: BTreeMap<&str, &str> = BTreeMap::new();
        for c in self.connections.values().filter_map(|r|r.value.as_ref()) {
            if let Some(BackendOptions::OpenViking { bindings }) = &c.options {
                for (namespace, binding) in bindings {
                    let pair = (binding.account_id.as_str(), binding.user_id.as_str());
                    if identities.insert(pair, namespace).is_some_and(|old|old!=namespace) {
                        return Err(MemoryError::new("openviking_identity_reused"));
                    }
                    if binding.mode == OpenVikingAuthMode::UserKey && !binding.api_key.is_empty()
                        && user_keys.insert(&binding.api_key, namespace).is_some_and(|old|old!=namespace) {
                        return Err(MemoryError::new("openviking_key_reused"));
                    }
                }
            }
        }
        for record in self.connections.values() {
            if let Some(c) = &record.value { validate_connection(c)?; }
        }
        for record in self.bots.values() {
            if let Some(b) = &record.value {
                b.recall_budget.validate()?;
                if b.max_capture_deliveries_per_turn > 4 || b.unattended_capture {
                    return Err(MemoryError::new("enforced_spending_policy_required"));
                }
            }
        }
        for record in self.embeddings.values() {
            if let Some(p) = &record.value {
                crate::embeddings::EmbeddingSpec::from_profile(p).map_err(|_| MemoryError::new("invalid_embedding_profile"))?;
            }
        }
        if serde_json::to_vec(self).map_err(|_| MemoryError::new("invalid_config"))?.len() > 1024 * 1024 {
            return Err(MemoryError::new("config_too_large"));
        }
        Ok(())
    }
    pub fn merge(&mut self, incoming: &Self) -> Result<bool, MemoryError> {
        incoming.validate()?;
        let mut next = self.clone();
        merge_records(&mut next.connections, &incoming.connections)?;
        merge_records(&mut next.embeddings, &incoming.embeddings)?;
        merge_records(&mut next.bots, &incoming.bots)?;
        merge_extra(&mut next.extra, &incoming.extra);
        next.clock = next.clock.max(incoming.clock).max(next.max_revision());
        next.validate()?;
        let changed = *self != next;
        *self = next; Ok(changed)
    }
    fn max_revision(&self) -> u64 {
        self.connections.values().map(|r| r.revision.counter)
            .chain(self.embeddings.values().map(|r| r.revision.counter))
            .chain(self.bots.values().map(|r| r.revision.counter)).max().unwrap_or(0)
    }
    pub fn next_revision(&mut self, device: &str) -> Result<Revision, MemoryError> {
        if device.is_empty() { return Err(MemoryError::new("identity_required")); }
        self.clock = self.clock.max(self.max_revision()).checked_add(1).ok_or_else(|| MemoryError::new("clock_exhausted"))?;
        Ok(Revision { counter: self.clock, device_id: device.into() })
    }
    pub fn set_connection(&mut self, id: &str, value: Option<Connection>, device: &str) -> Result<(), MemoryError> {
        valid_id(id)?;
        if let Some(c) = &value { validate_connection(c)?; }
        let revision = self.next_revision(device)?;
        let extra = self.connections.get(id).map(|r| r.extra.clone()).unwrap_or_default();
        self.connections.insert(id.into(), Record { revision, value, extra }); Ok(())
    }
    pub fn set_bot(&mut self, id: &str, mut value: BotMemory, device: &str) -> Result<(), MemoryError> {
        valid_id(id)?; value.recall_budget.validate()?;
        let revision = self.next_revision(device)?;
        value.consent_revision = revision.clone();
        let extra = self.bots.get(id).map(|r| r.extra.clone()).unwrap_or_default();
        self.bots.insert(id.into(), Record { revision, value: Some(value), extra }); Ok(())
    }
    pub fn touch_embedding_connections(&mut self, profile_id:&str, device:&str)->Result<(),MemoryError>{
        if !self.connections.values().any(|r|r.value.as_ref().is_some_and(|c|c.embedding_profile.as_deref()==Some(profile_id))) { return Ok(()); }
        let revision=self.next_revision(device)?;
        for record in self.connections.values_mut() {
            if record.value.as_ref().is_some_and(|c|c.embedding_profile.as_deref()==Some(profile_id)) { record.revision=revision.clone(); }
        }
        Ok(())
    }
    pub fn summary(&self) -> Value {
        json!({"schema_version":self.schema_version,
            "connections":self.connections.iter().filter_map(|(id, r)| r.value.as_ref().map(|c|
                json!({"id":id,"revision":r.revision,"backend":c.backend,"name":c.name,"has_secret":connection_has_secret(c),"embedding_profile":c.embedding_profile,
                    "availability":if cloud_blocked(c){"blocked"}else{"supported"},"reason":if cloud_blocked(c){Some("lancedb_cloud_transport_unavailable")}else{None}}))).collect::<Vec<_>>(),
            "embeddings":self.embeddings.iter().filter_map(|(id, r)| r.value.as_ref().map(|p|
                json!({"id":id,"revision":r.revision,"model":p.model,"model_revision":p.revision,"dimensions":p.dimensions,"has_secret":p.secret.is_some()}))).collect::<Vec<_>>(),
            "bots":self.bots.iter().filter_map(|(id, r)| r.value.as_ref().map(|b|
                json!({"bot_id":id,"preferences":preferences_summary(b)}))).collect::<Vec<_>>()})
    }
}
pub fn valid_id(id: &str) -> Result<(), MemoryError> {
    if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) { return Err(MemoryError::new("invalid_id")); }
    Ok(())
}
pub fn validate_connection(c: &Connection) -> Result<(), MemoryError> {
    if c.name.is_empty() || c.name.len() > 256 || c.secret.as_ref().is_some_and(|s| s.len() > 8192) {
        return Err(MemoryError::new("invalid_connection"));
    }
    if let Some(options) = &c.options {
        match (&c.backend, options) {
            (BackendKind::Hindsight, BackendOptions::Hindsight {}) => {},
            (BackendKind::OpenViking, BackendOptions::OpenViking { bindings }) => {
                for (namespace, binding) in bindings {
                    if namespace.len() != 64 || !namespace.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                        || binding.account_id.is_empty() || binding.user_id.is_empty()
                        || binding.api_key.len() > 8192
                        || ![&binding.account_id, &binding.user_id].iter().all(|s| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')) {
                        return Err(MemoryError::new("invalid_openviking_binding"));
                    }
                }
            },
            (BackendKind::Pgvector, BackendOptions::Pgvector { schema, role }) => {
                if role.as_ref().is_some_and(|s|s.is_empty()||s.len()>63||s.chars().any(char::is_control)) {
                    return Err(MemoryError::new("invalid_postgres_role"));
                }
                if schema.is_empty() || schema.len() > 63 || !schema.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    return Err(MemoryError::new("invalid_schema"));
                }
            },
            (BackendKind::LanceDb, BackendOptions::LanceDb { region }) => {
                if region.as_ref().is_some_and(|s| s.is_empty() || s.len() > 128) { return Err(MemoryError::new("invalid_region")); }
            },
            _ => return Err(MemoryError::new("backend_options_mismatch")),
        }
    }
    if let Some(endpoint) = &c.endpoint {
        let url = reqwest::Url::parse(endpoint).map_err(|_| MemoryError::new("invalid_endpoint"))?;
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err(MemoryError::new("unsafe_endpoint"));
        }
        match c.backend {
            BackendKind::Hindsight | BackendKind::OpenViking => {
                if url.scheme() != "https" && !(url.scheme() == "http" && c.allow_insecure_http) {
                    return Err(MemoryError::new("insecure_endpoint"));
                }
            },
            BackendKind::Pgvector if !matches!(url.scheme(), "postgres" | "postgresql") => return Err(MemoryError::new("invalid_endpoint")),
            BackendKind::LanceDb if url.scheme() != "db" => return Err(MemoryError::new("runner_local_binding_required")),
            _ => {},
        }
    }
    Ok(())
}

pub fn preferences_summary(b: &BotMemory) -> Value {
    json!({"connection_id":b.connection_id,"auto_recall":b.auto_recall,
        "capture_conversation":b.capture_conversation,"capture_group_text":b.capture_group_text,
        "unattended_capture":b.unattended_capture,"max_capture_deliveries_per_turn":b.max_capture_deliveries_per_turn,
        "consent_revision":b.consent_revision,"deletion_epoch":b.deletion_epoch,"recall_budget":b.recall_budget})
}

pub fn cloud_blocked(c:&Connection)->bool { c.backend==BackendKind::LanceDb&&c.endpoint.is_some() }
fn connection_has_secret(c:&Connection)->bool {
    c.secret.is_some()||matches!(&c.options,Some(BackendOptions::OpenViking{bindings})if bindings.values().any(|b|!b.api_key.is_empty()))
}
