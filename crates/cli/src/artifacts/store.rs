//! Private borrowed-transaction artifact persistence. The caller owns authority and rollback.
use super::*;
use rusqlite::{params, OptionalExtension, Transaction};

#[derive(Debug)]
enum StoreError { Invalid, ReadNamespaceCannotMutate, Removed, RequestBodyChanged, Conflict { current_revision: String, current_hash: String }, Incomplete, StaleAttempt, Storage(rusqlite::Error) }
type Result<T, E = StoreError> = std::result::Result<T, E>;
impl From<rusqlite::Error> for StoreError { fn from(e: rusqlite::Error) -> Self { Self::Storage(e) } }
fn ensure(ok: bool) -> Result<()> { if ok { Ok(()) } else { Err(StoreError::Invalid) } }
#[derive(Clone, Debug, PartialEq, Eq)]
enum NamespaceKind { LocalTaskEpoch, ImportedAccount }
#[derive(Clone, Debug)]
struct ArtifactNamespace { account_id: String, namespace_key: Vec<u8>, kind: NamespaceKind }
impl ArtifactNamespace {
    fn new(account: &str, kind: NamespaceKind, value: &str) -> Result<Self> {
        ensure(structural(account) && structural(value))?;
        if kind == NamespaceKind::ImportedAccount { ensure(account == value)?; }
        let tag = match kind { NamespaceKind::LocalTaskEpoch => "local-task-epoch", NamespaceKind::ImportedAccount => "imported-account" };
        let mut key = b"beans.evidence.namespace.v1\0".to_vec();
        for part in [account, tag, value] { key.extend_from_slice(&(part.len() as u32).to_be_bytes()); key.extend_from_slice(part.as_bytes()); }
        Ok(Self { account_id: account.into(), namespace_key: key, kind })
    }
}
struct EvidenceReadView<'a> { namespace: &'a ArtifactNamespace }
struct ValidatedMutation { namespace: ArtifactNamespace, actual_account_epoch: String, owner_epoch: String, incarnation: u64, requester: String, runner_id: String }
impl ValidatedMutation {
    fn validate(&self) -> Result<()> {
        if self.namespace.kind != NamespaceKind::LocalTaskEpoch { return Err(StoreError::ReadNamespaceCannotMutate); }
        let expected = ArtifactNamespace::new(&self.namespace.account_id, NamespaceKind::LocalTaskEpoch, &self.actual_account_epoch)?;
        ensure(expected.namespace_key == self.namespace.namespace_key && structural(&self.owner_epoch) && structural(&self.requester) && structural(&self.runner_id) && self.incarnation <= i64::MAX as u64)
    }
}
#[derive(Clone, Debug)]
struct AcceptanceKey { artifact_id: String, requester: String, request_id: String, origin_key: String, fingerprint: [u8;32] }
struct ReservedInput { key: AcceptanceKey, authority: ArtifactAuthority, origin: ArtifactOrigin, revision: ArtifactRevision, snapshot_id: String, plaintext: Vec<u8>, metadata_plaintext: Vec<u8>, file_ciphertext: Vec<u8>, metadata_ciphertext: Vec<u8> }
#[derive(Debug)]
struct Reservation { key: AcceptanceKey, revision_id: String, snapshot_id: String, incarnation: u64, owner_epoch: String }
#[derive(Debug)]
enum Lookup { Absent, Reserved(Reservation), Accepted(ArtifactResult) }
struct ObservedPublication { attempt_id: String, snapshot_id: String, revision_id: String, content_hash: String }
#[derive(Clone, Copy, PartialEq, Eq)]
enum UploadPhase { Bytes, Metadata, Control }
impl UploadPhase { fn text(self) -> &'static str { match self { Self::Bytes => "bytes", Self::Metadata => "metadata", Self::Control => "control" } } }
struct IntentIdentity { blob_id: String, revision_id: Option<String>, intent_revision: u64, ciphertext_sha256: String, phase: UploadPhase }
struct Eligibility { capability_one: bool, safety_ready: bool }
#[derive(PartialEq, Eq)]
enum Scope { Chat, Lineage }
struct TombstoneInput { scope: Scope, target_id: String, chat_id: String, blob_id: String, ciphertext: Vec<u8>, cleanup_blobs: Vec<IntentIdentity> }

fn install_schema_tx(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE artifact_lineages(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,id TEXT NOT NULL,chat_id TEXT NOT NULL,runner_id TEXT NOT NULL,root_revision TEXT NOT NULL,head_revision TEXT,head_hash TEXT,graph_state TEXT NOT NULL CHECK(graph_state IN ('ready','incomplete','conflict','removed')),PRIMARY KEY(account_id,namespace_key,id));
CREATE TABLE artifact_revisions(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,revision_id TEXT NOT NULL,artifact_id TEXT NOT NULL,parent_revision TEXT,parent_hash TEXT,content_hash TEXT NOT NULL CHECK(length(content_hash)=64),size INTEGER NOT NULL CHECK(size>=0),file_id TEXT NOT NULL,record_id TEXT NOT NULL,revision_json TEXT NOT NULL,PRIMARY KEY(account_id,namespace_key,revision_id),UNIQUE(account_id,namespace_key,file_id),UNIQUE(account_id,namespace_key,record_id));
CREATE TABLE artifact_acceptances(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,requester TEXT NOT NULL,request_id TEXT NOT NULL,artifact_id TEXT NOT NULL,origin_key TEXT NOT NULL,fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),revision_id TEXT NOT NULL,snapshot_id TEXT NOT NULL,incarnation INTEGER NOT NULL,owner_epoch TEXT NOT NULL,workspace_state TEXT NOT NULL CHECK(workspace_state IN ('reserved','attempt_admitted','published','unindexed','uncertain')),attempt_id TEXT,accepted_result_json TEXT,PRIMARY KEY(account_id,namespace_key,requester,request_id),UNIQUE(account_id,namespace_key,origin_key),UNIQUE(account_id,namespace_key,revision_id));
CREATE TABLE artifact_snapshots(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,snapshot_id TEXT NOT NULL,requester TEXT NOT NULL,request_id TEXT NOT NULL,revision_id TEXT NOT NULL,revision_json BLOB NOT NULL,plaintext BLOB NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=64),metadata_plaintext BLOB NOT NULL,file_ciphertext BLOB NOT NULL,metadata_ciphertext BLOB NOT NULL,PRIMARY KEY(account_id,namespace_key,snapshot_id),UNIQUE(account_id,namespace_key,requester,request_id),UNIQUE(account_id,namespace_key,revision_id));
CREATE TABLE artifact_intents(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,blob_id TEXT NOT NULL,revision_id TEXT,artifact_id TEXT,chat_id TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('bytes','metadata','control')),ciphertext BLOB NOT NULL,ciphertext_sha256 TEXT NOT NULL,intent_revision INTEGER NOT NULL CHECK(intent_revision>0),ack_seq INTEGER,state TEXT NOT NULL CHECK(state IN ('pending','synced','blocked')),error_code TEXT,PRIMARY KEY(account_id,namespace_key,blob_id));
CREATE TABLE artifact_tombstones(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,scope TEXT NOT NULL CHECK(scope IN ('chat','lineage')),target_id TEXT NOT NULL,chat_id TEXT NOT NULL,blob_id TEXT NOT NULL,ack_seq INTEGER,PRIMARY KEY(account_id,namespace_key,scope,target_id));
CREATE TABLE artifact_cleanup(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,blob_id TEXT NOT NULL,chat_id TEXT NOT NULL,artifact_id TEXT,required_tombstone_id TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('pending','removed','blocked')),error_code TEXT,PRIMARY KEY(account_id,namespace_key,blob_id));
CREATE TABLE artifact_safety_replay(account_id TEXT NOT NULL,namespace_key BLOB NOT NULL,relay_context TEXT NOT NULL,captured_head INTEGER NOT NULL,through_seq INTEGER NOT NULL,ready INTEGER NOT NULL CHECK(ready IN (0,1)),PRIMARY KEY(account_id,namespace_key,relay_context));")?;
    Ok(())
}
fn refuse_removed(tx: &Transaction<'_>, ns: &ArtifactNamespace, chat: &str, id: &str) -> Result<()> {
    let removed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM artifact_tombstones WHERE account_id=?1 AND namespace_key=?2 AND ((scope='chat' AND target_id=?3) OR (scope='lineage' AND target_id=?4)))",params![ns.account_id,ns.namespace_key,chat,id],|r|r.get(0))?;
    if removed { Err(StoreError::Removed) } else { Ok(()) }
}
fn lookup_acceptance_tx(tx: &Transaction<'_>, view: EvidenceReadView<'_>, key: &AcceptanceKey) -> Result<Lookup> {
    let ns=view.namespace;
    let row=tx.query_row("SELECT a.requester,a.request_id,a.artifact_id,a.origin_key,a.fingerprint,a.revision_id,a.snapshot_id,a.incarnation,a.owner_epoch,a.accepted_result_json,s.revision_json FROM artifact_acceptances a JOIN artifact_snapshots s ON s.account_id=a.account_id AND s.namespace_key=a.namespace_key AND s.snapshot_id=a.snapshot_id AND s.requester=a.requester AND s.request_id=a.request_id AND s.revision_id=a.revision_id WHERE a.account_id=?1 AND a.namespace_key=?2 AND ((a.requester=?3 AND a.request_id=?4) OR a.origin_key=?5)",params![ns.account_id,ns.namespace_key,key.requester,key.request_id,key.origin_key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,u64>(7)?,r.get::<_,String>(8)?,r.get::<_,Option<String>>(9)?,r.get::<_,Vec<u8>>(10)?))).optional()?;
    let Some((requester,request,id,origin,fp,revision,snapshot,incarnation,owner,result,bytes))=row else { return Ok(Lookup::Absent) };
    let record=match ArtifactEnvelope::parse(&bytes).map_err(|_|StoreError::Invalid)? { ArtifactEnvelope::Revision(r)=>r,_=>return Err(StoreError::Invalid) };
    refuse_removed(tx,ns,&record.chat_id,&id)?;
    if requester!=key.requester || request!=key.request_id || id!=key.artifact_id || origin!=key.origin_key || fp.as_slice()!=key.fingerprint { return Err(StoreError::RequestBodyChanged); }
    match result { Some(value)=>Ok(Lookup::Accepted(serde_json::from_str(&value).map_err(|_|StoreError::Invalid)?)),None=>Ok(Lookup::Reserved(Reservation {key:key.clone(),revision_id:revision,snapshot_id:snapshot,incarnation,owner_epoch:owner})) }
}
fn validate_input(m: &ValidatedMutation, input: &ReservedInput) -> Result<Vec<u8>> {
    m.validate()?;
    let r=&input.revision;
    r.verify_bytes(&input.plaintext).map_err(|_|StoreError::Invalid)?;
    ensure(input.authority.account_id==m.namespace.account_id && input.authority.account_epoch==m.actual_account_epoch && input.authority.owner_epoch==m.owner_epoch && input.authority.incarnation==m.incarnation && input.authority.requester==m.requester && input.authority.runner_id==m.runner_id && input.key.requester==m.requester && input.key.artifact_id==r.id && structural(&input.snapshot_id) && input.file_ciphertext.len()==input.plaintext.len()+40 && input.metadata_ciphertext.len()==input.metadata_plaintext.len()+40)?;
    ensure(ArtifactEnvelope::parse(&input.metadata_plaintext).map_err(|_|StoreError::Invalid)?==ArtifactEnvelope::Revision(r.clone()))?;
    let origin=match &input.origin { ArtifactOrigin::Proposal{chat_id,bot_id,card_id}=>serde_json::json!(["proposal",chat_id,bot_id,card_id]),ArtifactOrigin::Pin{chat_id,attachment_id}=>serde_json::json!(["pin",chat_id,attachment_id]),ArtifactOrigin::UserRevision{authenticated_request_id}=>serde_json::json!(["user_revision",authenticated_request_id]) };
    ensure(serde_json::to_string(&origin).map_err(|_|StoreError::Invalid)?==input.key.origin_key)?;
    let expected=r.parent_revision.as_deref().zip(r.parent_content_hash.as_deref());
    ensure(acceptance_fingerprint(&input.authority,&input.origin,&input.key.request_id,r,expected).map_err(|_|StoreError::Invalid)?==input.key.fingerprint)?;
    serde_json::to_vec(r).map_err(|_|StoreError::Invalid)
}
fn check_head(tx: &Transaction<'_>, ns: &ArtifactNamespace, r: &ArtifactRevision) -> Result<()> {
    refuse_removed(tx,ns,&r.chat_id,&r.id)?;
    r.validate().map_err(|_|StoreError::Invalid)?;
    if let Some(parent_id)=r.parent_revision.as_deref() {
        let stored=tx.query_row("SELECT revision_json,content_hash FROM artifact_revisions WHERE account_id=?1 AND namespace_key=?2 AND artifact_id=?3 AND revision_id=?4",params![ns.account_id,ns.namespace_key,r.id,parent_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).optional()?;
        let Some((bytes,stored_hash))=stored else{return Err(StoreError::Incomplete)};
        let parent=match ArtifactEnvelope::parse(bytes.as_bytes()).map_err(|_|StoreError::Invalid)? {ArtifactEnvelope::Revision(parent)=>parent,_=>return Err(StoreError::Invalid)};
        ensure(parent.revision_id==parent_id && parent.content_hash==stored_hash)?;
        r.validate_parent(Some(&parent)).map_err(|_|StoreError::Invalid)?;
    }
    let head=tx.query_row("SELECT head_revision,head_hash,graph_state,chat_id,runner_id FROM artifact_lineages WHERE account_id=?1 AND namespace_key=?2 AND id=?3",params![ns.account_id,ns.namespace_key,r.id],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?))).optional()?;
    match head { None if r.parent_revision.is_none()=>Ok(()),None=>Err(StoreError::Incomplete),Some((rev,hash,state,chat,runner))=>{
        if state=="removed" {return Err(StoreError::Removed)}
        if state!="ready" {return Err(StoreError::Incomplete)}
        ensure(chat==r.chat_id && runner==r.runner_id)?;
        if rev==r.parent_revision && hash==r.parent_content_hash {Ok(())} else {Err(StoreError::Conflict{current_revision:rev.unwrap_or_default(),current_hash:hash.unwrap_or_default()})}
    }}
}
fn compare_snapshot(tx: &Transaction<'_>, ns: &ArtifactNamespace, input: &ReservedInput, revision_json: &[u8]) -> Result<()> {
    let same:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM artifact_snapshots WHERE account_id=?1 AND namespace_key=?2 AND snapshot_id=?3 AND requester=?4 AND request_id=?5 AND revision_id=?6 AND revision_json=?7 AND plaintext=?8 AND metadata_plaintext=?9 AND file_ciphertext=?10 AND metadata_ciphertext=?11)",params![ns.account_id,ns.namespace_key,input.snapshot_id,input.key.requester,input.key.request_id,input.revision.revision_id,revision_json,input.plaintext,input.metadata_plaintext,input.file_ciphertext,input.metadata_ciphertext],|r|r.get(0))?;
    if same {Ok(())} else {Err(StoreError::RequestBodyChanged)}
}
fn reserve_workspace_tx(tx: &Transaction<'_>, m: &ValidatedMutation, input: &ReservedInput) -> Result<Lookup> {
    let bytes=validate_input(m,input)?;
    let ns=&m.namespace;
    refuse_removed(tx,ns,&input.revision.chat_id,&input.revision.id)?;
    let previous=lookup_acceptance_tx(tx,EvidenceReadView{namespace:ns},&input.key)?;
    match previous {
        Lookup::Accepted(result)=>return Ok(Lookup::Accepted(result)),
        Lookup::Reserved(reservation)=>{compare_snapshot(tx,ns,input,&bytes)?;return Ok(Lookup::Reserved(reservation))},
        Lookup::Absent=>{}
    }
    check_head(tx,ns,&input.revision)?;
    tx.execute("INSERT INTO artifact_snapshots VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![ns.account_id,ns.namespace_key,input.snapshot_id,input.key.requester,input.key.request_id,input.revision.revision_id,bytes,input.plaintext,input.revision.content_hash,input.metadata_plaintext,input.file_ciphertext,input.metadata_ciphertext])?;
    tx.execute("INSERT INTO artifact_acceptances VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'reserved',NULL,NULL)",params![ns.account_id,ns.namespace_key,input.key.requester,input.key.request_id,input.key.artifact_id,input.key.origin_key,&input.key.fingerprint[..],input.revision.revision_id,input.snapshot_id,m.incarnation,m.owner_epoch])?;
    lookup_acceptance_tx(tx,EvidenceReadView{namespace:ns},&input.key)
}
fn admit_workspace_attempt_tx(tx: &Transaction<'_>, m: &ValidatedMutation, reservation: &Reservation, attempt: &str) -> Result<Reservation> {
    m.validate()?; ensure(structural(attempt))?;
    let ns=&m.namespace;
    let bytes:Vec<u8>=tx.query_row("SELECT revision_json FROM artifact_snapshots WHERE account_id=?1 AND namespace_key=?2 AND snapshot_id=?3 AND requester=?4 AND request_id=?5 AND revision_id=?6",params![ns.account_id,ns.namespace_key,reservation.snapshot_id,reservation.key.requester,reservation.key.request_id,reservation.revision_id],|r|r.get(0))?;
    let r:ArtifactRevision=serde_json::from_slice(&bytes).map_err(|_|StoreError::Invalid)?;
    check_head(tx,ns,&r)?;
    if reservation.incarnation!=m.incarnation || reservation.owner_epoch!=m.owner_epoch || reservation.key.requester!=m.requester {return Err(StoreError::StaleAttempt)}
    let changed=tx.execute("UPDATE artifact_acceptances SET workspace_state='attempt_admitted',attempt_id=?6 WHERE account_id=?1 AND namespace_key=?2 AND requester=?3 AND request_id=?4 AND snapshot_id=?5 AND workspace_state='reserved' AND incarnation=?7 AND owner_epoch=?8",params![ns.account_id,ns.namespace_key,m.requester,reservation.key.request_id,reservation.snapshot_id,attempt,m.incarnation,m.owner_epoch])?;
    if changed!=1 {return Err(StoreError::StaleAttempt)}
    match lookup_acceptance_tx(tx,EvidenceReadView{namespace:ns},&reservation.key)? {Lookup::Reserved(r)=>Ok(r),_=>Err(StoreError::StaleAttempt)}
}
fn insert_intent(tx: &Transaction<'_>, ns: &ArtifactNamespace, blob: &str, revision: Option<&str>, artifact: Option<&str>, chat: &str, phase: UploadPhase, ciphertext: &[u8]) -> Result<()> {
    tx.execute("INSERT INTO artifact_intents VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,NULL,'pending',NULL)",params![ns.account_id,ns.namespace_key,blob,revision,artifact,chat,phase.text(),ciphertext,sha256_hex(ciphertext)])?; Ok(())
}
fn commit_artifact_acceptance_tx(tx: &Transaction<'_>, m: &ValidatedMutation, input: &ReservedInput, proof: &ObservedPublication) -> Result<ArtifactResult> {
    let bytes=validate_input(m,input)?; let ns=&m.namespace; let r=&input.revision;
    refuse_removed(tx,ns,&r.chat_id,&r.id)?;
    if let Lookup::Accepted(result)=lookup_acceptance_tx(tx,EvidenceReadView{namespace:ns},&input.key)? {return Ok(result)}
    compare_snapshot(tx,ns,input,&bytes)?;
    let admitted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM artifact_acceptances WHERE account_id=?1 AND namespace_key=?2 AND requester=?3 AND request_id=?4 AND snapshot_id=?5 AND revision_id=?6 AND attempt_id=?7 AND incarnation=?8 AND owner_epoch=?9 AND workspace_state='attempt_admitted')",params![ns.account_id,ns.namespace_key,m.requester,input.key.request_id,proof.snapshot_id,proof.revision_id,proof.attempt_id,m.incarnation,m.owner_epoch],|r|r.get(0))?;
    ensure(proof.snapshot_id==input.snapshot_id && proof.revision_id==r.revision_id && proof.content_hash==r.content_hash)?;
    if !admitted {return Err(StoreError::StaleAttempt)}
    check_head(tx,ns,r)?;
    let record_id=format!("{}.record",r.revision_id);
    tx.execute("INSERT INTO artifact_revisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![ns.account_id,ns.namespace_key,r.revision_id,r.id,r.parent_revision,r.parent_content_hash,r.content_hash,r.size,r.file_id,record_id,std::str::from_utf8(&bytes).map_err(|_|StoreError::Invalid)?])?;
    if r.parent_revision.is_none() {tx.execute("INSERT INTO artifact_lineages VALUES(?1,?2,?3,?4,?5,?6,?6,?7,'ready')",params![ns.account_id,ns.namespace_key,r.id,r.chat_id,r.runner_id,r.revision_id,r.content_hash])?;} else {
        let changed=tx.execute("UPDATE artifact_lineages SET head_revision=?4,head_hash=?5 WHERE account_id=?1 AND namespace_key=?2 AND id=?3 AND head_revision=?6 AND head_hash=?7 AND graph_state='ready'",params![ns.account_id,ns.namespace_key,r.id,r.revision_id,r.content_hash,r.parent_revision,r.parent_content_hash])?; if changed!=1 {return Err(StoreError::StaleAttempt)}
    }
    insert_intent(tx,ns,&r.file_id,Some(&r.revision_id),Some(&r.id),&r.chat_id,UploadPhase::Bytes,&input.file_ciphertext)?;
    insert_intent(tx,ns,&record_id,Some(&r.revision_id),Some(&r.id),&r.chat_id,UploadPhase::Metadata,&input.metadata_ciphertext)?;
    let result=ArtifactResult{id:r.id.clone(),revision_id:r.revision_id.clone(),content_hash:r.content_hash.clone(),workspace:WorkspaceState::Published,sync:SyncState::Pending};
    tx.execute("UPDATE artifact_acceptances SET workspace_state='published',accepted_result_json=?5 WHERE account_id=?1 AND namespace_key=?2 AND requester=?3 AND request_id=?4",params![ns.account_id,ns.namespace_key,m.requester,input.key.request_id,serde_json::to_string(&result).map_err(|_|StoreError::Invalid)?])?;
    Ok(result)
}
fn eligible_intent_tx(tx: &Transaction<'_>, view: EvidenceReadView<'_>, identity: &IntentIdentity, gates: &Eligibility) -> Result<bool> {
    if !gates.capability_one || !gates.safety_ready {return Ok(false)}
    let ns=view.namespace;
    let row=tx.query_row("SELECT chat_id,artifact_id FROM artifact_intents WHERE account_id=?1 AND namespace_key=?2 AND blob_id=?3 AND revision_id IS ?4 AND intent_revision=?5 AND ciphertext_sha256=?6 AND phase=?7 AND state='pending'",params![ns.account_id,ns.namespace_key,identity.blob_id,identity.revision_id,identity.intent_revision,identity.ciphertext_sha256,identity.phase.text()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional()?;
    let Some((chat,id))=row else{return Ok(false)};
    if identity.phase!=UploadPhase::Control {refuse_removed(tx,ns,&chat,id.as_deref().unwrap_or(""))?;}
    if identity.phase!=UploadPhase::Metadata {return Ok(true)}
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM artifact_intents WHERE account_id=?1 AND namespace_key=?2 AND revision_id IS ?3 AND phase='bytes' AND state='synced' AND ack_seq>0)",params![ns.account_id,ns.namespace_key,identity.revision_id],|r|r.get(0))?)
}
fn ack_bytes_tx(tx: &Transaction<'_>, m: &ValidatedMutation, identity: &IntentIdentity, seq: i64) -> Result<bool> {
    m.validate()?; ensure(seq>0 && identity.phase==UploadPhase::Bytes)?;
    let ns=&m.namespace;
    let row=tx.query_row("SELECT chat_id,artifact_id,state,ack_seq FROM artifact_intents WHERE account_id=?1 AND namespace_key=?2 AND blob_id=?3 AND revision_id IS ?4 AND intent_revision=?5 AND ciphertext_sha256=?6 AND phase='bytes'",params![ns.account_id,ns.namespace_key,identity.blob_id,identity.revision_id,identity.intent_revision,identity.ciphertext_sha256],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<i64>>(3)?))).optional()?;
    let Some((chat,id,state,ack))=row else{return Err(StoreError::StaleAttempt)};
    refuse_removed(tx,ns,&chat,id.as_deref().unwrap_or(""))?;
    if state=="synced" {return if ack==Some(seq) {Ok(false)} else {Err(StoreError::StaleAttempt)}}
    if state!="pending" {return Err(StoreError::StaleAttempt)}
    Ok(tx.execute("UPDATE artifact_intents SET state='synced',ack_seq=?4 WHERE account_id=?1 AND namespace_key=?2 AND blob_id=?3 AND state='pending'",params![ns.account_id,ns.namespace_key,identity.blob_id,seq])?==1)
}
fn eligible_cleanup_tx(tx: &Transaction<'_>, view: EvidenceReadView<'_>, blob_id: &str) -> Result<bool> {
    let ns=view.namespace;
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM artifact_cleanup c JOIN artifact_intents i ON i.account_id=c.account_id AND i.namespace_key=c.namespace_key AND i.blob_id=c.required_tombstone_id WHERE c.account_id=?1 AND c.namespace_key=?2 AND c.blob_id=?3 AND c.state='pending' AND i.phase='control' AND i.state='synced' AND i.ack_seq>0)",params![ns.account_id,ns.namespace_key,blob_id],|r|r.get(0))?)
}
fn install_tombstone_tx(tx: &Transaction<'_>, m: &ValidatedMutation, control: &TombstoneInput) -> Result<()> {
    m.validate()?; ensure(structural(&control.chat_id) && !control.ciphertext.is_empty() && control.ciphertext.len()<=MAX_METADATA_BYTES+40)?;
    let scope=match control.scope {Scope::Chat=>"chat",Scope::Lineage=>"lineage"};
    let expected=if control.scope==Scope::Chat {ensure(control.target_id==control.chat_id)?;removed_chat_id(&control.chat_id).map_err(|_|StoreError::Invalid)?} else {ensure(artifact_id(&control.target_id))?;format!("{}.removed",control.target_id)};
    ensure(control.blob_id==expected)?; let ns=&m.namespace;
    let old=tx.query_row("SELECT blob_id,chat_id FROM artifact_tombstones WHERE account_id=?1 AND namespace_key=?2 AND scope=?3 AND target_id=?4",params![ns.account_id,ns.namespace_key,scope,control.target_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
    if let Some((id,chat))=old {
        let bytes:Vec<u8>=tx.query_row("SELECT ciphertext FROM artifact_intents WHERE account_id=?1 AND namespace_key=?2 AND blob_id=?3 AND phase='control'",params![ns.account_id,ns.namespace_key,id],|r|r.get(0))?;
        if id!=control.blob_id || chat!=control.chat_id || bytes!=control.ciphertext {return Err(StoreError::RequestBodyChanged)}
    } else {
        tx.execute("INSERT INTO artifact_tombstones VALUES(?1,?2,?3,?4,?5,?6,NULL)",params![ns.account_id,ns.namespace_key,scope,control.target_id,control.chat_id,control.blob_id])?;
        insert_intent(tx,ns,&control.blob_id,None,if control.scope==Scope::Lineage{Some(&control.target_id)}else{None},&control.chat_id,UploadPhase::Control,&control.ciphertext)?;
    }
    for identity in &control.cleanup_blobs {
        ensure(identity.phase!=UploadPhase::Control && identity.blob_id!=control.blob_id)?;
        let owned=tx.query_row("SELECT i.chat_id,i.artifact_id,r.revision_json,r.file_id,r.record_id FROM artifact_intents i JOIN artifact_revisions r ON r.account_id=i.account_id AND r.namespace_key=i.namespace_key AND r.revision_id=i.revision_id AND r.artifact_id=i.artifact_id WHERE i.account_id=?1 AND i.namespace_key=?2 AND i.blob_id=?3 AND i.revision_id IS ?4 AND i.intent_revision=?5 AND i.ciphertext_sha256=?6 AND i.phase=?7 AND i.phase IN ('bytes','metadata')",params![ns.account_id,ns.namespace_key,identity.blob_id,identity.revision_id,identity.intent_revision,identity.ciphertext_sha256,identity.phase.text()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).optional()?;
        let Some((chat,artifact,bytes,file,record))=owned else{return Err(StoreError::Invalid)};
        let revision=match ArtifactEnvelope::parse(bytes.as_bytes()).map_err(|_|StoreError::Invalid)? {ArtifactEnvelope::Revision(r)=>r,_=>return Err(StoreError::Invalid)};
        ensure(chat==control.chat_id && revision.chat_id==chat && revision.id==artifact && identity.revision_id.as_deref()==Some(revision.revision_id.as_str()) && file==revision.file_id && record==format!("{}.record",revision.revision_id) && identity.blob_id==if identity.phase==UploadPhase::Bytes{file}else{record} && (control.scope==Scope::Chat || artifact==control.target_id))?;
        let previous=tx.query_row("SELECT chat_id,artifact_id,required_tombstone_id FROM artifact_cleanup WHERE account_id=?1 AND namespace_key=?2 AND blob_id=?3",params![ns.account_id,ns.namespace_key,identity.blob_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((old_chat,old_artifact,old_control))=previous {
            if old_chat!=chat || old_artifact.as_deref()!=Some(artifact.as_str()) || old_control!=control.blob_id {return Err(StoreError::RequestBodyChanged)}
        } else {
            tx.execute("INSERT INTO artifact_cleanup VALUES(?1,?2,?3,?4,?5,?6,'pending',NULL)",params![ns.account_id,ns.namespace_key,identity.blob_id,chat,artifact,control.blob_id])?;
        }
    }
    tx.execute("UPDATE artifact_lineages SET graph_state='removed' WHERE account_id=?1 AND namespace_key=?2 AND ((?3='chat' AND chat_id=?4) OR (?3='lineage' AND id=?5))",params![ns.account_id,ns.namespace_key,scope,control.chat_id,control.target_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, TransactionBehavior};
    fn input() -> (ValidatedMutation,ReservedInput,ObservedPublication) {
        let ns=ArtifactNamespace::new("account",NamespaceKind::LocalTaskEpoch,"epoch").unwrap();
        let m=ValidatedMutation{namespace:ns,actual_account_epoch:"epoch".into(),owner_epoch:"owner".into(),incarnation:1,requester:"requester".into(),runner_id:"runner".into()};
        let authority=ArtifactAuthority{account_id:"account".into(),account_epoch:"epoch".into(),owner_epoch:"owner".into(),incarnation:1,requester:"requester".into(),runner_id:"runner".into()};
        let origin=ArtifactOrigin::Proposal{chat_id:"chat".into(),bot_id:"bot".into(),card_id:"card".into()};
        let revision=ArtifactRevision{version:1,id:proposal_id("chat","bot","card").unwrap(),revision_id:"00000000-0000-4000-8000-000000000001".into(),name:"note".into(),mime:"text/plain".into(),size:0,content_hash:sha256_hex(b""),parent_revision:None,parent_content_hash:None,file_id:"00000000-0000-4000-8000-000000000001.file".into(),chat_id:"chat".into(),bot_id:Some("bot".into()),runner_id:"runner".into(),card_id:Some("card".into()),attachment_id:None,created_at:1.0,path:Some("note.md".into())};
        let key=AcceptanceKey{artifact_id:revision.id.clone(),requester:"requester".into(),request_id:"request".into(),origin_key:"[\"proposal\",\"chat\",\"bot\",\"card\"]".into(),fingerprint:acceptance_fingerprint(&authority,&origin,"request",&revision,None).unwrap()};
        let metadata=serde_json::to_vec(&revision).unwrap();
        let proof=ObservedPublication{attempt_id:"attempt".into(),snapshot_id:"snapshot".into(),revision_id:revision.revision_id.clone(),content_hash:revision.content_hash.clone()};
        let input=ReservedInput{key,authority,origin,revision,snapshot_id:"snapshot".into(),plaintext:vec![],file_ciphertext:vec![1;40],metadata_ciphertext:vec![2;metadata.len()+40],metadata_plaintext:metadata};
        (m,input,proof)
    }
    fn count(tx: &Transaction<'_>, table: &str) -> i64 {tx.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get(0)).unwrap()}
    #[test]
    fn borrowed_transactions_preserve_namespace_and_acceptance() {
        let mut db=Connection::open_in_memory().unwrap();
        let (m,mut input,proof)=input();
        let imported=ArtifactNamespace::new("account",NamespaceKind::ImportedAccount,"account").unwrap();
        let mut exact=b"beans.evidence.namespace.v1\0".to_vec();
        for s in ["account","local-task-epoch","epoch"] {exact.extend_from_slice(&(s.len() as u32).to_be_bytes());exact.extend_from_slice(s.as_bytes());}
        assert_eq!(m.namespace.namespace_key,exact);assert_ne!(exact,imported.namespace_key);
        let tx=db.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();install_schema_tx(&tx).unwrap();tx.commit().unwrap();
        let tx=db.transaction().unwrap();reserve_workspace_tx(&tx,&m,&input).unwrap();tx.rollback().unwrap();
        let tx=db.transaction().unwrap();assert_eq!(count(&tx,"artifact_snapshots"),0);assert!(matches!(reserve_workspace_tx(&tx,&m,&input).unwrap(),Lookup::Reserved(_)));tx.commit().unwrap();
        let tx=db.transaction().unwrap();let Lookup::Reserved(reservation)=lookup_acceptance_tx(&tx,EvidenceReadView{namespace:&m.namespace},&input.key).unwrap() else{panic!("reservation")};admit_workspace_attempt_tx(&tx,&m,&reservation,"attempt").unwrap();tx.commit().unwrap();
        let tx=db.transaction().unwrap();assert!(matches!(lookup_acceptance_tx(&tx,EvidenceReadView{namespace:&imported},&input.key).unwrap(),Lookup::Absent));tx.rollback().unwrap();
        let imported_mutation=ValidatedMutation{namespace:imported,actual_account_epoch:"epoch".into(),owner_epoch:"owner".into(),incarnation:1,requester:"requester".into(),runner_id:"runner".into()};
        let tx=db.transaction().unwrap();assert!(matches!(reserve_workspace_tx(&tx,&imported_mutation,&input),Err(StoreError::ReadNamespaceCannotMutate)));tx.rollback().unwrap();
        for change in 0..3 {
            let original_time=input.revision.created_at;let original_metadata=input.metadata_plaintext.clone();
            if change==0 {input.revision.created_at=2.0;input.metadata_plaintext=serde_json::to_vec(&input.revision).unwrap();}
            if change==1 {input.file_ciphertext[0]^=1;}
            if change==2 {input.metadata_ciphertext[0]^=1;}
            let tx=db.transaction().unwrap();assert!(matches!(commit_artifact_acceptance_tx(&tx,&m,&input,&proof),Err(StoreError::RequestBodyChanged)));tx.rollback().unwrap();
            input.revision.created_at=original_time;input.metadata_plaintext=original_metadata;
            if change==1 {input.file_ciphertext[0]^=1;}if change==2 {input.metadata_ciphertext[0]^=1;}
            let tx=db.transaction().unwrap();assert_eq!(count(&tx,"artifact_lineages"),0);assert_eq!(count(&tx,"artifact_intents"),0);let accepted:i64=tx.query_row("SELECT count(*) FROM artifact_acceptances WHERE accepted_result_json IS NOT NULL",[],|r|r.get(0)).unwrap();assert_eq!(accepted,0);tx.rollback().unwrap();
        }
        let tx=db.transaction().unwrap();tx.execute_batch("CREATE TRIGGER fail_metadata BEFORE INSERT ON artifact_intents WHEN NEW.phase='metadata' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();tx.commit().unwrap();
        let tx=db.transaction().unwrap();assert!(matches!(commit_artifact_acceptance_tx(&tx,&m,&input,&proof),Err(StoreError::Storage(_))));tx.rollback().unwrap();
        let tx=db.transaction().unwrap();assert_eq!(count(&tx,"artifact_lineages"),0);assert_eq!(count(&tx,"artifact_revisions"),0);assert_eq!(count(&tx,"artifact_intents"),0);tx.execute_batch("DROP TRIGGER fail_metadata").unwrap();let result=commit_artifact_acceptance_tx(&tx,&m,&input,&proof).unwrap();tx.commit().unwrap();
        let tx=db.transaction().unwrap();assert_eq!(commit_artifact_acceptance_tx(&tx,&m,&input,&proof).unwrap(),result);let stored:Vec<u8>=tx.query_row("SELECT ciphertext FROM artifact_intents WHERE phase='bytes'",[],|r|r.get(0)).unwrap();assert_eq!(stored,input.file_ciphertext);let stored_metadata:Vec<u8>=tx.query_row("SELECT ciphertext FROM artifact_intents WHERE phase='metadata'",[],|r|r.get(0)).unwrap();assert_eq!(stored_metadata,input.metadata_ciphertext);tx.rollback().unwrap();
        let mut changed=input.key.clone();changed.fingerprint[0]^=1;let tx=db.transaction().unwrap();assert!(matches!(lookup_acceptance_tx(&tx,EvidenceReadView{namespace:&m.namespace},&changed),Err(StoreError::RequestBodyChanged)));tx.rollback().unwrap();
        let original_revision=input.revision.clone();let original_metadata=input.metadata_plaintext.clone();let original_file=input.file_ciphertext.clone();let original_cipher=input.metadata_ciphertext.clone();
        input.revision.revision_id="00000000-0000-4000-8000-000000000002".into();input.revision.file_id=format!("{}.file",input.revision.revision_id);input.revision.created_at=9.0;input.snapshot_id="regenerated".into();input.metadata_plaintext=serde_json::to_vec(&input.revision).unwrap();input.file_ciphertext=vec![7;40];input.metadata_ciphertext=vec![8;input.metadata_plaintext.len()+40];
        let tx=db.transaction().unwrap();let Lookup::Accepted(retry)=reserve_workspace_tx(&tx,&m,&input).unwrap() else{panic!("accepted retry")};assert_eq!(retry,result);assert_eq!(count(&tx,"artifact_snapshots"),1);assert_eq!(count(&tx,"artifact_revisions"),1);assert_eq!(count(&tx,"artifact_intents"),2);let stored:String=tx.query_row("SELECT revision_json FROM artifact_revisions",[],|r|r.get(0)).unwrap();let persisted:ArtifactRevision=serde_json::from_str(&stored).unwrap();assert_eq!(persisted.created_at,1.0);tx.rollback().unwrap();
        input.revision=original_revision;input.metadata_plaintext=original_metadata;input.file_ciphertext=original_file;input.metadata_ciphertext=original_cipher;input.snapshot_id="snapshot".into();
        let mut child=input.revision.clone();child.revision_id="00000000-0000-4000-8000-000000000003".into();child.file_id=format!("{}.file",child.revision_id);child.parent_revision=Some(input.revision.revision_id.clone());child.parent_content_hash=Some(input.revision.content_hash.clone());
        let tx=db.transaction().unwrap();check_head(&tx,&m.namespace,&child).unwrap();tx.execute("DELETE FROM artifact_revisions",[]).unwrap();assert!(matches!(check_head(&tx,&m.namespace,&child),Err(StoreError::Incomplete)));tx.rollback().unwrap();
        let tx=db.transaction().unwrap();let mut wrong=input.revision.clone();wrong.chat_id="foreign".into();tx.execute("UPDATE artifact_revisions SET revision_json=?1",[serde_json::to_string(&wrong).unwrap()]).unwrap();assert!(matches!(check_head(&tx,&m.namespace,&child),Err(StoreError::Invalid)));tx.rollback().unwrap();
        let child_origin=ArtifactOrigin::UserRevision{authenticated_request_id:"child-request".into()};let child_key=AcceptanceKey{artifact_id:child.id.clone(),requester:m.requester.clone(),request_id:"child-request".into(),origin_key:"[\"user_revision\",\"child-request\"]".into(),fingerprint: {let mut user=child.clone();user.card_id=None;acceptance_fingerprint(&input.authority,&child_origin,"child-request",&user,Some((&input.revision.revision_id,&input.revision.content_hash))).unwrap()}};
        child.card_id=None;let child_metadata=serde_json::to_vec(&child).unwrap();let child_input=ReservedInput{key:child_key,authority:input.authority.clone(),origin:child_origin,revision:child.clone(),snapshot_id:"child-snapshot".into(),plaintext:vec![],metadata_ciphertext:vec![4;child_metadata.len()+40],metadata_plaintext:child_metadata,file_ciphertext:vec![5;40]};let child_proof=ObservedPublication{attempt_id:"child-attempt".into(),snapshot_id:"child-snapshot".into(),revision_id:child.revision_id.clone(),content_hash:child.content_hash.clone()};
        let tx=db.transaction().unwrap();let Lookup::Reserved(reserved)=reserve_workspace_tx(&tx,&m,&child_input).unwrap() else{panic!("child")};admit_workspace_attempt_tx(&tx,&m,&reserved,"child-attempt").unwrap();commit_artifact_acceptance_tx(&tx,&m,&child_input,&child_proof).unwrap();tx.commit().unwrap();
        let byte=IntentIdentity{blob_id:input.revision.file_id.clone(),revision_id:Some(input.revision.revision_id.clone()),intent_revision:1,ciphertext_sha256:sha256_hex(&input.file_ciphertext),phase:UploadPhase::Bytes};
        let metadata=IntentIdentity{blob_id:format!("{}.record",input.revision.revision_id),revision_id:byte.revision_id.clone(),intent_revision:1,ciphertext_sha256:sha256_hex(&input.metadata_ciphertext),phase:UploadPhase::Metadata};let gates=Eligibility{capability_one:true,safety_ready:true};
        let tx=db.transaction().unwrap();assert!(!eligible_intent_tx(&tx,EvidenceReadView{namespace:&m.namespace},&metadata,&gates).unwrap());assert!(ack_bytes_tx(&tx,&m,&byte,10).unwrap());assert!(eligible_intent_tx(&tx,EvidenceReadView{namespace:&m.namespace},&metadata,&gates).unwrap());tx.commit().unwrap();
        let other=TombstoneInput{scope:Scope::Chat,target_id:"other-chat".into(),chat_id:"other-chat".into(),blob_id:removed_chat_id("other-chat").unwrap(),ciphertext:vec![6;40],cleanup_blobs:vec![]};let tx=db.transaction().unwrap();install_tombstone_tx(&tx,&m,&other).unwrap();tx.commit().unwrap();
        let hostile=TombstoneInput{scope:Scope::Chat,target_id:"chat".into(),chat_id:"chat".into(),blob_id:removed_chat_id("chat").unwrap(),ciphertext:vec![3;40],cleanup_blobs:vec![IntentIdentity{blob_id:other.blob_id.clone(),revision_id:None,intent_revision:1,ciphertext_sha256:sha256_hex(&other.ciphertext),phase:UploadPhase::Control}]};let tx=db.transaction().unwrap();assert!(matches!(install_tombstone_tx(&tx,&m,&hostile),Err(StoreError::Invalid)));tx.rollback().unwrap();
        let cross=TombstoneInput{scope:Scope::Chat,target_id:"other-chat".into(),chat_id:"other-chat".into(),blob_id:other.blob_id.clone(),ciphertext:other.ciphertext.clone(),cleanup_blobs:vec![IntentIdentity{blob_id:byte.blob_id.clone(),revision_id:byte.revision_id.clone(),intent_revision:1,ciphertext_sha256:byte.ciphertext_sha256.clone(),phase:UploadPhase::Bytes}]};let tx=db.transaction().unwrap();assert!(matches!(install_tombstone_tx(&tx,&m,&cross),Err(StoreError::Invalid)));tx.rollback().unwrap();
        let tomb=TombstoneInput{scope:Scope::Chat,target_id:"chat".into(),chat_id:"chat".into(),blob_id:removed_chat_id("chat").unwrap(),ciphertext:vec![3;40],cleanup_blobs:vec![byte]};
        let tx=db.transaction().unwrap();install_tombstone_tx(&tx,&m,&tomb).unwrap();tx.rollback().unwrap();let tx=db.transaction().unwrap();assert_eq!(count(&tx,"artifact_tombstones"),1);assert_eq!(count(&tx,"artifact_cleanup"),0);install_tombstone_tx(&tx,&m,&tomb).unwrap();install_tombstone_tx(&tx,&m,&tomb).unwrap();tx.commit().unwrap();
        let tx=db.transaction().unwrap();assert!(!eligible_cleanup_tx(&tx,EvidenceReadView{namespace:&m.namespace},&tomb.cleanup_blobs[0].blob_id).unwrap());tx.execute("UPDATE artifact_intents SET state='synced',ack_seq=11 WHERE blob_id=?1 AND phase='control'",[&tomb.blob_id]).unwrap();assert!(eligible_cleanup_tx(&tx,EvidenceReadView{namespace:&m.namespace},&tomb.cleanup_blobs[0].blob_id).unwrap());tx.commit().unwrap();
        let tx=db.transaction().unwrap();assert!(matches!(commit_artifact_acceptance_tx(&tx,&m,&input,&proof),Err(StoreError::Removed)));assert!(matches!(eligible_intent_tx(&tx,EvidenceReadView{namespace:&m.namespace},&metadata,&gates),Err(StoreError::Removed)));assert_eq!(count(&tx,"artifact_cleanup"),1);tx.rollback().unwrap();
    }
}
