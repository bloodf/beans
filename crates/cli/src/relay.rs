//! HTTP client for the relay. Signs identity-level requests, authenticates machines with the
//! challenge, and moves ciphertext.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::now_unix;
use crate::keys::{b64, Identity, Machine};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct RelayError {
    pub status: Option<u16>,
    pub message: String,
}

impl RelayError {
    pub fn is_unauthorized(&self) -> bool {
        self.status == Some(401)
    }
    /// The relay does not know this machine: a different relay (or a reset one) than the one
    /// that attested it.
    pub fn is_unknown_machine(&self) -> bool {
        self.status == Some(404)
    }
    /// This machine was unpaired from another Device. Its key is dead for good.
    pub fn is_unpaired(&self) -> bool {
        self.status == Some(410)
    }
    /// The relay no longer serves the protocol this build speaks. Trying again changes
    /// nothing; a newer Beans does.
    pub fn is_update_required(&self) -> bool {
        self.status == Some(426)
    }
    /// The relay refused the request, and the same request would be refused again.
    pub fn is_client_error(&self) -> bool {
        matches!(self.status, Some(400..=499)) && !self.is_rate_limited()
    }
    /// Over the relay's rate limit: the same request goes through after a wait.
    pub fn is_rate_limited(&self) -> bool {
        self.status == Some(429)
    }
}

impl From<reqwest::Error> for RelayError {
    fn from(error: reqwest::Error) -> Self {
        // reqwest's own words name the request ("error sending request for url (…)"); what went
        // wrong with it is the last cause in the chain ("Connection refused (os error 61)").
        let mut cause: &dyn std::error::Error = &error;
        while let Some(source) = cause.source() {
            cause = source;
        }
        RelayError { status: error.status().map(|s| s.as_u16()), message: format!("relay unreachable: {cause}") }
    }
}

pub type RelayResult<T> = Result<T, RelayError>;

/// One message of a chat as the relay pages it: where it sits in the log, and its first and
/// latest version (or its removal).
#[derive(Debug, Clone, Deserialize)]
pub struct GroupSlot {
    pub place: i64,
    pub blobs: Vec<BlobIn>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct BlobIn {
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub seq: i64,
    pub ciphertext: String,
    pub created_at: i64,
}

const POLICY_PAGE_BYTES: usize = 8 * 1024 * 1024;

fn invalid_policy_replay() -> RelayError {
    RelayError { status: None, message: "Invalid managed policy replay".into() }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyCut {
    schema_version: u8,
    context: String,
    cut_token: String,
    through: i64,
    next_since: i64,
    done: bool,
    blobs: Vec<PolicyCutBlob>,
}

#[derive(Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case", deny_unknown_fields)]
enum PolicyCutBlob {
    Ordinary { id: String, seq: i64, ciphertext: Vec<u8> },
    Managed { receipt: PolicyReceipt, policy_ciphertext: Vec<u8>, roster_ciphertext: Vec<u8> },
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PolicyReceipt {
    schema_version: u8,
    account_id: String,
    transaction_id: String,
    request_sha256: String,
    expected_slot_seq: i64,
    policy_id: String,
    policy_ciphertext_len: i64,
    policy_sha256: String,
    policy_seq: i64,
    roster_id: String,
    roster_ciphertext_len: i64,
    roster_sha256: String,
    roster_seq: i64,
    #[serde(deserialize_with = "required_policy_nullable")]
    checkpoint_transaction_id: Option<String>,
}

fn required_policy_nullable<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(decoder)
}

fn policy_identifier(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn policy_blob_id(value: &str) -> bool {
    policy_identifier(value, 64) && value != "." && value != ".."
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl PolicyReceipt {
    fn validate(&self, account: &str, policy: &[u8], roster: &[u8]) -> RelayResult<()> {
        use sha2::{Digest, Sha256};
        let commitment = serde_json::to_vec(&json!([
            "beans-managed-skill-commit", 1, self.account_id, self.transaction_id, self.expected_slot_seq,
            [self.policy_id, self.policy_ciphertext_len, self.policy_sha256, "managed_skill_policy", null, null, null],
            [self.roster_id, self.roster_ciphertext_len, self.roster_sha256, "roster", "roster", null, null],
            self.checkpoint_transaction_id
        ])).map_err(|_| invalid_policy_replay())?;
        if self.schema_version != 1 || self.account_id != account
            || !policy_identifier(&self.transaction_id, 1024)
            || self.checkpoint_transaction_id.as_deref().is_some_and(|id| !policy_identifier(id, 1024) || id == self.transaction_id)
            || !policy_blob_id(&self.policy_id) || !policy_blob_id(&self.roster_id) || self.policy_id == self.roster_id
            || self.expected_slot_seq < 0 || self.expected_slot_seq >= self.policy_seq
            || self.policy_seq <= 0 || self.policy_seq.checked_add(1) != Some(self.roster_seq)
            || policy.is_empty() || roster.is_empty()
            || (policy.len().saturating_mul(4).saturating_add(2) / 3)
                .saturating_add(roster.len().saturating_mul(4).saturating_add(2) / 3) > 32768
            || self.policy_ciphertext_len != policy.len() as i64 || self.roster_ciphertext_len != roster.len() as i64
            || self.policy_sha256 != format!("{:x}", Sha256::digest(policy))
            || self.roster_sha256 != format!("{:x}", Sha256::digest(roster))
            || self.request_sha256 != format!("{:x}", Sha256::digest(commitment))
        { return Err(invalid_policy_replay()); }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct MachineIn {
    pub machine_pubkey: String,
    pub box_pubkey: String,
    pub last_seen: i64,
    pub created_at: i64,
    /// The machine has a sync socket open on the relay.
    #[serde(default)]
    pub online: bool,
}

/// What the relay says over the sync socket. It carries no data: a Device pulls after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    /// The identity has a blob this machine may read.
    Blobs,
    /// The machine list or a machine's presence changed.
    Machines,
}

/// This machine's sync socket. It is online on the relay while this is open.
pub struct SyncSocket {
    stream: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    /// When the relay has to have said something, a ping of its own or the answer to a probe.
    due: tokio::time::Instant,
}

/// The relay pings every 25 s. A socket silent for this long is dead: the computer slept, the
/// phone changed networks.
const SOCKET_SILENCE: std::time::Duration = std::time::Duration::from_secs(70);
/// How long the relay has to answer a probe.
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

impl SyncSocket {
    /// The next signal. An error means the socket is gone and the caller connects again.
    pub async fn next(&mut self) -> RelayResult<Signal> {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        loop {
            let message = tokio::time::timeout_at(self.due, self.stream.next())
                .await
                .map_err(|_| RelayError { status: None, message: "the sync socket went silent".into() })?
                .ok_or_else(|| RelayError { status: None, message: "the relay closed the sync socket".into() })?
                .map_err(socket_error)?;
            self.due = tokio::time::Instant::now() + SOCKET_SILENCE;
            match message {
                Message::Text(text) => match serde_json::from_str::<Value>(&text).ok().as_ref().and_then(|v| v["type"].as_str()) {
                    Some("blobs") => return Ok(Signal::Blobs),
                    Some("machines") => return Ok(Signal::Machines),
                    _ => {}
                },
                // The pong has to be flushed by hand when nothing else is written.
                Message::Ping(_) => self.stream.flush().await.map_err(socket_error)?,
                Message::Close(_) => return Err(RelayError { status: None, message: "the relay closed the sync socket".into() }),
                _ => {}
            }
        }
    }

    /// Pings the relay, which has `PROBE_WAIT` to answer before `next` gives the socket up: a
    /// phone's may have died while the app was suspended, with nothing said on it since.
    pub async fn probe(&mut self) -> RelayResult<()> {
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        self.due = self.due.min(tokio::time::Instant::now() + PROBE_WAIT);
        tokio::time::timeout_at(self.due, self.stream.send(Message::Ping(Default::default())))
            .await
            .map_err(|_| RelayError { status: None, message: "the sync socket took no ping".into() })?
            .map_err(socket_error)
    }
}

fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> RelayError {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        // The upgrade was refused: 401 for a stale token, 410 for an unpaired machine.
        Error::Http(response) => RelayError { status: Some(response.status().as_u16()), message: format!("sync socket refused ({})", response.status()) },
        other => RelayError { status: None, message: format!("sync socket: {other}") },
    }
}

/// TLS for `wss://`: an upgrade is an HTTP/1.1 request, so it offers no ALPN.
fn tls() -> Arc<rustls::ClientConfig> {
    Arc::new(beans_tls::client_config(&[]))
}

/// The relay protocol this client speaks, sent as `Beans-Protocol` with every request. A
/// relay may refuse one it no longer serves with `426`. 1: group paging, `DELETE /v1/identity`.
/// 2: `POST /v1/machines`. 3: durable encrypted policy events. 4: appearance-preserving rosters.
pub const PROTOCOL: u32 = 5;
const MIN_ROSTER_PROTOCOL: u32 = 5;

#[derive(Deserialize)]
struct HealthEvidence<'a> {
    ok: bool,
    service: &'a str,
    format: crate::config::Format,
    protocol: u32,
    min_protocol: u32,
    min_roster_protocol: u32,
    memory_config_version: u32,
}

const FILE_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub struct RelayClient {
    /// Replaced by `reset_connections`, and its pooled connections with it.
    http: Mutex<reqwest::Client>,
    token: Mutex<Option<(String, i64)>>,
}

impl RelayClient {
    pub fn new() -> anyhow::Result<Self> {
        Ok(RelayClient { http: Mutex::new(Self::http_client()?), token: Mutex::new(None) })
    }

    /// The relay's own client: what it says about this build goes to the relay and never to
    /// a provider.
    fn http_client() -> anyhow::Result<reqwest::Client> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("beans-protocol", reqwest::header::HeaderValue::from(PROTOCOL));
        headers.insert("beans-format", reqwest::header::HeaderValue::from_static(crate::config::FORMAT));
        Ok(beans_tls::client_builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(format!("beans/{} ({})", crate::config::VERSION, std::env::consts::OS))
            .default_headers(headers)
            .build()?)
    }

    fn http(&self) -> reqwest::Client {
        self.http.lock().unwrap().clone()
    }

    /// Requests from here on open new connections. A phone's pooled ones may have died while
    /// the app was suspended, some without a word (a VPN on the phone keeps its end open), and
    /// a request sent on one of those waits out its whole timeout.
    pub fn reset_connections(&self) {
        match Self::http_client() {
            Ok(http) => *self.http.lock().unwrap() = http,
            Err(error) => tracing::warn!(%error, "building the relay client"),
        }
    }

    pub fn forget_token(&self) {
        *self.token.lock().unwrap() = None;
    }

    async fn check(response: reqwest::Response) -> RelayResult<Value> {
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let text = response.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let message = value["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("{status}: {text}"));
            return Err(RelayError { status: Some(status.as_u16()), message });
        }
        Ok(value)
    }

    /// Validates fresh format and enforced floors before account traffic.
    pub async fn health(&self, url: &str) -> RelayResult<u32> {
        let incompatible = || RelayError { status: Some(426), message:
            "Relay update required: Beans v2 format, protocol 5 and compatible enforced floors are required".into() };
        let response = self.http().get(format!("{url}/v1/health")).send().await?;
        if !response.status().is_success() {
            Self::check(response).await?;
            return Err(incompatible());
        }
        let text = response.text().await?;
        // Struct deserialization rejects duplicate compatibility fields.
        let health: HealthEvidence<'_> = serde_json::from_str(&text).map_err(|_| incompatible())?;
        if health.ok && health.service == "beans-relay" && health.format == crate::config::Format::BeansV2
            && health.protocol >= PROTOCOL && health.min_protocol == PROTOCOL
            && health.min_roster_protocol == MIN_ROSTER_PROTOCOL && health.memory_config_version == 1 {
            Ok(health.protocol)
        } else {
            Err(incompatible())
        }
    }

    /// Signed by the identity: registers the identity (idempotent) and attests one machine.
    pub async fn register(&self, url: &str, identity: &Identity, machine_pubkey: &str, box_pubkey: &str) -> RelayResult<()> {
        self.health(url).await?;
        let payload = json!({
            "identity_pubkey": identity.pubkey(),
            "content_pubkey": identity.content_pubkey(),
            "machine": { "machine_pubkey": machine_pubkey, "box_pubkey": box_pubkey },
            "ts": now_unix(),
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let body = json!({ "payload": b64(&bytes), "signature": identity.sign(&bytes) });
        Self::check(self.http().post(format!("{url}/v1/identities")).json(&body).send().await?).await?;
        Ok(())
    }

    /// The machine behind the bearer attests another for its identity: how a Device without
    /// the identity key pairs one. Protocol 2.
    pub async fn attest(&self, url: &str, token: &str, machine_pubkey: &str, box_pubkey: &str) -> RelayResult<()> {
        let body = json!({ "machine_pubkey": machine_pubkey, "box_pubkey": box_pubkey });
        Self::check(self.http().post(format!("{url}/v1/machines")).bearer_auth(token).json(&body).send().await?).await?;
        Ok(())
    }

    pub async fn authenticate(&self, url: &str, machine: &Machine) -> RelayResult<String> {
        self.health(url).await?;
        let challenge = Self::check(
            self.http()
                .post(format!("{url}/v1/auth/challenge"))
                .json(&json!({ "machine_pubkey": machine.pubkey() }))
                .send()
                .await?,
        )
        .await?;
        let nonce = challenge["nonce"].as_str().ok_or_else(|| RelayError { status: None, message: "no nonce".into() })?;
        let verified = Self::check(
            self.http()
                .post(format!("{url}/v1/auth/verify"))
                .json(&json!({
                    "machine_pubkey": machine.pubkey(),
                    "nonce": nonce,
                    "signature": machine.sign(nonce.as_bytes()),
                }))
                .send()
                .await?,
        )
        .await?;
        let token = verified["token"].as_str().ok_or_else(|| RelayError { status: None, message: "no token".into() })?.to_string();
        let expires_at = verified["expires_at"].as_i64().unwrap_or(now_unix() + 600);
        *self.token.lock().unwrap() = Some((token.clone(), expires_at));
        Ok(token)
    }

    pub async fn token(&self, url: &str, machine: &Machine) -> RelayResult<String> {
        if let Some((token, expires_at)) = self.token.lock().unwrap().clone() {
            if expires_at - 60 > now_unix() {
                return Ok(token);
            }
        }
        self.authenticate(url, machine).await
    }

    pub async fn put_blob(&self, url: &str, token: &str, item: crate::app::OutboxItem, expected_slot_seq: i64) -> RelayResult<i64> {
        if item.kind == "file" {
            let mut request = self.http().put(format!("{url}/v1/files/{}", item.id))
                .bearer_auth(token)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .timeout(FILE_TRANSFER_TIMEOUT);
            if let Some(group) = &item.group {
                request = request.query(&[("group", group)]);
            }
            let value = Self::check(request.body(item.ciphertext).send().await?).await?;
            return Ok(value["seq"].as_i64().unwrap_or(0));
        }
        let mut body = json!({ "id": item.id, "kind": item.kind, "recipient_machine_pubkey": item.recipient, "ciphertext": b64(&item.ciphertext) });
        if let Some(slot) = &item.slot {
            body["slot"] = json!(slot.name);
            body["keep_first"] = json!(slot.keep_first);
        }
        if item.kind == "roster" {
            body["expected_slot_seq"] = json!(expected_slot_seq);
        }
        if let Some(group) = &item.group {
            body["group"] = json!(group);
        }
        let value = Self::check(
            self.http()
                .put(format!("{url}/v1/blobs"))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await?,
        )
        .await?;
        Ok(value["seq"].as_i64().unwrap_or(0))
    }

    /// Opens this machine's sync socket: `ws(s)://<relay>/v1/sync` with the bearer token.
    pub async fn sync_socket(&self, url: &str, token: &str) -> RelayResult<SyncSocket> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let address = match url.split_once("://") {
            Some(("https", rest)) => format!("wss://{rest}/v1/sync"),
            Some((_, rest)) => format!("ws://{rest}/v1/sync"),
            None => format!("ws://{url}/v1/sync"),
        };
        let mut request = address.into_client_request().map_err(socket_error)?;
        let bearer = format!("Bearer {token}").parse().map_err(|_| RelayError { status: None, message: "token is not a header value".into() })?;
        request.headers_mut().insert("authorization", bearer);
        request.headers_mut().insert("beans-protocol", PROTOCOL.into());
        request.headers_mut().insert("beans-format", crate::config::FORMAT.parse().expect("Beans format header"));
        let connector = tokio_tungstenite::Connector::Rustls(tls());
        let connect = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
        let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(20), connect)
            .await
            .map_err(|_| RelayError { status: None, message: "the sync socket timed out connecting".into() })?
            .map_err(socket_error)?;
        Ok(SyncSocket { stream, due: tokio::time::Instant::now() + SOCKET_SILENCE })
    }

    pub async fn list_blobs(&self, url: &str, token: &str, since: i64, kinds: &str) -> RelayResult<(Vec<BlobIn>, i64)> {
        let value = Self::check(
            self.http()
                .get(format!("{url}/v1/blobs"))
                .bearer_auth(token)
                .query(&[("since", since.to_string()), ("kinds", kinds.to_string())])
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await?,
        )
        .await?;
        let blobs: Vec<BlobIn> = serde_json::from_value(value["blobs"].clone()).unwrap_or_default();
        Ok((blobs, value["seq"].as_i64().unwrap_or(since)))
    }

    /// Inspects opaque commitments only. Completion never supplies activation authority.
    pub(crate) async fn inspect_policy_replay(&self, url: &str, token: &str, account: &str) -> RelayResult<()> {
        let mut cursor = 0;
        let mut cut: Option<(String, String, i64)> = None;
        let mut blob_ids = std::collections::HashSet::new();
        let mut transactions = std::collections::HashSet::new();
        let mut bytes = 0usize;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        for _ in 0..65_537 {
            let mut query = vec![("policy_replay", "1".to_string()), ("since", cursor.to_string()), ("kinds", "policy".to_string())];
            if let Some((context, token, through)) = &cut {
                query.extend([("context", context.clone()), ("cut_token", token.clone()), ("through", through.to_string())]);
            }
            let page: PolicyCut = tokio::time::timeout_at(deadline, async {
                let response = self.http().get(format!("{url}/v1/blobs")).bearer_auth(token).query(&query)
                    .timeout(std::time::Duration::from_secs(60)).send().await?;
                Self::policy_response(response).await
            }).await.map_err(|_| invalid_policy_replay())??;
            if page.schema_version != 1 || !policy_identifier(&page.context, 1024)
                || !policy_identifier(&page.cut_token, 4096) || page.through < cursor
                || page.blobs.len() > 1 || (!page.done && page.blobs.is_empty())
                || cut.as_ref().is_some_and(|(context, _, through)| context != &page.context || *through != page.through)
            { return Err(invalid_policy_replay()); }
            let mut last = cursor;
            for blob in page.blobs {
                let (seq, end, size) = match blob {
                    PolicyCutBlob::Ordinary { id, seq, ciphertext } => {
                        if !policy_blob_id(&id) || !blob_ids.insert(id) || ciphertext.is_empty() || ciphertext.len() > 4 * 1024 * 1024 {
                            return Err(invalid_policy_replay());
                        }
                        (seq, seq, ciphertext.len())
                    }
                    PolicyCutBlob::Managed { receipt, policy_ciphertext, roster_ciphertext } => {
                        receipt.validate(account, &policy_ciphertext, &roster_ciphertext)?;
                        if !transactions.insert(receipt.transaction_id.clone()) || !blob_ids.insert(receipt.policy_id.clone())
                            || !blob_ids.insert(receipt.roster_id.clone()) { return Err(invalid_policy_replay()); }
                        let response = self.http().get(format!("{url}/v1/blobs")).bearer_auth(token)
                            .query(&[("policy_replay", "1"), ("receipt_transaction_id", receipt.transaction_id.as_str())])
                            .timeout(std::time::Duration::from_secs(60)).send();
                        let found: PolicyReceipt = tokio::time::timeout_at(deadline, async {
                            Self::policy_response(response.await?).await
                        }).await.map_err(|_| invalid_policy_replay())??;
                        if found != receipt { return Err(invalid_policy_replay()); }
                        (receipt.policy_seq, receipt.roster_seq, policy_ciphertext.len())
                    }
                };
                if seq <= last || end > page.through { return Err(invalid_policy_replay()); }
                last = end;
                bytes = bytes.checked_add(size).ok_or_else(invalid_policy_replay)?;
                if bytes > 1024 * 1024 * 1024 || blob_ids.len() > 1_048_576 { return Err(invalid_policy_replay()); }
            }
            if page.next_since != if page.done { page.through } else { last }
                || (!page.done && page.next_since <= cursor) { return Err(invalid_policy_replay()); }
            if page.done { return Ok(()); }
            cursor = page.next_since;
            cut = Some((page.context, page.cut_token, page.through));
        }
        Err(invalid_policy_replay())
    }

    async fn policy_response<T: serde::de::DeserializeOwned>(mut response: reqwest::Response) -> RelayResult<T> {
        let status = response.status();
        if !status.is_success() {
            return Err(RelayError { status: Some(status.as_u16()), message: "Managed policy replay refused".into() });
        }
        if response.content_length().is_some_and(|size| size > POLICY_PAGE_BYTES as u64) { return Err(invalid_policy_replay()); }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > POLICY_PAGE_BYTES - bytes.len() { return Err(invalid_policy_replay()); }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| invalid_policy_replay())
    }

    /// A chat backwards: the `limit` messages placed below `before` (the newest without it),
    /// oldest first, and whether older ones remain.
    pub async fn group_page(&self, url: &str, token: &str, group: &str, before: Option<i64>, limit: usize) -> RelayResult<(Vec<GroupSlot>, bool)> {
        let mut query = vec![("limit", limit.to_string())];
        if let Some(before) = before {
            query.push(("before", before.to_string()));
        }
        let value = Self::check(
            self.http().get(format!("{url}/v1/groups/{group}/blobs")).bearer_auth(token).query(&query).timeout(std::time::Duration::from_secs(60)).send().await?,
        )
        .await?;
        let slots = serde_json::from_value(value["slots"].clone()).map_err(|e| RelayError { status: None, message: format!("group page: {e}") })?;
        Ok((slots, value["has_more"].as_bool().unwrap_or(false)))
    }

    /// An attachment's encrypted bytes; `None` when it is no longer stored for this identity.
    pub async fn get_file(&self, url: &str, token: &str, id: &str) -> RelayResult<Option<Vec<u8>>> {
        let mut response = self.http().get(format!("{url}/v1/files/{id}")).bearer_auth(token).timeout(FILE_TRANSFER_TIMEOUT).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            Self::check(response).await?;
            unreachable!();
        }
        if response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) != Some("application/octet-stream") {
            return Err(RelayError { status: None, message: "Relay returned an invalid attachment content type".into() });
        }
        let max = crate::files::MAX_ATTACHMENT_BYTES as usize + crate::crypto::ENVELOPE_OVERHEAD;
        let too_large = || RelayError { status: None, message: "Relay attachment exceeds the file size limit".into() };
        if response.content_length().is_some_and(|size| size > max as u64) {
            return Err(too_large());
        }
        let mut bytes = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > max - bytes.len() {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }

    pub async fn delete_blob(&self, url: &str, token: &str, id: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/blobs/{id}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Deletes every blob of a group. Good to repeat.
    pub async fn delete_group(&self, url: &str, token: &str, group: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/groups/{group}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn machines(&self, url: &str, token: &str) -> RelayResult<(Vec<MachineIn>, i64)> {
        let value = Self::check(self.http().get(format!("{url}/v1/machines")).bearer_auth(token).send().await?).await?;
        let machines: Vec<MachineIn> = serde_json::from_value(value["machines"].clone()).unwrap_or_default();
        Ok((machines, value["now"].as_i64().unwrap_or(now_unix())))
    }

    // MARK: - Push

    /// Where this phone takes pushes: its APNs or FCM device token.
    pub async fn put_push_token(&self, url: &str, token: &str, platform: &str, device_token: &str, environment: Option<&str>) -> RelayResult<()> {
        Self::check(
            self.http()
                .put(format!("{url}/v1/push/token"))
                .bearer_auth(token)
                .json(&json!({ "platform": platform, "token": device_token, "environment": environment }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    pub async fn delete_push_token(&self, url: &str, token: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/push/token")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Asks the relay to push this ciphertext to the identity's phones; answers how many it queued.
    pub async fn push(&self, url: &str, token: &str, ciphertext_b64: &str) -> RelayResult<u64> {
        let value = Self::check(self.http().post(format!("{url}/v1/push")).bearer_auth(token).json(&json!({ "ciphertext": ciphertext_b64 })).send().await?).await?;
        Ok(value["queued"].as_u64().unwrap_or(0))
    }

    // MARK: - Pairing mailbox

    /// Unpairs a machine of this identity, this one included.
    pub async fn revoke_machine(&self, url: &str, token: &str, machine_pubkey: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/machines/{machine_pubkey}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Deletes the identity and everything the relay holds for it; every Device gets `410`.
    pub async fn delete_identity(&self, url: &str, token: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/identity")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn pair_create(&self, url: &str, token: &str) -> RelayResult<String> {
        let value = Self::check(self.http().post(format!("{url}/v1/pair")).bearer_auth(token).send().await?).await?;
        Ok(value["nonce"].as_str().unwrap_or_default().to_string())
    }

    pub async fn pair_post_request(&self, url: &str, nonce: &str, ciphertext: &[u8]) -> RelayResult<()> {
        Self::check(
            self.http()
                .post(format!("{url}/v1/pair/{nonce}/request"))
                .json(&json!({ "ciphertext": b64(ciphertext) }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    pub async fn pair_get_request(&self, url: &str, token: &str, nonce: &str) -> RelayResult<Option<Vec<u8>>> {
        let value = Self::check(self.http().get(format!("{url}/v1/pair/{nonce}/request")).bearer_auth(token).send().await?).await?;
        decode_optional(&value)
    }

    pub async fn pair_post_reply(&self, url: &str, token: &str, nonce: &str, ciphertext: &[u8]) -> RelayResult<()> {
        Self::check(
            self.http()
                .post(format!("{url}/v1/pair/{nonce}/reply"))
                .bearer_auth(token)
                .json(&json!({ "ciphertext": b64(ciphertext) }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    /// The identity retires a pairing: the mailbox goes and a Device polling it gets a 404.
    pub async fn pair_delete(&self, url: &str, token: &str, nonce: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/pair/{nonce}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn pair_get_reply(&self, url: &str, nonce: &str) -> RelayResult<Option<Vec<u8>>> {
        let value = Self::check(self.http().get(format!("{url}/v1/pair/{nonce}/reply")).send().await?).await?;
        decode_optional(&value)
    }
}

fn decode_optional(value: &Value) -> RelayResult<Option<Vec<u8>>> {
    match value["ciphertext"].as_str() {
        Some(text) => crate::keys::unb64(text)
            .map(Some)
            .map_err(|e| RelayError { status: None, message: format!("bad ciphertext: {e}") }),
        None => Ok(None),
    }
}
