//! Private durable admission receipt, including negative consent. Never synced or model input.
use serde::{Deserialize,Serialize};
use rusqlite::{params,OptionalExtension};
use super::{dispatch::{self,TurnAccess},queue::Delivery,types::*};
use crate::{app::App,local_store::LocalStore,model::{Author,Body,Job}};
use tokio_util::sync::CancellationToken;

#[derive(Clone,Serialize,Deserialize)]
pub(crate) struct AdmittedText {pub source:Source,pub text:String}
#[derive(Clone,Serialize,Deserialize)]
pub(crate) struct Admission {
    pub bot_id:String,pub chat_id:String,pub trigger_id:String,
    pub scope:Option<MemoryScope>,pub consent_revision:Option<Revision>,pub group:bool,pub cap:u32,
    pub trigger:Option<AdmittedText>,pub captured:bool,
}
fn storage()->MemoryError{MemoryError::new("memory_storage_failed")}
impl LocalStore {
    pub(crate) fn memory_admission(&self,id:&str)->Result<Option<Admission>,MemoryError>{
        let json:Option<String>=self.connection.lock().unwrap().query_row("SELECT json FROM memory_turn_admissions WHERE job_id=?1",[id],|r|r.get(0)).optional().map_err(|_|storage())?;
        json.map(|v|serde_json::from_str(&v).map_err(|_|storage())).transpose()
    }
    fn admit_memory_turn(&self,id:&str,record:&Admission)->Result<Admission,MemoryError>{
        self.connection.lock().unwrap().execute("INSERT OR IGNORE INTO memory_turn_admissions(job_id,json) VALUES(?1,?2)",
            params![id,serde_json::to_string(record).map_err(|_|storage())?]).map_err(|_|storage())?;
        self.memory_admission(id)?.ok_or_else(storage)
    }
    pub(crate) fn enqueue_admitted_memory(&self,d:&Delivery,turn_id:&str)->Result<(),MemoryError>{
        let mut connection=self.connection.lock().unwrap();let tx=connection.transaction().map_err(|_|storage())?;
        let json:String=tx.query_row("SELECT json FROM memory_turn_admissions WHERE job_id=?1",[turn_id],|r|r.get(0)).map_err(|_|storage())?;
        let mut record:Admission=serde_json::from_str(&json).map_err(|_|storage())?;
        if record.captured{return Err(MemoryError::new("turn_already_captured"));}
        if record.bot_id!=d.bot_id||record.scope.as_ref()!=Some(&d.scope)||record.consent_revision!=d.consent_revision||record.consent_revision.is_none(){
            return Err(MemoryError::new("consent_changed"));
        }
        let prior:Option<String>=tx.query_row("SELECT json FROM memory_deliveries WHERE id=?1",[&d.id],|r|r.get(0)).optional().map_err(|_|storage())?;
        if let Some(json)=prior{let old:Delivery=serde_json::from_str(&json).map_err(|_|storage())?;
            if old.document!=d.document||old.scope!=d.scope||old.bot_id!=d.bot_id{return Err(MemoryError::new("request_id_conflict"));}
        }else{
            tx.execute("INSERT INTO memory_deliveries(id,bot_id,state,json) VALUES(?1,?2,'queued',?3)",params![d.id,d.bot_id,serde_json::to_string(d).map_err(|_|storage())?]).map_err(|_|storage())?;
        }
        record.captured=true;record.trigger=None;
        tx.execute("UPDATE memory_turn_admissions SET json=?2 WHERE job_id=?1",params![turn_id,serde_json::to_string(&record).map_err(|_|storage())?]).map_err(|_|storage())?;
        tx.commit().map_err(|_|storage())?;Ok(())
    }
}
pub fn admit_turn(app:&App,job:&Job,cancel:&CancellationToken)->Result<Option<TurnAccess>,MemoryError>{
    let _edit=app.roster_edit.lock().unwrap();
    let record=if let Some(record)=app.store.memory_admission(&job.id)?{record}else{
        let group=app.chat(&job.chat_id).is_some_and(|c|c.meta.is_group());
        let access=match dispatch::admit(app,&job.bot_id,group,cancel){Ok(a)=>Some(a),Err(e)if matches!(e.code.as_str(),"memory_not_configured"|"connection_disconnected")=>None,Err(e)=>return Err(e)};
        let trigger=if access.as_ref().is_some_and(|a|a.consent_revision.is_some()&&a.capture_cap>0){
            app.message(&job.chat_id,&job.trigger_message_id).filter(|m|m.is_complete()).and_then(|m|match (m.author,m.body){
                (Author::You,Body::Text{text,..})if !text.is_empty()&&text.len()<=32768=>Some(AdmittedText{source:Source{chat_id:job.chat_id.clone(),message_id:job.trigger_message_id.clone(),speaker:Speaker::User},text:crate::memory::scrub(&text)}),_=>None})
        }else{None};
        app.store.admit_memory_turn(&job.id,&Admission{bot_id:job.bot_id.clone(),chat_id:job.chat_id.clone(),trigger_id:job.trigger_message_id.clone(),
            scope:access.as_ref().map(|a|a.scope.clone()),consent_revision:access.as_ref().and_then(|a|a.consent_revision.clone()),group,
            cap:access.as_ref().map_or(0,|a|a.capture_cap),trigger,captured:false})?
    };
    if record.bot_id!=job.bot_id||record.chat_id!=job.chat_id||record.trigger_id!=job.trigger_message_id{return Err(MemoryError::new("turn_identity_conflict"));}
    let Some(scope)=record.scope else{return Ok(None)};
    let mut access=dispatch::admit(app,&job.bot_id,record.group,cancel)?;
    if access.scope!=scope{return Err(MemoryError::new("authority_changed"));}
    access.consent_revision=if record.captured{None}else{record.consent_revision};
    access.capture_cap=record.cap;access.turn_id=Some(job.id.clone());
    Ok(Some(access))
}
