//! Standalone self-update is unavailable. Beans uses its signed release mechanism;
//! Runner admission drain is independent in `update_control`.

use std::sync::Arc;
use serde_json::Value;
use crate::app::App;
use crate::model::UpdateStatus;

pub const SELF_UPDATING: bool = false;
pub const UNAVAILABLE: &str = crate::config::SELF_UPDATE_UNAVAILABLE;
/// The service supervisor's immediate-restart exit contract on Windows.
pub const RESTART_EXIT: i32 = 75;

#[derive(Default)]
pub struct Updater;

impl Updater {
    pub fn status(&self) -> Option<UpdateStatus> { None }
}

/// No scheduler, pending-record recovery, download or binary mutation is started.
pub fn start(_app: &Arc<App>) {}
pub fn relay_refused(_app: &App) {}

pub async fn check(_app: &Arc<App>, _install: bool, _explicit: bool) -> Result<Value, String> {
    Err(UNAVAILABLE.into())
}

pub async fn install_now(_app: &Arc<App>) -> Result<Value, String> {
    Err(UNAVAILABLE.into())
}

pub fn set_auto(_app: &Arc<App>, _on: bool) -> Result<Value, String> {
    Err(UNAVAILABLE.into())
}

pub async fn latest_version(_app: &App) -> Result<String, String> {
    Err(UNAVAILABLE.into())
}

pub async fn install_here(_app: &Arc<App>) -> Result<Option<String>, String> {
    Err(UNAVAILABLE.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn beans_self_update_fails_closed_before_discovery_or_mutation() {
        let home = tempfile::tempdir().unwrap();
        let app = App::load(crate::config::Config { home: home.path().into(), port: 4874 }).unwrap();
        let pending = br#"{"pending":{"version":"9.0.0","previous":"0.1.10"}}"#;
        let retained = home.path().join("beans.previous");
        std::fs::write(app.config.update_path(), pending).unwrap();
        std::fs::write(&retained, b"retained-binary").unwrap();
        assert_eq!(check(&app, false, false).await.unwrap_err(), UNAVAILABLE);
        assert_eq!(check(&app, true, true).await.unwrap_err(), UNAVAILABLE);
        assert_eq!(latest_version(&app).await.unwrap_err(), UNAVAILABLE);
        assert_eq!(install_here(&app).await.unwrap_err(), UNAVAILABLE);
        assert_eq!(install_now(&app).await.unwrap_err(), UNAVAILABLE);
        assert_eq!(set_auto(&app, true).unwrap_err(), UNAVAILABLE);
        for method in ["device.update", "device.auto_update"] {
            assert_eq!(crate::api::dispatch(&app, method, serde_json::json!({"device_id":"remote","on":true})).await.unwrap_err(), UNAVAILABLE);
        }
        start(&app);
        relay_refused(&app);
        assert!(app.updates.status().is_none());
        assert_eq!(std::fs::read(app.config.update_path()).unwrap(), pending);
        assert_eq!(std::fs::read(retained).unwrap(), b"retained-binary");
        assert!(app.settings.lock().unwrap().auto_update.is_none());
    }
}
