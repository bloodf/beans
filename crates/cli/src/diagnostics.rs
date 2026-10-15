//! Privacy-safe setup evidence. No account snapshot or remote error text enters this report.
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::{app::App, credentials::PROVIDER_KINDS, relay::RelayError};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RelayReason {
    None,
    NotConfigured,
    InvalidUrl,
    Unreachable,
    UpdateRequired,
    HttpError,
}

fn reason(error: &RelayError) -> RelayReason {
    match error.status {
        Some(426) => RelayReason::UpdateRequired,
        Some(_) => RelayReason::HttpError,
        None => RelayReason::Unreachable,
    }
}

fn os(value: &str) -> &'static str {
    match value {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        "ios" => "ios",
        "ipados" => "ipados",
        "android" => "android",
        _ => "unknown",
    }
}

// Paired Device versions are untrusted text, not build constants. Only numeric releases
// are disclosed; arbitrary suffixes could contain private names or remote error text.
fn version(value: &str) -> Option<&str> {
    let mut parts = value.split('.');
    let valid = (0..3).all(|_| parts.next().is_some_and(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit())))
        && parts.next().is_none();
    valid.then_some(value)
}

fn plugin_state(value: &str) -> &'static str {
    match value {
        "ready" => "ready",
        "needs_setup" => "needs_setup",
        "needs_auth" => "needs_auth",
        "connecting" => "connecting",
        "error" => "error",
        _ => "unknown",
    }
}

/// The relay host and explicit port are deliberately disclosed so a failing selection
/// can be identified. No userinfo, path, query, fragment or raw error is returned.
/// This probes only public relay health, never providers or plugins, and uploads nothing.
pub async fn report(app: &Arc<App>) -> Value {
    let relay = match app.relay_url() {
        None => json!({"configured": false, "host": null, "reachable": null, "reason": RelayReason::NotConfigured, "http_status": null, "protocol": null}),
        Some(url) => match reqwest::Url::parse(&url).ok().filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some()) {
            None => json!({"configured": true, "host": null, "reachable": null, "reason": RelayReason::InvalidUrl, "http_status": null, "protocol": null}),
            Some(parsed) => {
                let host = match parsed.port() {
                    Some(port) => format!("{}:{port}", parsed.host().unwrap()),
                    None => parsed.host().unwrap().to_string(),
                };
                // Preserve the selected relay URL and health semantics; do not invent a
                // different endpoint by removing a configured reverse-proxy path.
                match app.relay.health(&url).await {
                    Ok(protocol) => json!({"configured": true, "host": host, "reachable": true, "reason": RelayReason::None, "http_status": null, "protocol": protocol}),
                    Err(error) => json!({"configured": true, "host": host, "reachable": error.status.is_some(), "reason": reason(&error), "http_status": error.status, "protocol": null}),
                }
            }
        },
    };
    let providers = {
        let credentials = app.credentials.lock().unwrap();
        let built_in: Vec<Value> = PROVIDER_KINDS.iter().map(|kind| {
            let configured = match *kind {
                "chatgpt" => credentials.chatgpt.is_some(),
                "grok" => credentials.grok.is_some(),
                _ => credentials.api_key(kind).is_some(),
            };
            json!({"kind": kind, "configured": configured})
        }).collect();
        json!({"built_in": built_in, "custom_configured": credentials.custom.len(), "health": "not_checked"})
    };
    let (plugins, mcp_json) = {
        let store = app.plugins.lock().unwrap();
        let plugins: Vec<Value> = store.statuses().iter().enumerate().map(|(index, status)|
            json!({"slot": index + 1, "state": plugin_state(&status.state)})
        ).collect();
        let mcp_json = json!({"servers": store.mcp.servers.len(), "problems": store.mcp.servers.iter().filter(|s| s.problem().is_some()).count(), "file_error": store.mcp.error.is_some()});
        (json!({"basis": "cached_setup_state", "entries": plugins}), mcp_json)
    };
    let this_id = app.this_device_id();
    let this_os = app.machine.lock().unwrap().as_ref().map(|m| os(&m.os)).unwrap_or_else(|| os(if std::env::consts::OS == "macos" { "macos" } else { std::env::consts::OS }));
    let runners: Vec<Value> = {
        let state = app.state.lock().unwrap();
        state.devices.iter().filter(|d| d.is_runner()).map(|d| {
            let is_this = this_id.as_deref() == Some(d.id.as_str());
            json!({"is_this_device": is_this, "os": os(&d.os), "presence": if is_this || state.device_online.contains(&d.id) { "online" } else { "offline" }, "version": version(&d.version)})
        }).collect()
    };
    json!({
        "schema_version": 1,
        "versions": {"core": crate::config::VERSION, "relay_protocol": crate::relay::PROTOCOL},
        "this_device": {"os": this_os, "is_runner": matches!(this_os, "macos" | "linux" | "windows"), "has_identity": app.has_identity()},
        "home": {"exists": app.config.home.is_dir()},
        "port": {"free": std::net::TcpListener::bind(("127.0.0.1", app.config.port)).is_ok()},
        "relay": relay, "providers": providers, "plugins": plugins, "mcp_json": mcp_json, "runners": runners,
        "computer_host": computer_host_report(&HostObservations::default()),
    })
}

/// Measured per-guest requirements and explicit Runner reserves; no default sizing.
#[derive(Clone, Copy)]
pub struct CapacityBudget {
    pub guest_ram_mib: u64,
    pub guest_disk_mib: u64,
    pub guest_vcpus: u64,
    pub runner_reserved_ram_mib: u64,
    pub runner_reserved_disk_mib: u64,
    pub runner_reserved_cpus: u64,
    pub ram_headroom_percent: u8,
}

/// Typed integration boundary for the separate host probe; no paths or raw output.
#[derive(Default)]
pub struct HostObservations {
    pub kvm_device_present: Option<bool>,
    pub kvm_device_accessible: Option<bool>,
    pub free_ram_mib: Option<u64>,
    pub free_disk_mib: Option<u64>,
    pub host_cpus: Option<u64>,
    pub budget: Option<CapacityBudget>,
}

fn computer_check(id: &str, observed: Option<bool>, evidence: [&str; 2], remediation: &str) -> Value {
    let (status, evidence, remediation) = match observed {
        Some(true) => ("pass", evidence[0], None),
        Some(false) => ("fail", evidence[1], Some(remediation)),
        None => ("warn", "not observed", Some("Supply complete host observations and measured guest budgets: guest RAM, disk and vCPUs must be positive; RAM headroom must be 0 through 100 percent.")),
    };
    json!({"id": id, "status": status, "evidence": evidence, "remediation": remediation})
}

/// Pure KVM prerequisite and sizing evaluation. It performs no I/O or provisioning.
pub fn computer_host_report(observed: &HostObservations) -> Value {
    let fits = match (observed.budget, observed.free_ram_mib, observed.free_disk_mib, observed.host_cpus) {
        (Some(b), Some(ram), Some(disk), Some(cpus)) if b.guest_ram_mib > 0 && b.guest_disk_mib > 0 && b.guest_vcpus > 0 && b.ram_headroom_percent <= 100 => {
            let ram = (ram.saturating_sub(b.runner_reserved_ram_mib) as u128 * (100 - b.ram_headroom_percent) as u128 / 100 / b.guest_ram_mib as u128) as u64;
            let disk = disk.saturating_sub(b.runner_reserved_disk_mib) / b.guest_disk_mib;
            let cpu = cpus.saturating_sub(b.runner_reserved_cpus) / b.guest_vcpus;
            Some([ram, disk, cpu])
        }
        _ => None,
    };
    let guests = fits.map(|fits| *fits.iter().min().unwrap());
    let checks = [
        computer_check("kvm_device_present", observed.kvm_device_present, ["/dev/kvm present", "/dev/kvm absent"], "Use a Linux host that exposes /dev/kvm; request KVM support from the VPS provider or enable virtualization in firmware."),
        computer_check("kvm_device_access", observed.kvm_device_accessible, ["Runner user can open /dev/kvm", "Runner user cannot open /dev/kvm"], "Grant the Runner user access through the group owning /dev/kvm, then restart its login session; do not elevate Beans."),
        computer_check("capacity", guests.map(|n| n > 0), ["At least one guest fits the supplied measured budgets", "No guest fits after Runner reserves and RAM headroom"], "Free RAM or disk, or use a host with more capacity; retain the measured guest budgets and Runner reserves."),
    ];
    let overall = if checks.iter().any(|c| c["status"] == "fail") { "not_ready" } else if checks.iter().any(|c| c["status"] == "warn") { "unknown" } else { "ready" };
    let capacity = json!({"guests": guests.filter(|_| overall == "ready"), "ram": fits.map(|n| n[0]), "disk": fits.map(|n| n[1]), "cpu": fits.map(|n| n[2])});
    json!({"schema_version": 1, "overall": overall, "checks": checks, "capacity": capacity})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured() -> HostObservations {
        HostObservations {
            kvm_device_present: Some(true), kvm_device_accessible: Some(true),
            free_ram_mib: Some(32768), free_disk_mib: Some(500000), host_cpus: Some(16),
            budget: Some(CapacityBudget {
                guest_ram_mib: 4096, guest_disk_mib: 40960, guest_vcpus: 4,
                runner_reserved_ram_mib: 4096, runner_reserved_disk_mib: 50000,
                runner_reserved_cpus: 2, ram_headroom_percent: 10,
            }),
        }
    }

    #[test]
    fn computer_host_contract() {
        let ready = computer_host_report(&measured());
        assert_eq!(ready["overall"], "ready");
        assert_eq!(ready["capacity"], json!({"guests": 3, "ram": 6, "disk": 10, "cpu": 3}));
        assert_eq!(ready["checks"].as_array().unwrap().len(), 3);
        assert!(ready["checks"].as_array().unwrap().iter().all(|c| c["status"] == "pass" && c["remediation"].is_null()));
        let unknown = computer_host_report(&HostObservations::default());
        assert_eq!(unknown["overall"], "unknown");
        assert!(unknown["capacity"]["guests"].is_null());
        assert!(unknown["checks"].as_array().unwrap().iter().all(|c| c["status"] == "warn" && c["remediation"].is_string()));
        for input in [
            HostObservations { kvm_device_present: Some(false), ..measured() },
            HostObservations { kvm_device_accessible: Some(false), ..measured() },
            HostObservations { free_ram_mib: Some(0), ..measured() },
            HostObservations { free_disk_mib: Some(0), ..measured() },
            HostObservations { host_cpus: Some(0), ..measured() },
        ] {
            let report = computer_host_report(&input);
            assert_eq!(report["overall"], "not_ready");
            assert!(report["capacity"]["guests"].is_null());
            for check in report["checks"].as_array().unwrap().iter().filter(|c| c["status"] == "fail") {
                assert!(check["remediation"].is_string());
                let text = check["remediation"].as_str().unwrap();
                assert!(!text.contains("container") && !text.contains("fallback") && !text.contains("host execution"));
            }
        }
        let budget = measured().budget.unwrap();
        for invalid in [
            CapacityBudget { guest_ram_mib: 0, ..budget }, CapacityBudget { guest_disk_mib: 0, ..budget },
            CapacityBudget { guest_vcpus: 0, ..budget }, CapacityBudget { ram_headroom_percent: 101, ..budget },
        ] {
            let report = computer_host_report(&HostObservations { budget: Some(invalid), ..measured() });
            assert_eq!(report["overall"], "unknown");
            assert_eq!(report["checks"][2]["status"], "warn");
        }
        for missing in [
            HostObservations { budget: None, ..measured() }, HostObservations { host_cpus: None, ..measured() },
            HostObservations { free_ram_mib: None, ..measured() }, HostObservations { free_disk_mib: None, ..measured() },
            HostObservations { kvm_device_present: None, ..measured() }, HostObservations { kvm_device_accessible: None, ..measured() },
        ] {
            assert_eq!(computer_host_report(&missing)["overall"], "unknown");
        }
        let headroom = computer_host_report(&HostObservations { budget: Some(CapacityBudget { ram_headroom_percent: 100, ..budget }), ..measured() });
        assert_eq!(headroom["overall"], "not_ready");
        let reserve = computer_host_report(&HostObservations { budget: Some(CapacityBudget { runner_reserved_ram_mib: u64::MAX, ..budget }), ..measured() });
        assert_eq!(reserve["capacity"]["ram"], 0);
        let maximum = computer_host_report(&HostObservations {
            free_ram_mib: Some(u64::MAX), free_disk_mib: Some(u64::MAX), host_cpus: Some(u64::MAX),
            budget: Some(CapacityBudget { guest_ram_mib: 1, guest_disk_mib: 1, guest_vcpus: 1,
                runner_reserved_ram_mib: 0, runner_reserved_disk_mib: 0, runner_reserved_cpus: 0, ram_headroom_percent: 0 }),
            ..measured()
        });
        assert_eq!(maximum["capacity"]["guests"], json!(u64::MAX));
        assert_eq!(computer_host_report(&measured()), ready);
    }
}
