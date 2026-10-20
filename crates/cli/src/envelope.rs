//! Versioned, authenticated Device transport and durable local delivery custody.
use crate::{app::{App, OutboxItem}, keys::{b64, unb64}, model::{Device, Job, JobCancel, JobResult, Request, Response}, local_store::LocalStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

const DOMAIN: &[u8] = b"beans/envelope/v1\0";
const MAX_BYTES: usize = 4 * 1024 * 1024;
pub fn addressed(kind: &str) -> bool { matches!(kind, "job" | "job_cancel" | "job_result" | "request" | "response") }

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signed { envelope_version: u8, envelope: String, signature: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    envelope_version: u8, pub identity: String, pub from: String, pub to: String,
    pub kind: String, pub id: String, at: i64, pub body: Value,
}
#[derive(Debug, Clone)]
pub(crate) struct Opened { pub envelope: Envelope, pub bytes: Vec<u8> }
#[derive(Debug)]
pub(crate) enum Refusal { Legacy, Unsupported, Deferred, Rejected }
impl Refusal {
    fn code(&self) -> &'static str { match self {
        Self::Legacy => "transport_legacy_inbound_quarantined", Self::Unsupported => "transport_unsupported_version",
        Self::Deferred => "transport_sender_evidence_pending", Self::Rejected => "transport_rejected",
    } }
}

pub(crate) fn prepare<T: Serialize>(app: &App, kind: &str, recipient: &Device, body: &T) -> anyhow::Result<OutboxItem> {
    anyhow::ensure!(addressed(kind), "Invalid addressed kind");
    anyhow::ensure!(recipient.envelope_version == Some(1), "transport_peer_upgrade_required");
    let file = app.machine_file().ok_or_else(|| anyhow::anyhow!("Pair this Device first"))?;
    let machine = file.machine()?;
    let envelope = Envelope { envelope_version: 1, identity: file.identity_pubkey, from: machine.pubkey(),
        to: recipient.id.clone(), kind: kind.into(), id: uuid::Uuid::new_v4().to_string(),
        at: crate::config::now_unix(), body: serde_json::to_value(body)? };
    let bytes = serde_json::to_vec(&envelope)?;
    anyhow::ensure!(bytes.len() <= MAX_BYTES / 2, "Transport payload exceeds limit");
    let signed = Signed { envelope_version: 1, envelope: b64(&bytes), signature: machine.sign(&[DOMAIN, &bytes].concat()) };
    Ok(OutboxItem { id: uuid::Uuid::new_v4().to_string(), kind: kind.into(), recipient: Some(recipient.id.clone()),
        ciphertext: crate::crypto::seal_json(&recipient.box_pubkey, &signed)?, slot: None, group: None })
}

pub(crate) fn send<T: Serialize>(app: &App, kind: &str, recipient: &Device, body: &T) -> anyhow::Result<()> {
    let item = prepare(app, kind, recipient, body)?;
    app.store.transport_queue(&item)?;
    app.outbox_notify.notify_waiters();
    Ok(())
}

#[cfg(test)]
pub(crate) fn body_of<T: serde::de::DeserializeOwned>(secret: &crypto_box::SecretKey, ciphertext: &[u8]) -> T {
    let signed: Signed = crate::crypto::unseal_json(secret, ciphertext).unwrap();
    let envelope: Envelope = serde_json::from_slice(&unb64(&signed.envelope).unwrap()).unwrap();
    serde_json::from_value(envelope.body).unwrap()
}

#[cfg(test)]
pub(crate) fn fixture_blob<T: Serialize>(app: &App, sender: &crate::keys::Machine, seq: i64, kind: &str, id: &str, body: &T) -> crate::relay::BlobIn {
    let device=app.local_device().unwrap();
    let e=Envelope{envelope_version:1,identity:account(app).unwrap(),from:sender.pubkey(),to:device.id.clone(),kind:kind.into(),id:uuid::Uuid::new_v4().to_string(),at:crate::config::now_unix(),body:serde_json::to_value(body).unwrap()};
    let bytes=serde_json::to_vec(&e).unwrap();
    let signed=Signed{envelope_version:1,envelope:b64(&bytes),signature:sender.sign(&[DOMAIN,&bytes].concat())};
    app.state.lock().unwrap().listed_machines.insert(sender.pubkey(),1);
    crate::relay::BlobIn{id:id.into(),kind:kind.into(),recipient_machine_pubkey:Some(device.id),seq,ciphertext:b64(&crate::crypto::seal_json(&device.box_pubkey,&signed).unwrap()),created_at:seq}
}

pub(crate) fn inspect(app: &App, blob: &crate::relay::BlobIn) -> Result<Opened, Refusal> {
    let decode = || -> Result<Opened, Refusal> {
        if blob.ciphertext.len() > MAX_BYTES * 2 { return Err(Refusal::Rejected); }
        let file = app.machine_file().ok_or(Refusal::Rejected)?;
        let machine = file.machine().map_err(|_| Refusal::Rejected)?;
        let ciphertext = unb64(&blob.ciphertext).map_err(|_| Refusal::Rejected)?;
        let plain = crate::crypto::unseal(&machine.box_secret, &ciphertext).map_err(|_| Refusal::Rejected)?;
        if plain.len() > MAX_BYTES { return Err(Refusal::Rejected); }
        let value: Value = serde_json::from_slice(&plain).map_err(|_| Refusal::Rejected)?;
        if value.get("envelope_version").is_none() { return Err(Refusal::Legacy); }
        let signed: Signed = serde_json::from_slice(&plain).map_err(|_| Refusal::Rejected)?;
        if signed.envelope_version != 1 { return Err(Refusal::Unsupported); }
        let bytes = unb64(&signed.envelope).map_err(|_| Refusal::Rejected)?;
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| Refusal::Rejected)?;
        if envelope.envelope_version != 1 { return Err(Refusal::Unsupported); }
        let uuid = uuid::Uuid::parse_str(&envelope.id).map_err(|_| Refusal::Rejected)?;
        if uuid.hyphenated().to_string() != envelope.id || envelope.identity != file.identity_pubkey
            || envelope.to != machine.pubkey() || blob.recipient_machine_pubkey.as_deref() != Some(envelope.to.as_str())
            || envelope.kind != blob.kind || !addressed(&envelope.kind) { return Err(Refusal::Rejected); }
        let signature = ed25519_dalek::Signature::from_slice(&unb64(&signed.signature).map_err(|_| Refusal::Rejected)?).map_err(|_| Refusal::Rejected)?;
        crate::keys::verifying_key(&envelope.from).map_err(|_| Refusal::Rejected)?
            .verify_strict(&[DOMAIN, &bytes].concat(), &signature).map_err(|_| Refusal::Rejected)?;
        let now = crate::config::now_unix();
        let age = if matches!(envelope.kind.as_str(), "request" | "response") { 3600 } else { 8 * 86400 };
        if envelope.at > now.checked_add(3600).ok_or(Refusal::Rejected)? || envelope.at < now.checked_sub(age).ok_or(Refusal::Rejected)? { return Err(Refusal::Rejected); }
        {
            let state = app.state.lock().unwrap();
            if !state.listed_machines.contains_key(&envelope.from) {
                return Err(if state.caught_up { Refusal::Rejected } else { Refusal::Deferred });
            }
            if !state.devices.iter().any(|d| d.id == envelope.from) { return Err(Refusal::Deferred); }
        }
        match envelope.kind.as_str() {
            "job" => { let body: Job = serde_json::from_value(envelope.body.clone()).map_err(|_| Refusal::Rejected)?;
                if body.requested_by != envelope.from { return Err(Refusal::Rejected); } },
            "request" => { let body: Request = serde_json::from_value(envelope.body.clone()).map_err(|_| Refusal::Rejected)?;
                if body.requested_by != envelope.from || crate::requests::verify(app, &body).is_err() { return Err(Refusal::Rejected); } },
            "job_cancel" => { serde_json::from_value::<JobCancel>(envelope.body.clone()).map_err(|_| Refusal::Rejected)?; },
            "job_result" => { serde_json::from_value::<JobResult>(envelope.body.clone()).map_err(|_| Refusal::Rejected)?; },
            "response" => { serde_json::from_value::<Response>(envelope.body.clone()).map_err(|_| Refusal::Rejected)?; },
            _ => return Err(Refusal::Rejected),
        }
        Ok(Opened { envelope, bytes })
    };
    decode()
}

fn operation(e: &Envelope) -> anyhow::Result<Option<String>> {
    Ok(match e.kind.as_str() {
        "job" => Some(format!("job:{}", serde_json::from_value::<Job>(e.body.clone())?.id)),
        "request" => Some(format!("request:{}:{}", e.from, serde_json::from_value::<Request>(e.body.clone())?.id)),
        "response" | "job_result" | "job_cancel" => Some(format!("delivery:{}:{}", e.from, e.id)),
        _ => None,
    })
}

fn semantic_binding(e: &Envelope) -> anyhow::Result<String> {
    fn ordered(value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let sorted=fields.iter().collect::<std::collections::BTreeMap<_,_>>();
                Value::Object(sorted.into_iter().map(|(key,value)|(key.clone(),ordered(value))).collect())
            },
            Value::Array(values) => Value::Array(values.iter().map(ordered).collect()),
            value => value.clone(),
        }
    }
    Ok(serde_json::to_string(&ordered(&json!([e.from,e.to,e.kind,e.body])))?)
}
pub(crate) fn initialize(c: &rusqlite::Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS transport_outbound(id TEXT PRIMARY KEY, version INTEGER NOT NULL, ciphertext BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS transport_outbound_refusals(id TEXT PRIMARY KEY, http_status INTEGER NOT NULL, first_at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS transport_custody(account TEXT NOT NULL,id TEXT NOT NULL,blob TEXT NOT NULL,status TEXT NOT NULL,reason TEXT NOT NULL,first_at INTEGER NOT NULL,PRIMARY KEY(account,id));
        CREATE TABLE IF NOT EXISTS transport_replay(account TEXT NOT NULL,sender TEXT NOT NULL,id TEXT NOT NULL,bytes BLOB NOT NULL,PRIMARY KEY(account,sender,id));
        CREATE TABLE IF NOT EXISTS transport_operations(account TEXT NOT NULL,id TEXT NOT NULL,binding TEXT NOT NULL,status TEXT NOT NULL,envelope TEXT NOT NULL,response TEXT,PRIMARY KEY(account,id));
        CREATE TABLE IF NOT EXISTS transport_cancelled(account TEXT NOT NULL,job TEXT NOT NULL,PRIMARY KEY(account,job));")?;
    Ok(())
}

// All custody/claim transactions reach the WAL before effects or cursor publication.
fn durable<T>(store: &LocalStore, f: impl FnOnce(&rusqlite::Transaction<'_>) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut c = store.connection.lock().unwrap();
    let previous: i64 = c.pragma_query_value(None, "synchronous", |r| r.get(0))?;
    c.pragma_update(None, "synchronous", "FULL")?;
    let result: anyhow::Result<T> = (|| { let tx = c.transaction()?; let value = f(&tx)?; tx.commit()?; Ok(value) })();
    let restored = c.pragma_update(None, "synchronous", previous);
    restored?;
    result
}
fn queue(tx: &rusqlite::Transaction<'_>, item: &OutboxItem) -> anyhow::Result<()> {
    let old: Option<Vec<u8>> = tx.query_row("SELECT ciphertext FROM transport_outbound WHERE id=?1", [&item.id], |r| r.get(0)).optional()?;
    anyhow::ensure!(old.as_ref().is_none_or(|b| b == &item.ciphertext), "transport_identity_conflict");
    tx.execute("INSERT OR IGNORE INTO transport_outbound VALUES(?1,1,?2)", params![item.id,item.ciphertext])?;
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE id=?1)", [&item.id], |r| r.get::<_,bool>(0))? {
        crate::local_store::queue_outbox_tx(tx,item)?;
    }
    Ok(())
}
impl LocalStore {
    pub(crate) fn transport_uploadable(&self, app: &App) -> anyhow::Result<Option<OutboxItem>> {
        let items = {
            let c = self.connection.lock().unwrap();
            let mut s = c.prepare("SELECT id,kind,recipient,ciphertext,slot_name,slot_keep_first,group_name FROM outbox ORDER BY position")?;
            let rows = s.query_map([], crate::local_store::outbox_row)?.collect::<Result<Vec<_>,_>>()?;
            rows
        };
        for item in items {
            if !addressed(&item.kind) || (self.transport_version(&item)? && item.recipient.as_deref()
                .and_then(|id|app.device(id)).is_some_and(|d|d.envelope_version==Some(1))) { return Ok(Some(item)); }
        }
        Ok(None)
    }
    pub(crate) fn transport_queue(&self,item:&OutboxItem) -> anyhow::Result<()> { durable(self,|tx|queue(tx,item)) }
    pub(crate) fn transport_version(&self,item:&OutboxItem) -> anyhow::Result<bool> {
        Ok(self.connection.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM transport_outbound WHERE id=?1 AND version=1 AND ciphertext=?2 AND NOT EXISTS(SELECT 1 FROM transport_outbound_refusals WHERE id=?1))",params![item.id,item.ciphertext],|r|r.get(0))?)
    }
    pub(crate) fn transport_refuse_outbound(&self,id:&str,status:u16) -> anyhow::Result<()> {
        durable(self,|tx|{tx.execute("INSERT OR IGNORE INTO transport_outbound_refusals VALUES(?1,?2,?3)",params![id,status,crate::config::now_unix()])?;Ok(())})
    }
    pub(crate) fn transport_recover(&self) -> anyhow::Result<()> {
        durable(self,|tx| {
            tx.execute("UPDATE transport_operations SET status=CASE WHEN id LIKE 'delivery:%' THEN 'pending' ELSE 'uncertain' END WHERE status='claimed'",[])?;
            Ok(())
        })
    }
    pub(crate) fn transport_uncertain(&self,account:&str,id:&str) -> anyhow::Result<()> {
        durable(self,|tx|{anyhow::ensure!(tx.execute("UPDATE transport_operations SET status='uncertain' WHERE account=?1 AND id=?2 AND status='claimed'",params![account,id])?==1,"Uncertain operation claim is unavailable");Ok(())})
    }
    pub(crate) fn transport_claim(&self,account:&str,id:&str) -> anyhow::Result<bool> {
        durable(self,|tx| {
            if let Some(job)=id.strip_prefix("job:") {
                if tx.query_row("SELECT EXISTS(SELECT 1 FROM transport_cancelled WHERE account=?1 AND job=?2)",params![account,job],|r|r.get::<_,bool>(0))? {
                    tx.execute("UPDATE transport_operations SET status='cancelled' WHERE account=?1 AND id=?2 AND status='pending'",params![account,id])?; return Ok(false);
                }
            }
            Ok(tx.execute("UPDATE transport_operations SET status='claimed' WHERE account=?1 AND id=?2 AND status='pending'",params![account,id])? == 1)
        })
    }
    pub(crate) fn transport_cancelled(&self,account:&str,job:&str) -> anyhow::Result<bool> {
        Ok(self.connection.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM transport_cancelled WHERE account=?1 AND job=?2)",params![account,job],|r|r.get(0))?)
    }
    pub(crate) fn transport_finish_job(&self,account:&str,id:&str,item:&OutboxItem) -> anyhow::Result<()> {
        durable(self,|tx| {
            let status:Option<String>=tx.query_row("SELECT status FROM transport_operations WHERE account=?1 AND id=?2",params![account,id],|r|r.get(0)).optional()?;
            match status.as_deref() {
                None => queue(tx,item)?,
                Some("claimed") => {
                    let saved=json!({"id":item.id,"kind":item.kind,"recipient":item.recipient,"ciphertext":b64(&item.ciphertext)}).to_string();
                    queue(tx,item)?;
                    tx.execute("UPDATE transport_operations SET status='completed',response=?3 WHERE account=?1 AND id=?2 AND status='claimed'",params![account,id,saved])?;
                },
                _ => anyhow::bail!("Transport operation cannot report a second outcome"),
            }
            Ok(())
        })
    }
    pub(crate) fn transport_complete(&self,account:&str,id:&str,response:Option<&OutboxItem>) -> anyhow::Result<()> {
        durable(self,|tx| {
            if let Some(item)=response { queue(tx,item)?; }
            let saved=response.map(|i|json!({"id":i.id,"kind":i.kind,"recipient":i.recipient,"ciphertext":b64(&i.ciphertext)}).to_string());
            anyhow::ensure!(tx.execute("UPDATE transport_operations SET status='completed',response=?3 WHERE account=?1 AND id=?2 AND status='claimed'",params![account,id,saved])? == 1,"Operation is not claimed");
            Ok(())
        })
    }
    pub(crate) fn transport_pending(&self,account:&str) -> anyhow::Result<Vec<Envelope>> {
        let c=self.connection.lock().unwrap(); let mut s=c.prepare("SELECT envelope FROM transport_operations WHERE account=?1 AND status='pending' ORDER BY rowid")?;
        let rows=s.query_map([account],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
        rows.iter().map(|s|serde_json::from_str(s).map_err(Into::into)).collect()
    }
    pub(crate) fn transport_deferred(&self,account:&str) -> anyhow::Result<Vec<crate::relay::BlobIn>> {
        let c=self.connection.lock().unwrap(); let mut s=c.prepare("SELECT blob FROM transport_custody WHERE account=?1 AND status='deferred' ORDER BY rowid")?;
        let rows=s.query_map([account],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
        rows.iter().map(|s|serde_json::from_str(s).map_err(Into::into)).collect()
    }
    pub(crate) fn transport_status(&self) -> anyhow::Result<Value> {
        let c=self.connection.lock().unwrap();
        let mut s=c.prepare("SELECT id,reason,first_at FROM transport_custody WHERE status NOT IN ('accepted') ORDER BY rowid DESC LIMIT 100")?;
        let mut items=s.query_map([],|r|Ok(json!({"direction":"inbound","blob_id":r.get::<_,String>(0)?,"reason":r.get::<_,String>(1)?,"first_at":r.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
        let mut s=c.prepare("SELECT id FROM transport_operations WHERE status='uncertain' ORDER BY rowid DESC LIMIT 100")?;
        items.extend(s.query_map([],|r|Ok(json!({"operation_id":r.get::<_,String>(0)?,"reason":"transport_operation_uncertain"})))?.collect::<Result<Vec<_>,_>>()?);
        let mut s=c.prepare("SELECT o.id,o.kind,o.recipient,q.http_status,q.first_at FROM transport_outbound_refusals q JOIN outbox o ON o.id=q.id ORDER BY q.rowid DESC LIMIT 100")?;
        items.extend(s.query_map([],|r|Ok(json!({"direction":"outbound","blob_id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"peer":r.get::<_,Option<String>>(2)?,"http_status":r.get::<_,u16>(3)?,"first_at":r.get::<_,i64>(4)?,"reason":"transport_relay_rejected"})))?.collect::<Result<Vec<_>,_>>()?);
        let count:i64=c.query_row("SELECT count(*) FROM outbox o WHERE kind IN ('job','job_cancel','job_result','request','response') AND NOT EXISTS(SELECT 1 FROM transport_outbound t WHERE t.id=o.id AND t.version=1 AND t.ciphertext=o.ciphertext)",[],|r|r.get(0))?;
        Ok(json!({"items":items,"legacy_outbound_count":count}))
    }
}

pub(crate) fn account(app:&App) -> anyhow::Result<String> { Ok(app.machine_file().ok_or_else(||anyhow::anyhow!("No identity"))?.identity_pubkey) }


fn admit_tx(tx:&rusqlite::Transaction<'_>,account:&str,blob:&crate::relay::BlobIn,inspected:&Result<Opened,Refusal>,advance:bool,staged:bool) -> anyhow::Result<bool> {
    let (status,reason)=match inspected {Ok(_)=>("accepted",""),Err(Refusal::Deferred)=>(if staged{"staged_deferred"}else{"deferred"},"transport_sender_evidence_pending"),Err(e)=>("rejected",e.code())};
    let blob_json=serde_json::to_string(blob)?;
        let old:Option<String>=tx.query_row("SELECT blob FROM transport_custody WHERE account=?1 AND id=?2",params![account,blob.id],|r|r.get(0)).optional()?;
        anyhow::ensure!(old.as_ref().is_none_or(|s|s==&blob_json),"transport_identity_conflict");
        tx.execute("INSERT INTO transport_custody VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(account,id) DO UPDATE SET status=excluded.status,reason=excluded.reason WHERE transport_custody.status IN ('deferred','reserved')",params![account,blob.id,blob_json,status,reason,crate::config::now_unix()])?;
        let mut fresh=false;
        if let Ok(opened)=&inspected {
            let e=&opened.envelope;
            let previous:Option<Vec<u8>>=tx.query_row("SELECT bytes FROM transport_replay WHERE account=?1 AND sender=?2 AND id=?3",params![account,e.from,e.id],|r|r.get(0)).optional()?;
            anyhow::ensure!(previous.as_ref().is_none_or(|b|b==&opened.bytes),"transport_identity_conflict");
            fresh=previous.is_none();
            tx.execute("INSERT OR IGNORE INTO transport_replay VALUES(?1,?2,?3,?4)",params![account,e.from,e.id,opened.bytes])?;
            if let Some(id)=operation(e)? {
                let binding=semantic_binding(e)?;
                let old:Option<(String,Option<String>)>=tx.query_row("SELECT binding,response FROM transport_operations WHERE account=?1 AND id=?2",params![account,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                anyhow::ensure!(old.as_ref().is_none_or(|(b,_)|b==&binding),"transport_identity_conflict");
                if let Some((_,Some(saved)))=&old { let v:Value=serde_json::from_str(saved)?; let item=OutboxItem{id:v["id"].as_str().unwrap().into(),kind:v["kind"].as_str().unwrap().into(),recipient:v["recipient"].as_str().map(str::to_string),ciphertext:unb64(v["ciphertext"].as_str().unwrap())?,slot:None,group:None}; queue(tx,&item)?; }
                if fresh && old.is_none() {
                    tx.execute("INSERT INTO transport_operations(account,id,binding,status,envelope) VALUES(?1,?2,?3,?4,?5)",params![account,id,binding,if staged{"staged"}else{"pending"},serde_json::to_string(e)?])?;
                }
            }
            if !staged && e.kind=="job_cancel" { let cancel:JobCancel=serde_json::from_value(e.body.clone())?;
                tx.execute("INSERT OR IGNORE INTO transport_cancelled VALUES(?1,?2)",params![account,cancel.job_id])?;
            }
        }
        if advance { tx.execute("UPDATE metadata SET last_seq=max(last_seq,?1) WHERE id=1",[blob.seq])?; }
        Ok(fresh)
}

pub(crate) fn preflight_page(app:&App,blobs:&[crate::relay::BlobIn]) -> anyhow::Result<()> {
    let account=account(app)?;
    let inspected=blobs.iter().filter(|b|addressed(&b.kind)).map(|b|(b,inspect(app,b))).collect::<Vec<_>>();
    durable(&app.store,|tx|{for (blob,opened) in &inspected {admit_tx(tx,&account,blob,opened,false,true)?;}Ok(())})
}

pub(crate) fn activate(app:&App,blob:&crate::relay::BlobIn) -> anyhow::Result<()> { activate_at(app,blob,true) }
pub(crate) fn activate_first(app:&App,blob:&crate::relay::BlobIn) -> anyhow::Result<()> { activate_at(app,blob,false) }
fn activate_at(app:&App,blob:&crate::relay::BlobIn,advance:bool) -> anyhow::Result<()> {
    activate_prepared(app,blob,advance,||{},||{})
}

fn activate_prepared(app:&App,blob:&crate::relay::BlobIn,advance:bool,prepared:impl FnOnce(),locked:impl FnOnce()) -> anyhow::Result<()> {
    // All authority reads precede the SQLite lifetime. The transaction borrows only values.
    let account=account(app)?;
    let inspected=inspect(app,blob);
    let blob_json=serde_json::to_string(blob)?;
    prepared();
    durable(&app.store,|tx|{
        locked();
        let (status,saved):(String,String)=tx.query_row("SELECT status,blob FROM transport_custody WHERE account=?1 AND id=?2",params![account,blob.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        anyhow::ensure!(saved==blob_json,"transport_identity_conflict");
        let opened=if status=="accepted" {Some(inspected.as_ref().map_err(|_|anyhow::anyhow!("Accepted envelope evidence changed"))?)}else{None};
        if status=="staged_deferred" {
            tx.execute("UPDATE transport_custody SET status='deferred' WHERE account=?1 AND id=?2 AND status='staged_deferred'",params![account,blob.id])?;
        }
        if let Some(opened)=&opened {
            if let Some(id)=operation(&opened.envelope)? {
                tx.execute("UPDATE transport_operations SET status='pending' WHERE account=?1 AND id=?2 AND status='staged'",params![account,id])?;
            }
            if opened.envelope.kind=="job_cancel" {
                let cancel:JobCancel=serde_json::from_value(opened.envelope.body.clone())?;
                tx.execute("INSERT OR IGNORE INTO transport_cancelled VALUES(?1,?2)",params![account,cancel.job_id])?;
            }
        }
        if advance {tx.execute("UPDATE metadata SET last_seq=max(last_seq,?1) WHERE id=1",[blob.seq])?;}
        Ok(())
    })?;
    if advance {let mut state=app.state.lock().unwrap();state.last_seq=state.last_seq.max(blob.seq);}Ok(())
}

pub(crate) fn receive(app:&Arc<App>,blob:&crate::relay::BlobIn,advance:bool) -> anyhow::Result<()> {
    let account=account(app)?;
    let inspected=inspect(app,blob);
    let blob_json=serde_json::to_string(blob)?;
    let admitted=durable(&app.store,|tx|admit_tx(tx,&account,blob,&inspected,advance,false));
    let fresh = match admitted {
        Ok(fresh) => fresh,
        Err(error) => {
        if error.to_string().contains("transport_identity_conflict") {
            durable(&app.store, |tx| {
                tx.execute("INSERT INTO transport_custody VALUES(?1,?2,?3,'rejected','transport_identity_conflict',?4) ON CONFLICT(account,id) DO UPDATE SET reason='transport_identity_conflict'", params![account,blob.id,blob_json,crate::config::now_unix()])?;
                Ok(())
            })?;
        }
            return Err(error);
        }
    };
    if advance { let mut state=app.state.lock().unwrap(); state.last_seq=state.last_seq.max(blob.seq); }
    // Every accepted kind has a recoverable inbox operation; no publication lives only here.
    if fresh && !advance && !app.bulk_sync.load(std::sync::atomic::Ordering::Relaxed) { pump(app)?; }
    if !app.bulk_sync.load(std::sync::atomic::Ordering::Relaxed) { app.emit(crate::events::Event::Snapshot(app.snapshot())); }
    Ok(())
}

pub(crate) fn pump(app:&Arc<App>) -> anyhow::Result<()> { pump_selected(app,None) }

pub(crate) fn pump_at(app:&Arc<App>,blob:&crate::relay::BlobIn) -> anyhow::Result<()> {
    let selected=inspect(app,blob).ok().map(|opened|operation(&opened.envelope)).transpose()?.flatten();
    if let Some(id)=selected {pump_selected(app,Some(&id))?;}
    Ok(())
}

fn pump_selected(app:&Arc<App>,selected:Option<&str>) -> anyhow::Result<()> {
    let account=account(app)?;
    for e in app.store.transport_pending(&account)? {
        if selected.is_some_and(|id|operation(&e).ok().flatten().as_deref()!=Some(id)){continue;}
        if !app.state.lock().unwrap().listed_machines.contains_key(&e.from) {continue;}
        if e.kind == "job_result" && app.bulk_sync.load(std::sync::atomic::Ordering::Relaxed) { continue; }
        if e.kind == "response" {
            let response:Response=serde_json::from_value(e.body.clone())?;
            if !app.pending_responses.lock().unwrap().get(&response.request_id).is_some_and(|(runner,_)|runner==&e.from) { continue; }
        }
        if e.kind == "job_result" {
            let result:JobResult=serde_json::from_value(e.body.clone())?;
            if !app.running_jobs.lock().unwrap().get(&result.job_id).is_some_and(|job|job.runner_id.as_deref()==Some(e.from.as_str()) && job.chat_id==result.chat_id && job.bot_id==result.bot_id) { continue; }
        }
        let continues = matches!(e.kind.as_str(), "response" | "job_result" | "job_cancel")
            || (e.kind=="request" && crate::sync::continues_work(e.body["verb"].as_str().unwrap_or("")));
        let admission=if continues {app.update.hold()} else {let Some(a)=app.update.try_admit() else {continue}; a};
        let id=operation(&e)?.expect("pending operation kind");
        if e.kind=="request" {
            let request:Request=serde_json::from_value(e.body)?;
            if crate::requests::verify(app,&request).is_err(){continue;}
            if app.store.transport_claim(&account,&id)? {crate::requests::serve(app.clone(),request,String::new(),admission);}
        } else if e.kind=="job" {
            crate::runtime::spawn_local_job(app.clone(),serde_json::from_value(e.body)?,Some(String::new()),admission);
        } else if app.store.transport_claim(&account,&id)? {
            match e.kind.as_str() {
                "job_cancel" => {
                    let cancel:JobCancel=serde_json::from_value(e.body)?;
                    let _registration=app.transport_registration.lock().unwrap();
                    app.cancel_job(&cancel.job_id);
                },
                "response" => crate::requests::deliver(app.clone(), &e.from, serde_json::from_value(e.body)?, String::new()),
                "job_result" => crate::runtime::deliver_job_result(app, &e.from, serde_json::from_value(e.body)?),
                _ => unreachable!("validated delivery kind"),
            }
            app.store.transport_complete(&account,&id,None)?;
        }
    }
    Ok(())
}

pub(crate) fn refresh(app:&Arc<App>) -> anyhow::Result<()> {
    for blob in app.store.transport_deferred(&account(app)?)? {receive(app,&blob,false)?;}
    pump(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Pair { a: Arc<App>, b: Arc<App>, homes: Vec<std::path::PathBuf> }
    impl Drop for Pair { fn drop(&mut self) { for p in &self.homes { let _=std::fs::remove_dir_all(p); } } }
    fn pair() -> Pair {
        let homes=(0..2).map(|_|std::env::temp_dir().join(format!("beans-transport-{}",uuid::Uuid::new_v4()))).collect::<Vec<_>>();
        let a=App::load(crate::config::Config{home:homes[0].clone(),port:0}).unwrap();
        crate::identity::create(&a,None).unwrap();
        let b=App::load(crate::config::Config{home:homes[1].clone(),port:0}).unwrap();
        let mut file=a.machine_file().unwrap(); file.machine_secret=b64(&crate::keys::Machine::generate().secret);
        *b.machine.lock().unwrap()=Some(file); b.save_machine().unwrap();
        let devices=[a.local_device().unwrap(),b.local_device().unwrap()];
        {let source=a.state.lock().unwrap();let mut target=b.state.lock().unwrap();target.bots=source.bots.clone();target.chats=source.chats.clone();for bot in &mut target.bots{bot.runner_id=devices[1].id.clone();}}
        for app in [&a,&b] { let mut state=app.state.lock().unwrap(); state.devices=devices.to_vec();
            for d in &devices {state.listed_machines.insert(d.id.clone(),1);} drop(state); app.save_state_now(); }
        Pair{a,b,homes}
    }
    fn blob(p:&Pair,kind:&str,body:Value)->crate::relay::BlobIn {
        let item=prepare(&p.a,kind,&p.b.local_device().unwrap(),&body).unwrap();
        crate::relay::BlobIn{id:item.id,kind:kind.into(),recipient_machine_pubkey:item.recipient,ciphertext:b64(&item.ciphertext),seq:9,created_at:1}
    }
    fn request(p:&Pair,id:&str)->Value {
        let mut r=Request{id:id.into(),verb:"memory.write".into(),requested_by:p.a.this_device_id().unwrap(),body:json!({"bot_id":"absent","text":"effect"}),created_at:crate::config::now_secs()};
        let bytes=serde_json::to_vec(&("beans-request-v1",&r.id,&r.verb,&r.requested_by,p.b.this_device_id().unwrap(),r.created_at,&r.body)).unwrap();
        r.body=json!({"payload":r.body,"signature":p.a.machine_file().unwrap().machine().unwrap().sign(&bytes)});
        json!(r)
    }
    fn hold(p:&Pair){p.b.update.prepare(&p.b,std::time::Duration::from_secs(60));}
    fn count(app:&App,table:&str)->i64 {app.store.connection.lock().unwrap().query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get(0)).unwrap()}

    #[test]
    fn mixed_peer_capability_preserves_metadata_and_refuses_production() {
        let p=pair(); let mut wire=json!(p.b.local_device().unwrap());
        for value in [Value::Null,json!(2),json!("1"),json!({})] {wire["envelope_version"]=value;
            let d:Device=serde_json::from_value(wire.clone()).unwrap(); assert_eq!(d.id,p.b.this_device_id().unwrap());
            assert!(prepare(&p.a,"job_cancel",&d,&json!({"job_id":"j"})).is_err());}
        let item=prepare(&p.a,"job_cancel",&p.b.local_device().unwrap(),&json!({"job_id":"j"})).unwrap();
        p.a.store.transport_queue(&item).unwrap();
        p.a.state.lock().unwrap().devices.iter_mut().find(|d|d.id==p.b.this_device_id().unwrap()).unwrap().envelope_version=None;
        let selected=p.a.store.transport_uploadable(&p.a).unwrap();
        assert!(selected.as_ref().is_none_or(|row|row.id!=item.id));
        assert_eq!(p.a.store.outbox().unwrap().into_iter().find(|row|row.id==item.id).unwrap().ciphertext,item.ciphertext);
        assert!(crate::crypto::unseal_json::<JobCancel>(&p.b.machine_file().unwrap().machine().unwrap().box_secret,&item.ciphertext).is_err());
    }
    #[tokio::test]
    async fn legacy_custody_survives_restart_without_upload_or_dispatch() {
        let p=pair(); let machine=p.b.machine_file().unwrap().machine().unwrap();
        let legacy=OutboxItem{id:"legacy".into(),kind:"job_cancel".into(),recipient:Some(machine.pubkey()),ciphertext:crate::crypto::seal_json(&machine.box_pubkey(),&json!({"job_id":"j"})).unwrap(),slot:None,group:None};
        p.b.store.queue_outbox(&legacy).unwrap();
        let incoming=crate::relay::BlobIn{id:"old-inbound".into(),kind:legacy.kind.clone(),recipient_machine_pubkey:legacy.recipient.clone(),ciphertext:b64(&legacy.ciphertext),seq:7,created_at:1};
        receive(&p.b,&incoming,true).unwrap();
        assert_eq!(p.b.store.load_state().unwrap().last_seq,7);
        let reloaded=App::load(crate::config::Config{home:p.homes[1].clone(),port:0}).unwrap();
        assert_eq!(reloaded.store.transport_status().unwrap()["legacy_outbound_count"],1);
        assert_eq!(reloaded.store.transport_status().unwrap()["items"][0]["reason"],"transport_legacy_inbound_quarantined");
        assert_eq!(reloaded.store.outbox().unwrap().iter().find(|i|i.id=="legacy").unwrap().ciphertext,legacy.ciphertext);
        assert_eq!(count(&reloaded,"transport_operations"),0);
    }
    #[tokio::test]
    async fn request_claim_crash_never_repeats_effect_and_completion_reuses_response() {
        let p=pair();hold(&p);let incoming=blob(&p,"request",request(&p,"req-effect"));
        receive(&p.b,&incoming,true).unwrap();let account=account(&p.b).unwrap();let id=format!("request:{}:req-effect",p.a.this_device_id().unwrap());
        assert!(p.b.store.transport_claim(&account,&id).unwrap());
        let effect=p.homes[1].join("effect");std::fs::write(&effect,b"once").unwrap();
        p.b.store.transport_recover().unwrap();assert!(!p.b.store.transport_claim(&account,&id).unwrap());
        assert_eq!(std::fs::read(effect).unwrap(),b"once");
        assert_eq!(p.b.store.transport_status().unwrap()["items"][0]["reason"],"transport_operation_uncertain");
        let next=blob(&p,"request",request(&p,"req-complete"));receive(&p.b,&next,false).unwrap();
        let id=format!("request:{}:req-complete",p.a.this_device_id().unwrap());assert!(p.b.store.transport_claim(&account,&id).unwrap());
        let saved=prepare(&p.b,"response",&p.a.local_device().unwrap(),&json!({"request_id":"req-complete","body":"done"})).unwrap();
        p.b.store.transport_complete(&account,&id,Some(&saved)).unwrap();
        p.b.store.connection.lock().unwrap().execute("DELETE FROM outbox WHERE id=?1",[&saved.id]).unwrap();
        receive(&p.b,&next,false).unwrap();
        let retry=p.b.store.outbox().unwrap().into_iter().find(|i|i.id==saved.id).unwrap();assert_eq!(retry.ciphertext,saved.ciphertext);
        let mut retransmit=next.clone(); retransmit.id="completed-retransmit".into();
        receive(&p.b,&retransmit,false).unwrap();
        let same_operation=blob(&p,"request",inspect(&p.b,&next).unwrap().envelope.body);
        receive(&p.b,&same_operation,false).unwrap();
        p.b.store.transport_recover().unwrap();
        assert!(!p.b.store.transport_claim(&account,&id).unwrap());
        assert!(!p.b.store.transport_pending(&account).unwrap().iter().any(|e|operation(e).unwrap().as_deref()==Some(id.as_str())));
    }
    #[tokio::test]
    async fn deferred_sender_keeps_cursor_custody_then_accepts_once() {
        let p=pair();hold(&p);let incoming=blob(&p,"request",request(&p,"req-deferred"));
        let sender=p.a.local_device().unwrap();p.b.state.lock().unwrap().devices.retain(|d|d.id!=sender.id);
        receive(&p.b,&incoming,true).unwrap();assert_eq!(count(&p.b,"transport_replay"),0);
        assert_eq!(p.b.store.load_state().unwrap().last_seq,9);
        p.b.store.transport_recover().unwrap();p.b.state.lock().unwrap().devices.push(sender);
        refresh(&p.b).unwrap();assert_eq!(count(&p.b,"transport_replay"),1);assert_eq!(count(&p.b,"transport_operations"),1);
        refresh(&p.b).unwrap();assert_eq!(count(&p.b,"transport_operations"),1);
    }
    #[tokio::test]
    async fn stop_tombstone_survives_restart_and_serializes_job_claim() {
        let p=pair();let incoming=blob(&p,"job_cancel",json!({"job_id":"j"}));receive(&p.b,&incoming,false).unwrap();
        let a=account(&p.b).unwrap();assert!(p.b.store.transport_cancelled(&a,"j").unwrap());
        durable(&p.b.store,|tx|{tx.execute("INSERT INTO transport_operations VALUES(?1,'job:j','binding','pending','{}',NULL)",[&a])?;Ok(())}).unwrap();
        assert!(!p.b.store.transport_claim(&a,"job:j").unwrap());p.b.store.transport_recover().unwrap();
        assert!(p.b.store.transport_cancelled(&a,"j").unwrap());assert!(!p.b.store.transport_claim(&a,"job:j").unwrap());
    }
    #[tokio::test]
    async fn wrong_runner_leaves_response_waiter_intact() {
        let p=pair();let(tx,mut rx)=tokio::sync::oneshot::channel();let expected=p.a.this_device_id().unwrap();
        p.b.pending_responses.lock().unwrap().insert("q".into(),(expected.clone(),tx));
        crate::requests::deliver(p.b.clone(),"wrong",Response{request_id:"q".into(),body:json!(1),error:None},String::new());
        assert!(rx.try_recv().is_err());assert!(p.b.pending_responses.lock().unwrap().contains_key("q"));
        crate::requests::deliver(p.b.clone(),&expected,Response{request_id:"q".into(),body:json!(2),error:None},String::new());
        assert_eq!(rx.await.unwrap().body,json!(2));
    }
    #[tokio::test]
    async fn replay_and_operation_identity_reject_conflicting_bodies() {
        let p=pair();hold(&p);let first=blob(&p,"request",request(&p,"req-one"));receive(&p.b,&first,false).unwrap();
        let mut duplicate=first.clone();duplicate.id="another-relay-id".into();receive(&p.b,&duplicate,false).unwrap();
        assert_eq!(count(&p.b,"transport_replay"),1);assert_eq!(count(&p.b,"transport_operations"),1);
        let mut changed_body=request(&p,"req-one");
        changed_body["created_at"]=json!(crate::config::now_secs()+1.0);
        let mut r:Request=serde_json::from_value(changed_body).unwrap();
        let payload=r.body["payload"].clone();
        let bytes=serde_json::to_vec(&("beans-request-v1",&r.id,&r.verb,&r.requested_by,p.b.this_device_id().unwrap(),r.created_at,&payload)).unwrap();
        r.body=json!({"payload":payload,"signature":p.a.machine_file().unwrap().machine().unwrap().sign(&bytes)});
        let changed=blob(&p,"request",json!(r));
        assert!(receive(&p.b,&changed,false).is_err());assert_eq!(count(&p.b,"transport_operations"),1);
        let mut same_id=inspect(&p.b,&first).unwrap().envelope;
        same_id.at+=1;
        let bytes=serde_json::to_vec(&same_id).unwrap();
        let signed=Signed{envelope_version:1,envelope:b64(&bytes),signature:p.a.machine_file().unwrap().machine().unwrap().sign(&[DOMAIN,&bytes].concat())};
        let mut conflicting=first.clone();conflicting.id="conflicting-replay".into();conflicting.ciphertext=b64(&crate::crypto::seal_json(&p.b.local_device().unwrap().box_pubkey,&signed).unwrap());
        assert!(receive(&p.b,&conflicting,false).is_err());assert_eq!(count(&p.b,"transport_replay"),1);
        let mut wrong=first.clone();wrong.kind="response".into();assert!(matches!(inspect(&p.b,&wrong),Err(Refusal::Rejected)));
    }
    #[tokio::test]
    async fn transaction_failure_leaves_no_receipt_or_cursor_or_response() {
        let p=pair();hold(&p);let incoming=blob(&p,"request",request(&p,"req-fault"));
        p.b.store.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_transport BEFORE INSERT ON transport_operations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(receive(&p.b,&incoming,true).is_err());assert_eq!(count(&p.b,"transport_replay"),0);assert_eq!(count(&p.b,"transport_custody"),0);assert_eq!(p.b.store.load_state().unwrap().last_seq,0);
        p.b.store.connection.lock().unwrap().execute_batch("DROP TRIGGER reject_transport;").unwrap();receive(&p.b,&incoming,true).unwrap();
        let a=account(&p.b).unwrap();let id=format!("request:{}:req-fault",p.a.this_device_id().unwrap());assert!(p.b.store.transport_claim(&a,&id).unwrap());
        let saved=prepare(&p.b,"response",&p.a.local_device().unwrap(),&json!({"request_id":"req-fault","body":0})).unwrap();
        p.b.store.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE ON transport_operations WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(p.b.store.transport_complete(&a,&id,Some(&saved)).is_err());assert!(!p.b.store.outbox().unwrap().iter().any(|i|i.id==saved.id));
        p.b.store.transport_recover().unwrap();assert!(!p.b.store.transport_claim(&a,&id).unwrap());
    }
    #[tokio::test]
    async fn drain_inspection_does_not_consume_and_stop_progresses() {
        let p=pair();hold(&p);let incoming=blob(&p,"request",request(&p,"req-held"));
        inspect(&p.b,&incoming).unwrap();inspect(&p.b,&incoming).unwrap();assert_eq!(count(&p.b,"transport_replay"),0);
        receive(&p.b,&incoming,false).unwrap();let a=account(&p.b).unwrap();assert_eq!(p.b.store.transport_pending(&a).unwrap().len(),1);
        receive(&p.b,&blob(&p,"job_cancel",json!({"job_id":"not-here-yet"})),false).unwrap();
        assert!(p.b.store.transport_cancelled(&a,"not-here-yet").unwrap());assert_eq!(p.b.store.load_state().unwrap().last_seq,0);
    }
    #[tokio::test]
    async fn claimed_memory_effect_is_not_repeated_after_interruption() {
        let p=pair(); let bot=p.b.state.lock().unwrap().bots[0].clone();
        hold(&p);let incoming=blob(&p,"request",request(&p,"request-memory"));receive(&p.b,&incoming,false).unwrap();
        let a=account(&p.b).unwrap();let id=format!("request:{}:request-memory",p.a.this_device_id().unwrap());
        assert!(p.b.store.transport_claim(&a,&id).unwrap());
        crate::requests::memory_write(&p.b,&bot.id,"first write",None).unwrap();
        p.b.store.transport_recover().unwrap();
        crate::requests::memory_write(&p.b,&bot.id,"intervening write",None).unwrap();
        receive(&p.b,&incoming,false).unwrap();p.b.update.cancel(&p.b);pump(&p.b).unwrap();
        assert!(crate::requests::memory_read(&p.b,&bot.id).unwrap().to_string().contains("intervening write"));
        assert!(!p.b.store.transport_claim(&a,&id).unwrap());
    }

    #[tokio::test]
    async fn accepted_reply_recovers_after_commit_before_publication() {
        let p=pair();let incoming=blob(&p,"response",json!({"request_id":"q","body":"answer"}));
        receive(&p.b,&incoming,true).unwrap();
        let a=account(&p.b).unwrap();assert_eq!(p.b.store.transport_pending(&a).unwrap().len(),1);
        p.b.store.transport_recover().unwrap();
        let(tx,rx)=tokio::sync::oneshot::channel();p.b.pending_responses.lock().unwrap().insert("q".into(),(p.a.this_device_id().unwrap(),tx));
        pump(&p.b).unwrap();assert_eq!(rx.await.unwrap().body,json!("answer"));
        assert!(p.b.store.transport_pending(&a).unwrap().is_empty());
        let mut duplicate=incoming;duplicate.id="reply-retransmit".into();receive(&p.b,&duplicate,false).unwrap();
        assert!(p.b.store.transport_pending(&a).unwrap().is_empty());
    }

    #[tokio::test]
    async fn stop_between_claim_and_registration_and_concurrent_claim() {
        let p=pair();hold(&p);let a=account(&p.b).unwrap();
        durable(&p.b.store,|tx|{tx.execute("INSERT INTO transport_operations VALUES(?1,'job:racing','binding','pending','{}',NULL)",[&a])?;Ok(())}).unwrap();
        assert!(p.b.store.transport_claim(&a,"job:racing").unwrap());
        receive(&p.b,&blob(&p,"job_cancel",json!({"job_id":"racing"})),false).unwrap();
        let token=tokio_util::sync::CancellationToken::new();
        {let _registration=p.b.transport_registration.lock().unwrap();if p.b.store.transport_cancelled(&a,"racing").unwrap(){token.cancel();}}
        assert!(token.is_cancelled());
        let barrier=Arc::new(std::sync::Barrier::new(2));
        durable(&p.b.store,|tx|{tx.execute("INSERT INTO transport_operations VALUES(?1,'job:concurrent','binding','pending','{}',NULL)",[&a])?;Ok(())}).unwrap();
        let app=p.b.clone();let gate=barrier.clone();let account=a.clone();
        let claim=std::thread::spawn(move||{gate.wait();app.store.transport_claim(&account,"job:concurrent").unwrap()});
        barrier.wait();receive(&p.b,&blob(&p,"job_cancel",json!({"job_id":"concurrent"})),false).unwrap();
        let won=claim.join().unwrap();assert!(p.b.store.transport_cancelled(&a,"concurrent").unwrap());
        assert!(!p.b.store.transport_claim(&a,"job:concurrent").unwrap());
        let status:String=p.b.store.connection.lock().unwrap().query_row("SELECT status FROM transport_operations WHERE id='job:concurrent'",[],|r|r.get(0)).unwrap();
        assert_eq!(status,if won{"claimed"}else{"cancelled"});
    }

    #[tokio::test]
    async fn tampered_headers_time_and_signature_never_admit() {
        let p=pair();let original=blob(&p,"job_cancel",json!({"job_id":"secure"}));let opened=inspect(&p.b,&original).unwrap();
        for field in ["identity","to","kind","from","id","at","signature"] {
            let mut e=opened.envelope.clone();
            match field {"identity"=>e.identity="other".into(),"to"=>e.to="other".into(),"kind"=>e.kind="request".into(),"from"=>e.from=p.b.this_device_id().unwrap(),"id"=>e.id="bad".into(),"at"=>e.at=0,_=>{}}
            let bytes=serde_json::to_vec(&e).unwrap();
            let signer=if field=="signature"{p.b.machine_file().unwrap().machine().unwrap()}else{p.a.machine_file().unwrap().machine().unwrap()};
            let signed=Signed{envelope_version:1,envelope:b64(&bytes),signature:signer.sign(&[DOMAIN,&bytes].concat())};
            let mut invalid=original.clone();invalid.id=uuid::Uuid::new_v4().to_string();invalid.ciphertext=b64(&crate::crypto::seal_json(&p.b.local_device().unwrap().box_pubkey,&signed).unwrap());
            assert!(matches!(inspect(&p.b,&invalid),Err(Refusal::Rejected)),"{field}");receive(&p.b,&invalid,false).unwrap();
        }
        assert_eq!(count(&p.b,"transport_replay"),0);assert_eq!(count(&p.b,"transport_cancelled"),0);
    }
    #[tokio::test]
    async fn request_dispatch_effect_then_completion_failure_is_uncertain() {
        let p=pair();let bot=p.b.state.lock().unwrap().bots[0].clone();
        let mut r:Request=serde_json::from_value(request(&p,"real-write")).unwrap();
        let payload=json!({"bot_id":bot.id,"text":"persisted effect"});
        let bytes=serde_json::to_vec(&("beans-request-v1",&r.id,&r.verb,&r.requested_by,p.b.this_device_id().unwrap(),r.created_at,&payload)).unwrap();
        r.body=json!({"payload":payload,"signature":p.a.machine_file().unwrap().machine().unwrap().sign(&bytes)});
        p.b.store.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_response BEFORE UPDATE ON transport_operations WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'completion fault'); END;").unwrap();
        let incoming=blob(&p,"request",json!(r));receive(&p.b,&incoming,false).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5),async {
            loop {if crate::requests::memory_read(&p.b,&bot.id).unwrap().to_string().contains("persisted effect"){break;} tokio::task::yield_now().await;}
        }).await.unwrap();
        // The completed file effect cannot be included in the SQLite completion transaction.
        tokio::time::timeout(std::time::Duration::from_secs(5),async {loop {if p.b.store.transport_status().unwrap()["items"].as_array().unwrap().iter().any(|i|i["reason"]=="transport_operation_uncertain"){break;}tokio::task::yield_now().await;}}).await.unwrap();
        crate::requests::memory_write(&p.b,&bot.id,"newer edit",None).unwrap();
        receive(&p.b,&incoming,false).unwrap();pump(&p.b).unwrap();
        assert!(crate::requests::memory_read(&p.b,&bot.id).unwrap().to_string().contains("newer edit"));
        assert!(p.b.store.transport_status().unwrap()["items"].as_array().unwrap().iter().any(|i|i["reason"]=="transport_operation_uncertain"));
    }

    #[tokio::test]
    async fn deferred_accept_failure_retains_custody_and_teardown_clears_all_transport_state() {
        let p=pair();hold(&p);let incoming=blob(&p,"request",request(&p,"defer-fault"));let sender=p.a.local_device().unwrap();
        p.b.state.lock().unwrap().devices.retain(|d|d.id!=sender.id);receive(&p.b,&incoming,true).unwrap();
        p.b.state.lock().unwrap().devices.push(sender);
        p.b.store.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_accept BEFORE INSERT ON transport_replay BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
        assert!(refresh(&p.b).is_err());assert_eq!(p.b.store.transport_deferred(&account(&p.b).unwrap()).unwrap().len(),1);assert_eq!(count(&p.b,"transport_replay"),0);
        let sync:i64=p.b.store.connection.lock().unwrap().pragma_query_value(None,"synchronous",|r|r.get(0)).unwrap();assert_eq!(sync,1);
        p.b.store.connection.lock().unwrap().execute_batch("DROP TRIGGER fail_accept;").unwrap();refresh(&p.b).unwrap();
        p.b.store.clear().unwrap();
        for table in ["transport_custody","transport_replay","transport_operations","transport_cancelled","transport_outbound"] {assert_eq!(count(&p.b,table),0);}
    }

    #[test]
    fn both_inner_request_domains_and_closed_wrapper_versions_are_enforced() {
        let p=pair();
        for verb in ["memory.read","providers.custom.preview"] {
            let mut r:Request=serde_json::from_value(request(&p,"domain")).unwrap();r.verb=verb.into();
            let payload=json!({"bot_id":"b"});let domain=if verb=="memory.read"{"beans-request-v1"}else{"beans-provider-request-v1"};
            let bytes=serde_json::to_vec(&(domain,&r.id,&r.verb,&r.requested_by,p.b.this_device_id().unwrap(),r.created_at,&payload)).unwrap();
            r.body=json!({"payload":payload,"signature":p.a.machine_file().unwrap().machine().unwrap().sign(&bytes)});
            assert!(inspect(&p.b,&blob(&p,"request",json!(r))).is_ok());
        }
        let original=blob(&p,"job_cancel",json!({"job_id":"closed"}));
        let plain=crate::crypto::unseal(&p.b.machine_file().unwrap().machine().unwrap().box_secret,&unb64(&original.ciphertext).unwrap()).unwrap();
        let wire:String=String::from_utf8(plain.clone()).unwrap();
        let malformed=wire.replacen("\"envelope_version\":1","\"envelope_version\":1,\"envelope_version\":1",1);
        for bytes in [malformed.into_bytes(),{let mut v:Value=serde_json::from_slice(&plain).unwrap();v["extra"]=json!(true);serde_json::to_vec(&v).unwrap()}] {
            let mut b=original.clone();b.ciphertext=b64(&crate::crypto::seal(&p.b.local_device().unwrap().box_pubkey,&bytes).unwrap());assert!(matches!(inspect(&p.b,&b),Err(Refusal::Rejected)));
        }
        let mut v:Value=serde_json::from_slice(&plain).unwrap();v["envelope_version"]=json!(2);
        let mut b=original;b.ciphertext=b64(&crate::crypto::seal_json(&p.b.local_device().unwrap().box_pubkey,&v).unwrap());assert!(matches!(inspect(&p.b,&b),Err(Refusal::Unsupported)));
    }
    #[tokio::test]
    async fn held_control_deferred_sender_retries_without_forward_cursor() {
        let p=pair();hold(&p);let incoming=blob(&p,"job_cancel",json!({"job_id":"held-future"}));
        let sender=p.a.local_device().unwrap();p.b.state.lock().unwrap().devices.retain(|d|d.id!=sender.id);
        receive(&p.b,&incoming,false).unwrap();assert_eq!(p.b.store.load_state().unwrap().last_seq,0);
        p.b.state.lock().unwrap().devices.push(sender);refresh(&p.b).unwrap();
        assert!(p.b.store.transport_cancelled(&account(&p.b).unwrap(),"held-future").unwrap());
        assert_eq!(p.b.store.load_state().unwrap().last_seq,0);
    }
    #[test]
    fn activation_sql_commit_never_waits_for_state_held_by_concurrent_save() {
        let p=pair();let incoming=blob(&p,"job_cancel",json!({"job_id":"lock-order"}));
        preflight_page(&p.b,std::slice::from_ref(&incoming)).unwrap();
        let prepared=Arc::new(std::sync::Barrier::new(2));
        let state_held=Arc::new(std::sync::Barrier::new(2));
        let sql_locked=Arc::new(std::sync::Barrier::new(2));
        let (attempt_tx,attempt_rx)=std::sync::mpsc::channel();
        let (done_tx,done_rx)=std::sync::mpsc::channel();
        let app=p.b.clone();let ready=prepared.clone();let held=state_held.clone();let sql=sql_locked.clone();
        let saver=std::thread::spawn(move||{
            ready.wait();let state=app.state.lock().unwrap();held.wait();sql.wait();
            attempt_tx.send(()).unwrap();
            let result=app.store.save_state(&state);
            done_tx.send(result.map_err(|e|e.to_string())).unwrap();
        });
        let (activated_tx,activated_rx)=std::sync::mpsc::channel();
        let app=p.b.clone();
        let activator=std::thread::spawn(move||{
            let result=activate_prepared(&app,&incoming,false,||{
                prepared.wait();state_held.wait();
            },||{
                sql_locked.wait();
                attempt_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("concurrent save did not reach SQLite");
                // SQLite is held here while the saver holds state. No app lock may follow.
            });
            activated_tx.send(result.map_err(|e|e.to_string())).unwrap();
        });
        activated_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("activation blocked by state/SQLite inversion").unwrap();
        done_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("save blocked by activation lock inversion").unwrap();
        saver.join().unwrap();
        activator.join().unwrap();
        assert!(p.b.store.transport_cancelled(&account(&p.b).unwrap(),"lock-order").unwrap());
        assert_eq!(p.b.store.load_state().unwrap().last_seq,0);
    }
}
