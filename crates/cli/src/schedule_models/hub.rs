//! The model hub: where catalog files download from. Hugging Face by default, reached as
//! `<base>/<repo>/resolve/<revision>/<path>`, whose answers may redirect to its CDN hosts.
//! This is the one place that knows the hub's address and where its downloads may be sent on;
//! listings and downloads both ask it.
//!
//! `BEANS_MODEL_HUB` names another hub with the same layout: a mirror (https), or a mock on
//! this machine (http, loopback only) for tests and development. Whatever the hub, nothing
//! downloads without a preview the user approved, and only bytes matching the catalog's pinned
//! size and SHA-256 are installed. Authentication for gated repositories would be added here.

use crate::embeddings::assets::RedirectHosts;

use super::{CatalogFile, CatalogModel};

pub const HUGGING_FACE: &str = "https://huggingface.co";

#[derive(Clone, Debug, PartialEq)]
pub struct Hub {
    /// The origin (and any path prefix) files resolve under, without a trailing slash.
    pub base: String,
    /// Plain http, for a hub on this machine only.
    pub allow_insecure_http: bool,
    /// Where a file's answer may redirect, besides nowhere.
    pub redirects: RedirectHosts,
}

impl Default for Hub {
    fn default() -> Self {
        Hub::hugging_face()
    }
}

impl Hub {
    /// Hugging Face, whose file downloads go on to its CDN (cdn-lfs.hf.co, cas-bridge.xethub.hf.co).
    pub fn hugging_face() -> Hub {
        let hosts = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
        Hub {
            base: HUGGING_FACE.into(),
            allow_insecure_http: false,
            redirects: RedirectHosts { exact: hosts(&["huggingface.co", "hf.co"]), parents: hosts(&["huggingface.co", "hf.co"]) },
        }
    }

    /// The hub this Runner uses: `BEANS_MODEL_HUB` when it names a valid hub, else Hugging Face.
    pub fn from_env() -> Hub {
        let configured = std::env::var("BEANS_MODEL_HUB").ok();
        Hub::parse(configured.as_deref()).unwrap_or_else(|error| {
            tracing::warn!(%error, "BEANS_MODEL_HUB is not a hub; using Hugging Face");
            Hub::hugging_face()
        })
    }

    /// `value` as a hub: none or blank is Hugging Face; https anywhere; http only on this
    /// machine. No credentials, query, or fragment. Its downloads may redirect only to its own
    /// host.
    pub fn parse(value: Option<&str>) -> Result<Hub, String> {
        let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else { return Ok(Hub::hugging_face()) };
        let url = reqwest::Url::parse(value).map_err(|_| "not a URL".to_string())?;
        let host = url.host_str().ok_or("no host")?.to_ascii_lowercase();
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err("credentials, a query, or a fragment".into());
        }
        let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
        let allow_insecure_http = match url.scheme() {
            "https" => false,
            "http" if loopback => true,
            "http" => return Err("plain http away from this machine".into()),
            _ => return Err("neither https nor http".into()),
        };
        if value == HUGGING_FACE || value == format!("{HUGGING_FACE}/") {
            return Ok(Hub::hugging_face());
        }
        Ok(Hub {
            base: url.as_str().trim_end_matches('/').to_string(),
            allow_insecure_http,
            redirects: RedirectHosts { exact: [host].into(), parents: Default::default() },
        })
    }

    /// Where `file` of `model` downloads from.
    pub fn file_url(&self, model: &CatalogModel, file: &CatalogFile) -> String {
        format!("{}/{}/resolve/{}/{}", self.base, model.repo, model.revision, file.path)
    }

    /// How a preview names the hub to the user: "Hugging Face", or another hub's host.
    pub fn label(&self) -> String {
        if self.base == HUGGING_FACE {
            return "Hugging Face".into();
        }
        reqwest::Url::parse(&self.base).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_else(|| self.base.clone())
    }

    /// How many bytes `url` serves, asking for its first byte only and following this hub's
    /// redirects as installs do: what the live checks compare with the catalog's pins without
    /// downloading every model.
    #[cfg(test)]
    pub(crate) async fn served_size(&self, url: &str) -> Result<u64, String> {
        let (redirects, insecure) = (self.redirects.clone(), self.allow_insecure_http);
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 3 || !redirects.allows(attempt.url(), insecure) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let client = beans_tls::client_builder().redirect(policy).timeout(std::time::Duration::from_secs(60)).build().map_err(|e| e.to_string())?;
        let response = client.get(url).header(reqwest::header::RANGE, "bytes=0-0").send().await.map_err(|e| e.to_string())?;
        match response.status().as_u16() {
            206 => response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|range| range.to_str().ok()?.rsplit('/').next()?.parse().ok())
                .ok_or_else(|| "a partial answer without its whole size".to_string()),
            200 => response.content_length().ok_or_else(|| "no size".to_string()),
            status => Err(format!("HTTP {status}")),
        }
    }

    /// The origin downloads must start at.
    pub fn origin(&self) -> Result<String, String> {
        Ok(reqwest::Url::parse(&self.base).map_err(|_| "invalid_model_hub")?.origin().ascii_serialization())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_hub_is_hugging_face_and_its_cdn() {
        for value in [None, Some(""), Some("  "), Some(HUGGING_FACE), Some("https://huggingface.co/")] {
            assert_eq!(Hub::parse(value).unwrap(), Hub::hugging_face(), "{value:?}");
        }
        let hub = Hub::hugging_face();
        let allows = |url: &str| hub.redirects.exact.iter().chain(&hub.redirects.parents).any(|h| reqwest::Url::parse(url).unwrap().host_str().unwrap().ends_with(h.as_str()));
        assert!(allows("https://cdn-lfs.hf.co/x") && allows("https://cas-bridge.xethub.hf.co/x"));
        assert_eq!(hub.origin().unwrap(), "https://huggingface.co");
        assert_eq!(hub.label(), "Hugging Face");
        assert_eq!(Hub::parse(Some("https://models.example.com/hf")).unwrap().label(), "models.example.com", "a mirror is named by its host");
    }

    #[test]
    fn another_hub_is_https_or_this_machine_and_redirects_only_to_itself() {
        let mirror = Hub::parse(Some("https://models.example.com/hf/")).unwrap();
        assert_eq!((mirror.base.as_str(), mirror.allow_insecure_http), ("https://models.example.com/hf", false));
        assert_eq!(mirror.redirects.exact, ["models.example.com".to_string()].into());
        assert!(mirror.redirects.parents.is_empty());
        assert_eq!(mirror.origin().unwrap(), "https://models.example.com");
        for local in ["http://127.0.0.1:8080", "http://localhost:9", "http://[::1]:8080"] {
            assert!(Hub::parse(Some(local)).unwrap().allow_insecure_http, "{local}");
        }
        for refused in [
            "http://models.example.com",
            "ftp://localhost",
            "https://user:secret@models.example.com",
            "https://models.example.com/?token=1",
            "https://models.example.com/#x",
            "not a url",
            "file:///models",
        ] {
            assert!(Hub::parse(Some(refused)).is_err(), "{refused}");
        }
    }

    #[test]
    fn files_resolve_under_the_hub() {
        let model = super::super::catalog().model("bge-small-en-v1.5-int8").unwrap();
        let (_, file) = model.files()[0];
        let mock = Hub::parse(Some("http://127.0.0.1:4000")).unwrap();
        assert_eq!(mock.file_url(model, file), format!("http://127.0.0.1:4000/{}/resolve/{}/{}", model.repo, model.revision, file.path));
        assert!(Hub::hugging_face().file_url(model, file).starts_with("https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/"));
    }
}
