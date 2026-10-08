//! Beans's Device core, and, with the `runner` feature, the Runner: keys, the relay sync,
//! jobs and rooms, questions to other Runners, and the JSON API the apps speak. The `beans`
//! binary adds the local websocket server and the command line; the phone links the core
//! alone through `beans-mobile`.

pub mod api;
pub mod app;
pub mod appearance;
pub mod catalog;
pub mod config;
pub mod credentials;
pub mod crypto;
pub mod diagnostics;
pub mod embeddings;
pub mod events;
pub mod files;
pub mod identity;
pub mod keys;
#[cfg(feature = "runner")]
pub mod local_review;
pub mod marketplace;
pub mod memory;
pub mod memory_service;
pub mod model;
pub mod pairing;
pub mod plugins;
#[cfg(feature = "provider-auth")]
pub mod provider_auth;
#[cfg(feature = "runner")]
pub mod providers;
pub mod push;
pub mod relay;
pub mod requests;
pub mod routines;
pub mod runtime;
pub mod schedule;
#[cfg(feature = "runner")]
pub mod scripts;
pub mod served;
#[cfg(feature = "cli")]
pub mod service;
#[cfg(feature = "runner")]
pub mod shell;
pub mod sync;
pub mod local_store;
#[cfg(feature = "runner")]
pub mod turns;
pub mod update_control;
#[cfg(feature = "cli")]
pub mod update;
#[cfg(feature = "server")]
pub mod ws;
