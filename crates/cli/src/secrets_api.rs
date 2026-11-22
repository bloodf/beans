//! Managing a Runner's secret store from the apps (#560): `secrets.list`, `secrets.set`,
//! `secrets.delete` and `secrets.revoke`, for the account's own Devices, on the Runner a call
//! names. A response never holds a secret's value: the list gives names, lengths, where each came
//! from and the grants, and is passed through the shield before it leaves.

use std::sync::Arc;

use serde_json::Value;

use crate::app::App;

/// The API's verbs. All of them run on the Runner the call names.
pub fn is_method(method: &str) -> bool {
    matches!(method, "secrets.list" | "secrets.set" | "secrets.delete" | "secrets.revoke")
}

/// Routes a `secrets.*` call from the user's apps to the Runner it names.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if !is_method(method) {
        return Err("unknown_secrets_method".into());
    }
    let runner_id = params["runner_id"].as_str().ok_or("runner_id_required")?.to_owned();
    let runner = app.device(&runner_id).or_else(|| app.local_device().filter(|d| d.id == runner_id)).ok_or("runner_unknown")?;
    if !runner.is_runner() {
        return Err("runner_required".into());
    }
    let mut body = params;
    body.as_object_mut().ok_or("invalid_request")?.remove("runner_id");
    if app.this_device_id().as_deref() != Some(&runner_id) {
        return crate::requests::ask(app, &runner_id, method, body).await;
    }
    serve_as(app, method, body, &app.this_device_id().ok_or("identity_required")?).await
}

/// Serves a call on this Runner for `actor`, this Device or a paired one.
pub async fn serve_as(app: &Arc<App>, method: &str, params: Value, actor: &str) -> Result<Value, String> {
    #[cfg(feature = "runner")]
    return runner::serve(app, method, params, actor).await;
    #[cfg(not(feature = "runner"))]
    {
        let _ = (app, method, params, actor);
        Err("runner_required".into())
    }
}

#[cfg(feature = "runner")]
mod runner {
    use std::sync::Arc;

    use serde::Deserialize;
    use serde_json::{json, Value};

    use crate::app::App;
    use crate::schedule_models::runner::Authority;
    use crate::secret_store::StoredSecret;

    /// Longest value the apps may type in: a key, a token, a password, a short PEM block.
    const MAX_VALUE_BYTES: usize = 64 * 1024;

    fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|_| "invalid_request".to_string())
    }

    pub async fn serve(app: &Arc<App>, method: &str, params: Value, actor: &str) -> Result<Value, String> {
        let authority = Authority::capture(app, actor)?;
        if !app.local_device().is_some_and(|d| d.is_runner()) {
            return Err("runner_required".into());
        }
        let answer = match method {
            "secrets.list" => list(app)?,
            "secrets.set" => set(app, actor, params)?,
            "secrets.delete" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Request {
                    name: String,
                }
                let request: Request = decode(params)?;
                authority.check(app)?;
                json!({ "deleted": app.secret_store.delete(app, &request.name)? })
            }
            "secrets.revoke" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Request {
                    #[serde(default)]
                    name: Option<String>,
                }
                let request: Request = decode(params)?;
                authority.check(app)?;
                json!({ "revoked": app.secret_store.revoke(app, request.name.as_deref())? })
            }
            _ => return Err("unknown_secrets_method".into()),
        };
        // Whatever the store holds, no value leaves the Runner in an answer.
        let vault = crate::secret_shield::vault(app).await;
        Ok(vault.redact_json(&answer))
    }

    /// Names, lengths, sources and notes of the stored secrets, and the grants; never a value.
    fn list(app: &App) -> Result<Value, String> {
        let secrets: Vec<Value> = app
            .secret_store
            .secrets(app)?
            .into_iter()
            .map(|(name, secret)| {
                json!({
                    "name": name,
                    "length": secret.value.chars().count(),
                    "source": secret.source,
                    "note": secret.note,
                    "saved_by": secret.saved_by,
                    "updated_at": secret.updated_at,
                })
            })
            .collect();
        Ok(json!({ "secrets": secrets, "grants": app.secret_store.grants(app)? }))
    }

    /// Keeps a secret the user typed in an app, under `name`; the value is never shown again.
    fn set(app: &App, actor: &str, params: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            name: String,
            value: String,
            #[serde(default)]
            note: String,
        }
        let request: Request = decode(params)?;
        if request.value.trim().is_empty() {
            return Err("The secret is empty.".into());
        }
        if request.value.len() > MAX_VALUE_BYTES {
            return Err("The secret is longer than 64 KB.".into());
        }
        let device = app.device(actor).map(|device| device.name).unwrap_or_else(|| "this Device".into());
        let secret = StoredSecret {
            value: request.value,
            source: format!("typed in Beans on {device}"),
            note: request.note.trim().chars().take(200).collect(),
            saved_by: String::new(),
            updated_at: crate::secret_store::now(),
        };
        let replaced = app.secret_store.save(app, request.name.trim(), secret)?;
        Ok(json!({ "saved": true, "replaced": replaced.is_some() }))
    }
}

#[cfg(all(test, feature = "runner"))]
mod tests {
    use serde_json::json;

    use crate::app::App;

    const VALUE: &str = "sk-live-SECRETVALUE0123456789abcdef";

    fn runner() -> (Arc<App>, std::path::PathBuf) {
        let home = std::env::temp_dir().join(format!("beans-secrets-api-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Runner".into())).unwrap();
        (app, home)
    }

    use std::sync::Arc;

    async fn call(app: &Arc<App>, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value, String> {
        params["runner_id"] = app.this_device_id().unwrap().into();
        crate::api::dispatch(app, method, params).await
    }

    #[tokio::test]
    async fn the_apps_list_set_delete_and_revoke_without_ever_seeing_a_value() {
        let (app, home) = runner();
        let saved = call(&app, "secrets.set", json!({ "name": "STRIPE_KEY", "value": VALUE, "note": "billing" })).await.unwrap();
        assert_eq!(saved, json!({ "saved": true, "replaced": false }));
        assert_eq!(call(&app, "secrets.set", json!({ "name": "STRIPE_KEY", "value": VALUE })).await.unwrap()["replaced"], true);
        assert!(call(&app, "secrets.set", json!({ "name": "not a name", "value": "x" })).await.is_err());
        assert!(call(&app, "secrets.set", json!({ "name": "EMPTY", "value": "  " })).await.is_err());
        app.secret_store
            .grant(&app, crate::secret_store::Grant::Tool { tool: "create_charge".into(), secrets: vec!["STRIPE_KEY".into()], granted_at: 1.0 })
            .unwrap();

        let list = call(&app, "secrets.list", json!({})).await.unwrap();
        let text = list.to_string();
        assert!(!text.contains(VALUE) && !text.contains("SECRETVALUE"), "a value left the Runner: {text}");
        assert_eq!(list["secrets"][0]["name"], "STRIPE_KEY");
        assert_eq!(list["secrets"][0]["length"], VALUE.len());
        assert!(list["secrets"][0]["source"].as_str().unwrap().starts_with("typed in Beans on "));
        assert_eq!(list["grants"][0]["tool"], "create_charge");
        // The shield knows the value as soon as it is kept.
        assert_eq!(crate::secret_shield::vault(&app).await.redact(&format!("key {VALUE}")), "key {{secret:STRIPE_KEY}}");

        assert_eq!(call(&app, "secrets.revoke", json!({ "name": "STRIPE_KEY" })).await.unwrap(), json!({ "revoked": 1 }));
        assert_eq!(call(&app, "secrets.list", json!({})).await.unwrap()["grants"], json!([]));
        assert_eq!(call(&app, "secrets.delete", json!({ "name": "STRIPE_KEY" })).await.unwrap(), json!({ "deleted": true }));
        assert_eq!(call(&app, "secrets.delete", json!({ "name": "STRIPE_KEY" })).await.unwrap(), json!({ "deleted": false }));
        assert_eq!(call(&app, "secrets.list", json!({})).await.unwrap()["secrets"], json!([]));
        assert_eq!(crate::api::dispatch(&app, "secrets.list", json!({ "runner_id": "someone-else" })).await.unwrap_err(), "runner_unknown");
        assert!(crate::api::dispatch(&app, "secrets.list", json!({})).await.unwrap_err().contains("runner_id"));
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }
}
