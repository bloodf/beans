use super::*;
use crate::memory_service::types::EmbeddingProfile;
use reqwest::{header::{HeaderValue, AUTHORIZATION}, redirect::Policy, Client};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

pub struct ApiEmbedding {
    spec: EmbeddingSpec,
    client: Client,
    authorization: Option<HeaderValue>,
    limits: EmbeddingLimits,
}
impl ApiEmbedding {
    /// `endpoint` is the exact trusted /embeddings URL, not an inferred provider root.
    /// Calling new makes no network request. Private HTTP requires explicit UI approval.
    pub fn new(profile: &EmbeddingProfile, allow_insecure_http: bool, limits: EmbeddingLimits) -> Result<Self, EmbeddingError> {
        let spec=EmbeddingSpec::from_profile(profile)?; limits.validate()?;
        if spec.mode != EmbeddingMode::Api { return Err(EmbeddingError::InvalidProfile); }
        if spec.endpoint().ok_or(EmbeddingError::InvalidProfile)?.scheme() != "https" && !allow_insecure_http {
            return Err(EmbeddingError::InvalidProfile);
        }
        let authorization=profile.secret.as_ref().map(|key| {
            if key.is_empty() || key.len()>8192 { return Err(EmbeddingError::InvalidProfile); }
            let mut header=HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_|EmbeddingError::InvalidProfile)?;
            header.set_sensitive(true); Ok(header)
        }).transpose()?;
        let client=beans_tls::client_builder().redirect(Policy::none()).timeout(limits.timeout)
            .connect_timeout(limits.timeout).build().map_err(|_|EmbeddingError::Transport)?;
        Ok(Self {spec,client,authorization,limits})
    }
    async fn send(&self, inputs: Vec<String>) -> Result<EmbeddingBatch, EmbeddingError> {
        let count=inputs.len();
        let body=serde_json::json!({"model":self.spec.model(),"input":inputs,
            "dimensions":self.spec.dimensions,"encoding_format":"float"});
        let mut request=self.client.post(self.spec.endpoint().ok_or(EmbeddingError::InvalidProfile)?.clone()).json(&body);
        if let Some(header)=&self.authorization {request=request.header(AUTHORIZATION,header.clone());}
        let response=request.send().await.map_err(transport_error)?;
        let bytes=bounded_response(response,self.limits.max_response_bytes).await?;
        decode_response(&self.spec,&bytes,count)
    }
}
#[async_trait::async_trait]
impl Embedding for ApiEmbedding {
    fn fingerprint(&self) -> &EmbeddingFingerprint {self.spec.fingerprint()}
    async fn embed(&self,purpose:EmbeddingPurpose,texts:&[String],cancel:CancellationToken) -> Result<EmbeddingBatch,EmbeddingError> {
        let inputs=self.spec.prepare(purpose,texts,&self.limits)?;
        tokio::select! { biased;
            _=cancel.cancelled()=>Err(EmbeddingError::Cancelled),
            result=tokio::time::timeout(self.limits.timeout,self.send(inputs))=>result.map_err(|_|EmbeddingError::Timeout)?,
        }
    }
}
pub(crate) fn transport_error(e:reqwest::Error)->EmbeddingError {
    if e.is_timeout() {EmbeddingError::Timeout} else {EmbeddingError::Transport}
}
pub(crate) async fn bounded_response(mut response:reqwest::Response,max:usize)->Result<Vec<u8>,EmbeddingError> {
    if response.status().is_redirection() {return Err(EmbeddingError::Redirect);}
    if !response.status().is_success() {return Err(EmbeddingError::Service);}
    if response.content_length().is_some_and(|n|n>max as u64) {return Err(EmbeddingError::ResponseTooLarge);}
    let mut bytes=Vec::new();
    while let Some(chunk)=response.chunk().await.map_err(transport_error)? {
        if chunk.len()>max.saturating_sub(bytes.len()) {return Err(EmbeddingError::ResponseTooLarge);}
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[derive(Deserialize)]
struct Response {model:String,data:Vec<Row>}
#[derive(Deserialize)]
struct Row {index:usize,embedding:Vec<f32>}
fn decode_response(spec:&EmbeddingSpec,bytes:&[u8],count:usize)->Result<EmbeddingBatch,EmbeddingError> {
    let response:Response=serde_json::from_slice(bytes).map_err(|_|EmbeddingError::InvalidResponse)?;
    if response.model != spec.model() {return Err(EmbeddingError::ModelMismatch);}
    if count==0 || count>64 || response.data.len()!=count {return Err(EmbeddingError::InvalidResponse);}
    let mut ordered=vec![None;count];
    for row in response.data {
        if row.index>=count || ordered[row.index].is_some() {return Err(EmbeddingError::InvalidResponse);}
        ordered[row.index]=Some(row.embedding);
    }
    let vectors=ordered.into_iter().map(|v|v.ok_or(EmbeddingError::InvalidResponse)).collect::<Result<Vec<_>,_>>()?;
    spec.validate_vectors(vectors,count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::TcpListener};
    fn profile(endpoint:&str)->EmbeddingProfile {
        serde_json::from_value(json!({"mode":"api","model":"fixture","revision":"pinned","dimensions":2,
            "normalization":"l2","distance":"cosine","document_prefix":"D: ","query_prefix":"Q: ",
            "endpoint":endpoint,"secret":"fixture-secret"})).unwrap()
    }
    // A real loopback HTTP transport. Captures body only; no external address/credentials.
    async fn server(status:&str,body:String,headers:&str,delay:std::time::Duration)->(String,tokio::task::JoinHandle<String>) {
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/v1/embeddings",listener.local_addr().unwrap());
        let status=status.to_owned();let headers=headers.to_owned();
        let task=tokio::spawn(async move {
            let (mut stream,_)=listener.accept().await.unwrap();let mut bytes=Vec::new();let mut buf=[0;4096];
            let end=loop {
                let n=stream.read(&mut buf).await.unwrap(); if n==0 {return String::new();} bytes.extend_from_slice(&buf[..n]);
                if let Some(p)=bytes.windows(4).position(|w|w==b"\r\n\r\n") {break p+4;}
            };
            let head=String::from_utf8_lossy(&bytes[..end]);
            let length=head.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|s|s.trim().parse::<usize>().ok())).unwrap();
            while bytes.len()<end+length {let n=stream.read(&mut buf).await.unwrap();if n==0 {break;}bytes.extend_from_slice(&buf[..n]);}
            let request=String::from_utf8(bytes).unwrap();
            tokio::time::sleep(delay).await;
            let response=format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{headers}connection: close\r\n\r\n{body}",body.len());
            let _=stream.write_all(response.as_bytes()).await;
            request
        });(url,task)
    }
    fn response()->String {json!({"model":"fixture","data":[{"index":1,"embedding":[0.,-2.]},{"index":0,"embedding":[3.,4.]}]}).to_string()}
    #[tokio::test]
    async fn actual_request_pins_model_dimensions_prefix_key_and_reorders_batch() {
        let (url,task)=server("200 OK",response(),"",std::time::Duration::ZERO).await;
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits::default()).unwrap();
        let batch=api.embed(EmbeddingPurpose::Document,&["one".into(),"two".into()],CancellationToken::new()).await.unwrap();
        assert_eq!(batch.vectors,vec![vec![0.6,0.8],vec![0.,-1.]]);
        assert_eq!(&batch.fingerprint,api.fingerprint());
        let request=task.await.unwrap();let (head,body)=request.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("POST /v1/embeddings HTTP/1.1"));
        assert!(head.to_lowercase().contains("authorization: bearer fixture-secret"));
        let body:serde_json::Value=serde_json::from_str(body).unwrap();
        assert_eq!(body,json!({"model":"fixture","input":["D: one","D: two"],"dimensions":2,"encoding_format":"float"}));
    }
    #[test]
    fn hostile_batch_response_rejects_duplicate_missing_out_of_range_and_model_mismatch() {
        let spec=EmbeddingSpec::from_profile(&profile("https://example.invalid/embeddings")).unwrap();
        for rows in [json!([]),json!([{"index":0,"embedding":[1.,2.]}]),
            json!([{"index":0,"embedding":[1.,2.]},{"index":0,"embedding":[1.,2.]}]),
            json!([{"index":0,"embedding":[1.,2.]},{"index":2,"embedding":[1.,2.]}]),
            json!([{"index":-1,"embedding":[1.,2.]},{"index":1,"embedding":[1.,2.]}]),
            json!([{"index":0,"embedding":[1.]},{"index":1,"embedding":[1.,2.]}])] {
            assert!(decode_response(&spec,&serde_json::to_vec(&json!({"model":"fixture","data":rows})).unwrap(),2).is_err());
        }
        assert_eq!(decode_response(&spec,b"{\"model\":\"wrong\",\"data\":[]}",2).unwrap_err(),EmbeddingError::ModelMismatch);
        assert!(decode_response(&spec,b"{\"model\":\"fixture\",\"data\":[{\"index\":0,\"embedding\":[1e100,1]}]}",1).is_err());
    }
    #[tokio::test]
    async fn redirect_is_not_followed_and_service_body_is_never_exposed() {
        let target=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let headers=format!("location: http://{}/stolen\r\n",target.local_addr().unwrap());
        let (url,task)=server("307 Temporary Redirect","fixture-secret".into(),&headers,std::time::Duration::ZERO).await;
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits::default()).unwrap();
        assert_eq!(api.embed(EmbeddingPurpose::Query,&["one".into()],CancellationToken::new()).await.unwrap_err(),EmbeddingError::Redirect);
        task.await.unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50),target.accept()).await.is_err());
        let (url,task)=server("401 Unauthorized","fixture-secret payload-text".into(),"",std::time::Duration::ZERO).await;
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits::default()).unwrap();
        let e=api.embed(EmbeddingPurpose::Query,&["one".into()],CancellationToken::new()).await.unwrap_err();
        assert_eq!(e,EmbeddingError::Service);assert!(!format!("{e:?} {e}").contains("fixture-secret")); task.await.unwrap();
    }
    #[tokio::test]
    async fn bounded_body_timeout_and_inflight_cancel_have_no_fallback() {
        let (url,task)=server("200 OK",response(),"",std::time::Duration::ZERO).await;
        let limits=EmbeddingLimits{max_response_bytes:10,..EmbeddingLimits::default()};
        let api=ApiEmbedding::new(&profile(&url),true,limits).unwrap();
        assert_eq!(api.embed(EmbeddingPurpose::Query,&["one".into()],CancellationToken::new()).await.unwrap_err(),EmbeddingError::ResponseTooLarge);task.await.unwrap();
        let (url,task)=server("200 OK",response(),"",std::time::Duration::from_millis(100)).await;
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits{timeout:std::time::Duration::from_millis(20),..EmbeddingLimits::default()}).unwrap();
        assert_eq!(api.embed(EmbeddingPurpose::Query,&["one".into()],CancellationToken::new()).await.unwrap_err(),EmbeddingError::Timeout);task.await.unwrap();
        let (url,task)=server("200 OK",response(),"",std::time::Duration::from_millis(100)).await;
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits::default()).unwrap();
        let cancel=CancellationToken::new();let c=cancel.clone();
        tokio::spawn(async move {tokio::time::sleep(std::time::Duration::from_millis(20)).await;c.cancel();});
        assert_eq!(api.embed(EmbeddingPurpose::Query,&["one".into()],cancel).await.unwrap_err(),EmbeddingError::Cancelled);task.await.unwrap();
    }
    #[tokio::test]
    async fn pre_cancelled_request_and_unapproved_http_never_connect() {
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}/embeddings",listener.local_addr().unwrap());
        assert!(ApiEmbedding::new(&profile(&url),false,EmbeddingLimits::default()).is_err());
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits::default()).unwrap();let cancel=CancellationToken::new();cancel.cancel();
        assert_eq!(api.embed(EmbeddingPurpose::Query,&["one".into()],cancel).await.unwrap_err(),EmbeddingError::Cancelled);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30),listener.accept()).await.is_err());
        let mut p=profile("https://example.invalid/embeddings");p.secret=Some("bad\nheader".into());
        assert!(ApiEmbedding::new(&p,false,EmbeddingLimits::default()).is_err());
    }
    #[tokio::test]
    async fn unknown_length_chunked_response_is_bounded_after_decoding() {
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/embeddings",listener.local_addr().unwrap());
        let task=tokio::spawn(async move {
            let (mut stream,_)=listener.accept().await.unwrap();let mut buffer=[0;4096];
            stream.read(&mut buffer).await.unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n10\r\n0123456789abcdef\r\n0\r\n\r\n").await.unwrap();
        });
        let api=ApiEmbedding::new(&profile(&url),true,EmbeddingLimits{max_response_bytes:10,..EmbeddingLimits::default()}).unwrap();
        assert_eq!(api.embed(EmbeddingPurpose::Query,&["one".into()],CancellationToken::new()).await.unwrap_err(),EmbeddingError::ResponseTooLarge);
        task.await.unwrap();
    }
}
