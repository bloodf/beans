use super::*;
use crate::memory_service::types::EmbeddingProfile;
use serde_json::{json, Value};

fn profile_value() -> Value {
    json!({"mode":"api","model":"tiny-v1","revision":"immutable-1","dimensions":2,
        "normalization":"l2","distance":"cosine","document_prefix":"doc: ",
        "query_prefix":"query: ","endpoint":"https://example.invalid/v1/embeddings"})
}
fn profile(v: Value) -> EmbeddingProfile { serde_json::from_value(v).unwrap() }
fn spec() -> EmbeddingSpec { EmbeddingSpec::from_profile(&profile(profile_value())).unwrap() }

#[test]
fn fingerprint_covers_every_semantic_field_but_not_credentials() {
    let base = spec();
    for (field, value) in [
        ("model", json!("tiny-v2")), ("revision", json!("immutable-2")),
        ("dimensions", json!(3)), ("normalization", json!("none")),
        ("distance", json!("dot")), ("document_prefix", json!("")),
        ("query_prefix", json!("search: ")),
        ("endpoint", json!("https://other.invalid/v1/embeddings")),
        ("future_preprocessing", json!({"a":1})),
    ] {
        let mut p = profile_value(); p[field] = value;
        assert_ne!(base.fingerprint(), EmbeddingSpec::from_profile(&profile(p)).unwrap().fingerprint(), "{field}");
    }
    let mut p = profile_value(); p["secret"] = json!("never-log-me");
    assert_eq!(base.fingerprint(), EmbeddingSpec::from_profile(&profile(p.clone())).unwrap().fingerprint());
    assert_eq!(base.fingerprint().as_str().len(), 64);
    assert!(base.fingerprint().as_str().bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert!(!format!("{:?}", profile(p).secret.map(|_| base)).contains("never-log-me"));
}

#[test]
fn invalid_profiles_fail_without_dimension_or_preprocessing_guessing() {
    for (field, value) in [
        ("model", json!("")), ("revision", json!("")), ("dimensions", json!(0)),
        ("dimensions", json!(65537)), ("normalization", json!("auto")),
        ("distance", json!("cos")), ("endpoint", json!(null)),
        ("endpoint", json!("https://u:secret@example.invalid/embeddings")),
        ("endpoint", json!("https://example.invalid/embeddings?key=secret")),
        ("endpoint", json!("https://example.invalid/embeddings#secret")),
        ("endpoint", json!("https://example.invalid/v1")),
        ("endpoint", json!("file:///embeddings")),
    ] {
        let mut p = profile_value(); p[field] = value;
        assert!(EmbeddingSpec::from_profile(&profile(p)).is_err(), "{field}");
    }
    let mut p = profile_value(); p["query_prefix"] = json!("x".repeat(4097));
    assert!(EmbeddingSpec::from_profile(&profile(p)).is_err());
}

#[test]
fn ordered_batch_validation_rejects_count_dimension_nonfinite_and_zero_norm() {
    let s = spec();
    for vectors in [vec![],vec![vec![1.,2.]],vec![vec![1.],vec![2.,3.]],
        vec![vec![f32::NAN,1.],vec![1.,1.]],
        vec![vec![f32::INFINITY,1.],vec![1.,1.]],vec![vec![0.,0.],vec![1.,1.]]] {
        assert!(s.validate_vectors(vectors,2).is_err());
    }
    let batch = s.validate_vectors(vec![vec![3.,4.],vec![0.,-2.]],2).unwrap();
    assert_eq!(batch.vectors, vec![vec![0.6,0.8],vec![0.,-1.]]);
    assert_eq!(batch.fingerprint, *s.fingerprint());
    // f64 norm avoids f32 overflow and underflow for valid finite inputs.
    for x in [f32::MAX, f32::MIN_POSITIVE, f32::from_bits(1)] {
        assert_eq!(s.validate_vectors(vec![vec![x,0.]],1).unwrap().vectors[0],vec![1.,0.]);
    }
    let mut p = profile_value(); p["normalization"] = json!("none");
    assert_eq!(EmbeddingSpec::from_profile(&profile(p)).unwrap().validate_vectors(vec![vec![3.,4.]],1).unwrap().vectors[0],vec![3.,4.]);
}

#[test]
fn preprocessing_preserves_exact_text_and_applies_only_selected_prefix() {
    let s=spec(); let limits=EmbeddingLimits::default();
    let texts=vec!["  CAFÉ\n".into(),"two".into()];
    assert_eq!(s.prepare(EmbeddingPurpose::Query,&texts,&limits).unwrap(),vec!["query:   CAFÉ\n","query: two"]);
    assert_eq!(s.prepare(EmbeddingPurpose::Document,&texts,&limits).unwrap()[0],"doc:   CAFÉ\n");
    for texts in [vec![],vec!["".into()],vec!["x".repeat(32769)],vec!["x".into();65]] {
        assert!(s.prepare(EmbeddingPurpose::Query,&texts,&limits).is_err());
    }
    let limits=EmbeddingLimits { max_input_bytes: 8, ..limits };
    assert!(s.prepare(EmbeddingPurpose::Query,&vec!["abc".into()],&limits).is_err());
}

#[test]
fn generation_fences_space_and_requires_approved_monotonic_replacement() {
    let s=spec(); let active=IndexGeneration::new(1,s.fingerprint().clone(),2).unwrap();
    let batch=s.validate_vectors(vec![vec![3.,4.]],1).unwrap();
    active.validate_batch(&batch).unwrap();
    let mut p=profile_value(); p["revision"]=json!("new");
    let other=EmbeddingSpec::from_profile(&profile(p)).unwrap();
    assert!(active.validate_batch(&other.validate_vectors(vec![vec![3.,4.]],1).unwrap()).is_err());
    assert!(active.replacement(2,other.fingerprint().clone(),2,false).is_err());
    assert!(active.replacement(1,other.fingerprint().clone(),2,true).is_err());
    let next=active.replacement(2,other.fingerprint().clone(),2,true).unwrap();
    assert!(next.validate_batch(&batch).is_err());
    assert_eq!(active.generation,1); // old generation remains available for storage rollback.
    assert!(EmbeddingFingerprint::parse("abcdef").is_err());
    assert!(EmbeddingFingerprint::parse(&"A".repeat(64)).is_err());
    assert_eq!(EmbeddingFingerprint::parse(s.fingerprint().as_str()).unwrap(),*s.fingerprint());
    assert!(IndexGeneration::new(0,s.fingerprint().clone(),2).is_err());
}

#[test]
fn local_configuration_is_portable_and_exact_options_change_space() {
    let mut p=profile_value(); p["mode"]=json!("local_cpu"); p["endpoint"]=Value::Null;
    p["local"]=json!({"model_sha256":"a".repeat(64),"tokenizer_sha256":"b".repeat(64),
        "max_tokens":32,"pooling":"mean","tensors":{"input_ids":"input_ids","attention_mask":"attention_mask","token_type_ids":null,"output":"last_hidden_state"},
        "add_special_tokens":true,"pad_id":0,"pad_type_id":0,"pad_token":"[PAD]"});
    let base=EmbeddingSpec::from_profile(&profile(p.clone())).unwrap();
    for (field,value) in [("max_tokens",json!(64)),("pooling",json!("cls")),("pad_id",json!(1)),("add_special_tokens",json!(false)),("model_sha256",json!("c".repeat(64)))] {
        let mut q=p.clone(); q["local"][field]=value;
        assert_ne!(base.fingerprint(),EmbeddingSpec::from_profile(&profile(q)).unwrap().fingerprint());
    }
    p["local"]["max_tokens"]=json!(0); assert!(EmbeddingSpec::from_profile(&profile(p)).is_err());
}

#[test]
fn pooling_respects_mask_and_exact_shapes() {
    let hidden=[1.,2.,3.,4.,99.,99.]; let mask=[1i64,1,0];
    assert_eq!(pool_output(&hidden,&[1,3,2],&mask,1,3,2,Pooling::Mean).unwrap(),vec![vec![2.,3.]]);
    assert_eq!(pool_output(&hidden,&[1,3,2],&mask,1,3,2,Pooling::Cls).unwrap(),vec![vec![1.,2.]]);
    assert_eq!(pool_output(&[5.,6.],&[1,2],&mask,1,3,2,Pooling::Pooled).unwrap(),vec![vec![5.,6.]]);
    assert!(pool_output(&hidden,&[1,3,2],&[0,0,0],1,3,2,Pooling::Mean).is_err());
    assert!(pool_output(&hidden,&[1,3,2],&mask,1,3,3,Pooling::Mean).is_err());
    assert!(pool_output(&hidden,&[1,3,2],&[1],1,3,2,Pooling::Mean).is_err());
    assert!(pool_output(&[f32::NAN,1.],&[1,2],&mask,1,3,2,Pooling::Pooled).is_err());
}
