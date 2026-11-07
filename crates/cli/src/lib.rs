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
pub mod decision;
pub mod diagnostics;
pub mod embeddings;
pub mod events;
pub mod files;
pub mod artifacts;
pub mod history164_types;
pub mod identity;
pub mod keys;
pub mod keystore;
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
pub mod retention;
pub mod routines;
pub mod runtime;
pub mod schedule;
pub mod schedule_models;
#[cfg(feature = "runner")]
pub mod schedule_intent;
mod search;
/// Secret values stay on the Runner: the model sees placeholders.
pub mod secret_shield;
/// The secrets the user keeps on this Runner, and the tool bots keep them with.
#[cfg(feature = "runner")]
pub mod secret_store;
pub mod search_index;
#[cfg(feature = "runner")]
pub(crate) mod runner_state;
#[cfg(feature = "runner")]
pub mod scripts;
pub mod served;
#[cfg(feature = "cli")]
pub mod service;
#[cfg(feature = "runner")]
pub mod shell;
pub mod skill_discovery;
pub mod skill_library;
pub mod sync;
pub mod local_store;
#[cfg(feature = "runner")]
pub mod machine;
#[cfg(feature = "runner")]
pub mod turns;
pub mod update_control;
pub mod workflow;
#[cfg(feature = "cli")]
pub mod serve;
pub(crate) mod serve_owner;
#[cfg(feature = "cli")]
pub mod update;
#[cfg(feature = "server")]
pub mod ws;
