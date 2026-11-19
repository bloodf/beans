//! State a Runner keeps for local models: settings per account in a private JSON file, and
//! models loaded in the background. Shared by `schedule_models` and `search_index`.

use std::future::Future;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::app::App;

/// The account this Device belongs to, by its identity key.
pub(crate) fn account(app: &App) -> Result<String, String> {
    app.machine_file().map(|m| m.identity_pubkey).ok_or_else(|| "identity_required".into())
}

/// Settings a Runner keeps per account, in a private JSON file at `<home>/<PATH>`.
pub(crate) trait Settings: Clone + Default + Serialize + DeserializeOwned {
    const PATH: &'static str;
}

/// `T`'s file: read once and cached, changed as one read-change-write step. A file written
/// for another account reads as the default.
pub(crate) struct AccountFile<T> {
    cache: Mutex<Option<Stored<T>>>,
    edit: Mutex<()>,
}

impl<T> Default for AccountFile<T> {
    fn default() -> Self {
        AccountFile { cache: Mutex::new(None), edit: Mutex::new(()) }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Stored<T> {
    account: String,
    #[serde(flatten)]
    value: T,
}

impl<T: Settings> AccountFile<T> {
    /// The current account's settings.
    pub(crate) fn get(&self, app: &App) -> Result<T, String> {
        let account = account(app)?;
        let mut cache = self.cache.lock();
        if let Some(stored) = cache.as_ref().filter(|s| s.account == account) {
            return Ok(stored.value.clone());
        }
        let stored = std::fs::read(app.config.home.join(T::PATH))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored<T>>(&bytes).ok())
            .filter(|s| s.account == account)
            .unwrap_or(Stored { account, value: T::default() });
        let value = stored.value.clone();
        *cache = Some(stored);
        Ok(value)
    }

    /// Changes the settings while `still` holds, and writes them before anyone reads them.
    pub(crate) fn edit<R>(&self, app: &App, still: impl Fn() -> Result<(), String>, change: impl FnOnce(&mut T) -> Result<R, String>) -> Result<R, String> {
        let _edit = self.edit.lock();
        still()?;
        let mut value = self.get(app)?;
        let result = change(&mut value)?;
        let stored = Stored { account: account(app)?, value };
        crate::config::write_json_private(&app.config.home.join(T::PATH), &stored).map_err(|_| "storage_failed".to_string())?;
        *self.cache.lock() = Some(stored);
        Ok(result)
    }

    /// Forgets the cached copy; the file is read again when next needed.
    pub(crate) fn forget(&self) {
        *self.cache.lock() = None;
    }
}

/// A model loaded in the background for one key (the account and what is selected), one load
/// at a time. A load that failed is not retried until `reset`, so a broken install does not
/// spin; a load finishing after a `reset` or for a key no longer wanted is dropped.
pub(crate) struct Loader<T> {
    state: Mutex<Load<T>>,
    cancel: Mutex<CancellationToken>,
}

enum Load<T> {
    Idle,
    Loading(String),
    Ready(String, Arc<T>),
    Failed(String, String),
}

impl<T> Default for Loader<T> {
    fn default() -> Self {
        Loader { state: Mutex::new(Load::Idle), cancel: Mutex::new(CancellationToken::new()) }
    }
}

impl<T: Send + Sync + 'static> Loader<T> {
    /// The loaded model for `key`, if it is ready.
    pub(crate) fn ready(&self, key: &str) -> Option<Arc<T>> {
        match &*self.state.lock() {
            Load::Ready(k, value) if k == key => Some(value.clone()),
            _ => None,
        }
    }

    /// `ready`, `loading`, `failed`, or `not_loaded`, for `key`.
    pub(crate) fn status(&self, key: &str) -> &'static str {
        match &*self.state.lock() {
            Load::Ready(k, _) if k == key => "ready",
            Load::Loading(k) if k == key => "loading",
            Load::Failed(k, _) if k == key => "failed",
            _ => "not_loaded",
        }
    }

    /// Why the load for `key` failed.
    pub(crate) fn error(&self, key: &str) -> Option<String> {
        match &*self.state.lock() {
            Load::Failed(k, error) if k == key => Some(error.clone()),
            _ => None,
        }
    }

    /// Forgets any model, error, or load under way, which is cancelled.
    pub(crate) fn reset(&self) {
        let mut cancel = self.cancel.lock();
        cancel.cancel();
        *cancel = CancellationToken::new();
        *self.state.lock() = Load::Idle;
    }

    /// Claims the load of `key` unless it is ready, loading, or failed. Returns its token.
    fn claim(&self, key: &str) -> Option<CancellationToken> {
        let mut state = self.state.lock();
        match &*state {
            Load::Ready(k, _) | Load::Loading(k) | Load::Failed(k, _) if k == key => None,
            _ => {
                *state = Load::Loading(key.to_string());
                Some(self.cancel.lock().child_token())
            }
        }
    }

    fn finish(&self, key: &str, cancel: &CancellationToken, result: Result<T, String>) -> Result<Arc<T>, String> {
        let mut state = self.state.lock();
        let wanted = matches!(&*state, Load::Loading(k) if k == key) && !cancel.is_cancelled();
        if !wanted {
            return Err("cancelled".into());
        }
        match result {
            Ok(value) => {
                let value = Arc::new(value);
                *state = Load::Ready(key.to_string(), value.clone());
                Ok(value)
            }
            Err(error) => {
                *state = Load::Failed(key.to_string(), error.clone());
                Err(error)
            }
        }
    }

    /// Loads `key` now and waits, unless it is ready (then returns it) or already loading.
    pub(crate) async fn load_now<F: Future<Output = Result<T, String>>>(&self, key: &str, load: impl FnOnce(CancellationToken) -> F) -> Result<Arc<T>, String> {
        if let Some(ready) = self.ready(key) {
            return Ok(ready);
        }
        let cancel = {
            let mut state = self.state.lock();
            if matches!(&*state, Load::Loading(k) if k == key) {
                return Err("loading".into());
            }
            *state = Load::Loading(key.to_string());
            self.cancel.lock().child_token()
        };
        let result = load(cancel.clone()).await;
        self.finish(key, &cancel, result)
    }

    /// Starts loading `key` in the background unless it is ready, loading, or failed.
    pub(crate) fn start<F>(self: &Arc<Self>, key: &str, load: impl FnOnce(CancellationToken) -> F)
    where
        F: Future<Output = Result<T, String>> + Send + 'static,
    {
        let Some(cancel) = self.claim(key) else { return };
        let loader = self.clone();
        let key = key.to_string();
        let load = load(cancel.clone());
        tokio::spawn(async move {
            let result = load.await;
            if let Err(error) = loader.finish(&key, &cancel, result) {
                if error != "cancelled" {
                    tracing::warn!(%error, key, "loading a local model");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Choice {
        model: Option<String>,
    }
    impl Settings for Choice {
        const PATH: &'static str = "fixture/settings.json";
    }

    fn app() -> (Arc<App>, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        let app = App::load(crate::config::Config { home: home.path().to_path_buf(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Runner".into())).unwrap();
        (app, home)
    }

    #[tokio::test]
    async fn settings_belong_to_their_account_and_survive_bad_files() {
        let (app, _home) = app();
        let file = AccountFile::<Choice>::default();
        let path = app.config.home.join(Choice::PATH);
        assert_eq!(file.get(&app).unwrap(), Choice::default(), "nothing saved yet");
        file.edit(&app, || Ok(()), |choice| {
            choice.model = Some("m".into());
            Ok(())
        }).unwrap();
        assert_eq!(file.get(&app).unwrap().model.as_deref(), Some("m"));
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["account"], account(&app).unwrap().as_str(), "the file names its account");
        // A refused edit writes nothing.
        let refused = file.edit(&app, || Err("account_paused".into()), |choice| {
            choice.model = Some("other".into());
            Ok(())
        });
        assert_eq!(refused.unwrap_err(), "account_paused");
        let failed = file.edit(&app, || Ok(()), |_| Err::<(), _>("invalid".to_string()));
        assert_eq!(failed.unwrap_err(), "invalid");
        file.forget();
        assert_eq!(file.get(&app).unwrap().model.as_deref(), Some("m"));
        // Another account's file, or a broken one, reads as nothing saved.
        std::fs::write(&path, br#"{"account":"someone else","model":"theirs"}"#).unwrap();
        file.forget();
        assert_eq!(file.get(&app).unwrap(), Choice::default());
        for broken in [&b"not json"[..], b"", b"[]", br#"{"model":"no account"}"#] {
            std::fs::write(&path, broken).unwrap();
            file.forget();
            assert_eq!(file.get(&app).unwrap(), Choice::default(), "{}", String::from_utf8_lossy(broken));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.edit(&app, || Ok(()), |choice| {
                choice.model = Some("private".into());
                Ok(())
            }).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn overlapping_loads_keep_only_the_one_still_wanted() {
        let loader = Arc::new(Loader::<u32>::default());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        loader.start("a", |_| async move {
            let _ = wait.await;
            Ok(1)
        });
        assert_eq!(loader.load_now("a", |_| async { Ok(2) }).await.unwrap_err(), "loading", "one load of a key at a time");
        // Another key is wanted now: it takes over, and the first load's answer is dropped.
        assert_eq!(*loader.load_now("b", |_| async { Ok(3) }).await.unwrap(), 3);
        let _ = release.send(());
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!((loader.status("a"), loader.status("b")), ("not_loaded", "ready"));
        assert!(loader.ready("a").is_none() && loader.error("b").is_none());
        // A cancelled load reports cancelled and leaves nothing behind.
        loader.reset();
        let handle = loader.clone();
        let cancelled = tokio::spawn(async move { handle.load_now("c", |cancel| async move { cancel.cancelled().await; Ok(4) }).await });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        loader.reset();
        assert_eq!(cancelled.await.unwrap().unwrap_err(), "cancelled");
        assert_eq!(loader.status("c"), "not_loaded");
    }

    #[tokio::test]
    async fn a_load_is_claimed_once_kept_for_its_key_and_dropped_after_reset() {
        let loader = Arc::new(Loader::<u32>::default());
        assert_eq!(loader.status("a"), "not_loaded");
        assert_eq!(*loader.load_now("a", |_| async { Ok(1) }).await.unwrap(), 1);
        assert_eq!((loader.status("a"), loader.status("b")), ("ready", "not_loaded"));
        assert_eq!(*loader.load_now("a", |_| async { Ok(2) }).await.unwrap(), 1, "a ready key is not loaded again");

        // A failure sticks until reset, so a broken install does not spin.
        loader.reset();
        assert!(loader.load_now("b", |_| async { Err("broken".to_string()) }).await.is_err());
        assert_eq!((loader.status("b"), loader.error("b").as_deref()), ("failed", Some("broken")));
        loader.start("b", |_| async { Ok(3) });
        assert_eq!(loader.status("b"), "failed");

        // A load that finishes after a reset is dropped.
        loader.reset();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        loader.start("c", |_| async move {
            let _ = wait.await;
            Ok(4)
        });
        assert_eq!(loader.status("c"), "loading");
        loader.reset();
        let _ = release.send(());
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!(loader.status("c"), "not_loaded");
    }
}
