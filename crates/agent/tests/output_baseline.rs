//! Current-source, provider-free regression for #137. Counts are UTF-8 bytes and
//! ceil(Unicode scalar values / 4), not provider tokens or model-quality evidence.
#![cfg(all(feature = "codemode", unix))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beans_agent::codemode::{CodemodeOptions, CodemodeTool, StaticCatalog};
use beans_agent::tool::{DirectRunner, Tool, ToolOutcome, ToolRunner};
use beans_agent::tools::{BashTool, LsTool, ReadTool};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/output-baseline-current"
);
const WORKER: &str = "BEANS_OUTPUT_BASELINE_WORKER";

// Observe real outcomes; argument validation and execution remain in DirectRunner.
#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String, ToolOutcome)>>);

#[async_trait]
impl ToolRunner for Recorder {
    async fn run(
        &self,
        tool: Arc<dyn Tool>,
        id: String,
        args: Value,
        cancel: CancellationToken,
    ) -> ToolOutcome {
        let outcome = DirectRunner
            .run(tool.clone(), id.clone(), args, cancel)
            .await;
        self.0
            .lock()
            .unwrap()
            .push((id, tool.name().to_string(), outcome.clone()));
        outcome
    }
}

async fn call(tool: Arc<dyn Tool>, args: Value, id: &str) -> ToolOutcome {
    let outcome = DirectRunner
        .run(tool, id.into(), args, CancellationToken::new())
        .await;
    assert!(!outcome.blocked, "{id}: unexpectedly blocked");
    outcome
}

fn counts(text: &str) -> Value {
    json!({"utf8_bytes": text.len(), "estimated_tokens_chars_div_4": text.chars().count().div_ceil(4)})
}

fn markers(text: &str, expected: &[&str]) -> Vec<bool> {
    expected
        .iter()
        .map(|marker| text.contains(marker))
        .collect()
}

fn spill(outcome: &ToolOutcome) -> PathBuf {
    if let Some(path) = outcome.result.details["full_output_path"].as_str() {
        return path.into();
    }
    // Script bash names its sub-1-MB spill in the visible notice, not structured output.
    let text = outcome.result.text_content();
    text.split_once("Full output: ")
        .expect("missing spill notice")
        .1
        .split(']')
        .next()
        .unwrap()
        .into()
}

async fn recover(read: Arc<dyn Tool>, path: &Path) -> String {
    let mut pages = Vec::new();
    let mut offset = 1;
    loop {
        assert!(pages.len() < 100, "read continuation did not finish");
        let outcome = call(
            read.clone(),
            json!({"path": path, "offset": offset}),
            "recover",
        )
        .await;
        assert!(!outcome.is_error, "{}", outcome.result.text_content());
        let text = outcome.result.text_content();
        match text.rsplit_once("\n\n[Showing lines ") {
            Some((body, notice)) => {
                pages.push(body.to_string());
                let next: usize = notice
                    .split_once("Use offset=")
                    .unwrap()
                    .1
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(next > offset, "read continuation made no progress");
                offset = next;
            }
            None => {
                pages.push(text);
                break;
            }
        }
    }
    pages.join("\n")
}

async fn script(codemode: &CodemodeTool, recorder: &Recorder, id: &str, code: &str) -> ToolOutcome {
    // The public executor retains returned JSON separately without bypassing nested validation.
    let run = codemode
        .run_script(id, code, CancellationToken::new(), recorder)
        .await
        .unwrap();
    assert!(!run.result.is_error, "{id}: {}", run.result.text_content());
    let calls = run.result.details["calls"].as_array().unwrap();
    let rows = recorder.0.lock().unwrap();
    for call in calls {
        assert_eq!(
            rows.iter()
                .filter(|(record_id, name, _)| call["id"] == *record_id && call["name"] == *name)
                .count(),
            1
        );
    }
    ToolOutcome {
        result: run.result,
        is_error: false,
        blocked: false,
    }
}

async fn logs(root: &Path, read: Arc<dyn Tool>, recorder: &Recorder) {
    let direct: Arc<dyn Tool> = Arc::new(BashTool::new(root.into()));
    let nested: Arc<dyn Tool> = Arc::new(BashTool::for_script(root.into()));
    let codemode = CodemodeTool::new(
        Arc::new(StaticCatalog::new(vec![nested])),
        CodemodeOptions::default(),
    );
    for (name, exit, expected) in [
        (
            "failing-build.log",
            1,
            &[
                "error[E0308]",
                "error[E0432]",
                "error: linking",
                "could not compile",
            ] as &[&str],
        ),
        (
            "test-output.log",
            101,
            &[
                "parser_utf8 FAILED",
                "relay_retry FAILED",
                "assertion `left == right` failed",
                "test result: FAILED",
            ] as &[&str],
        ),
    ] {
        let raw = std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap();
        assert!(
            raw.len() > 50 * 1024 && raw.len() < 1024 * 1024 && raw.len() > raw.chars().count()
        );
        assert!(markers(&raw, expected).iter().all(|found| *found));
        std::fs::write(root.join(name), &raw).unwrap();
        let command = format!("/bin/cat {name}; exit {exit}");
        let outcome = call(
            direct.clone(),
            json!({"command": command, "description": "Emit synthetic fixture", "timeout": 10}),
            name,
        )
        .await;
        assert!(outcome.is_error && outcome.result.is_error);
        assert_eq!(outcome.result.details["exit_code"], exit);
        let direct_text = outcome.result.text_content();
        assert!(direct_text.contains(&format!("Command exited with code {exit}")));
        assert!(
            markers(&direct_text, expected).iter().any(|found| !found),
            "fixture must expose information lost by tail truncation"
        );
        let direct_path = spill(&outcome);
        assert_eq!(std::fs::read(&direct_path).unwrap(), raw.as_bytes());
        assert_eq!(recover(read.clone(), &direct_path).await, raw);
        std::fs::remove_file(direct_path).unwrap();

        let input = format!(
            "const r = await tools.bash({{command: {}, timeout: 10}});",
            json!(command)
        );
        let emitted = script(
            &codemode,
            recorder,
            &format!("{name}-emit"),
            &format!("{input} text(r.output);"),
        )
        .await;
        let rows = recorder.0.lock().unwrap().clone();
        let (_, _, ingress) = rows
            .iter()
            .find(|(id, _, _)| id == &format!("{name}-emit/1"))
            .unwrap();
        assert!(ingress.is_error && ingress.result.is_error && !ingress.blocked);
        let structured = ingress.result.structured.as_ref().unwrap();
        assert_eq!(structured["exit_code"], exit);
        assert_eq!(structured["truncated"], false);
        assert_eq!(structured["output"].as_str().unwrap(), raw);
        let nested_path = spill(ingress);
        assert_eq!(std::fs::read(&nested_path).unwrap(), raw.as_bytes());
        std::fs::remove_file(nested_path).unwrap();
        let emitted_text = emitted.result.text_content();
        assert!(
            emitted_text.starts_with("Script completed\n")
                && emitted_text.contains("Warning: truncated output")
        );
        let emitted_path = spill(&emitted);
        assert_eq!(std::fs::read(&emitted_path).unwrap(), raw.as_bytes());
        assert_eq!(recover(read.clone(), &emitted_path).await, raw);
        std::fs::remove_file(emitted_path).unwrap();

        // Select diagnostics from actual ingress, never pass the expected checklist to JS.
        let filtered = script(&codemode, recorder, &format!("{name}-filter"), &format!(
            "{input} return {{exit_code:r.exit_code, diagnostics:r.output.split('\\n').filter(line => /error|FAILED|assertion|could not compile/.test(line))}};"
        )).await;
        let filtered_text = filtered.result.text_content();
        assert!(markers(&filtered_text, expected).iter().all(|found| *found));
        let summary: Value = serde_json::from_str(filtered_text.lines().last().unwrap()).unwrap();
        assert_eq!(summary["exit_code"], exit);
        assert_eq!(
            summary["diagnostics"].as_array().unwrap().len(),
            expected.len()
        );
        assert!(filtered.result.details.get("full_output_path").is_none());
        assert!(filtered_text.len() < direct_text.len());
        let rows = recorder.0.lock().unwrap().clone();
        let (_, _, filtered_ingress) = rows
            .iter()
            .find(|(id, _, _)| id == &format!("{name}-filter/1"))
            .unwrap();
        assert_eq!(
            filtered_ingress.result.structured.as_ref().unwrap()["output"]
                .as_str()
                .unwrap(),
            raw
        );
        std::fs::remove_file(spill(filtered_ingress)).unwrap();
        println!(
            "{}",
            json!({"case":name, "raw":counts(&raw), "direct_model_text":counts(&direct_text), "script_ingress":counts(structured["output"].as_str().unwrap()), "emitted_model_text":counts(&emitted_text), "filtered_model_text":counts(&filtered_text), "direct_markers":markers(&direct_text, expected), "emitted_markers":markers(&emitted_text, expected), "filtered_markers":markers(&filtered_text, expected), "task_success":true, "exact_recovery":true})
        );
    }
}

async fn files(root: &Path, read: Arc<dyn Tool>, recorder: &Recorder) {
    let raw = std::fs::read_to_string(Path::new(FIXTURES).join("large-response.json")).unwrap();
    std::fs::write(root.join("large-response.json"), &raw).unwrap();
    let first = call(
        read.clone(),
        json!({"path":"large-response.json"}),
        "json-direct",
    )
    .await;
    assert!(!first.is_error && first.result.details["truncation"]["truncated"] == true);
    assert!(serde_json::from_str::<Value>(&first.result.text_content()).is_err());
    let recovered = recover(read.clone(), &root.join("large-response.json")).await;
    assert_eq!(recovered, raw);
    let parsed: Value = serde_json::from_str(&recovered).unwrap();
    let expected = json!([{"id":7,"code":"SCHEMA_MISMATCH"},{"id":1400,"code":"RETRY_EXHAUSTED"}]);
    let failures: Vec<Value> = parsed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["status"] == "failed")
        .map(|row| json!({"id":row["id"],"code":row["code"]}))
        .collect();
    assert_eq!(json!(failures), expected);

    let listing =
        std::fs::read_to_string(Path::new(FIXTURES).join("directory-listing.txt")).unwrap();
    let staging = root.join("staging");
    std::fs::create_dir(&staging).unwrap();
    for entry in listing.lines() {
        let name = entry.trim_end_matches('/');
        assert!(
            !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
        );
        if entry.ends_with('/') {
            std::fs::create_dir(staging.join(name)).unwrap();
        } else {
            std::fs::write(staging.join(name), []).unwrap();
        }
    }
    let ls: Arc<dyn Tool> = Arc::new(LsTool::new(root.into()));
    let prefix = call(ls.clone(), json!({"path":"staging"}), "ls-direct").await;
    assert!(
        !prefix.is_error
            && prefix
                .result
                .text_content()
                .contains("500 entries limit reached")
    );
    assert!(!prefix.result.text_content().contains("z-final.failed"));
    let complete = call(
        ls.clone(),
        json!({"path":"staging","limit":1000}),
        "ls-recovery",
    )
    .await;
    assert!(!complete.is_error);
    assert_eq!(
        complete.result.text_content(),
        listing.trim_end_matches('\n')
    );

    let codemode = CodemodeTool::new(
        Arc::new(StaticCatalog::new(vec![
            read,
            ls,
            Arc::new(BashTool::for_script(root.into())),
        ])),
        CodemodeOptions::default(),
    );
    // Use read's emitted continuation offsets rather than assuming a page size.
    let code = "let offset = 1, pages = []; for (let i = 0; i < 100; i++) { const page = await tools.read({path:'large-response.json',offset}); const match = page.match(/\\n\\n\\[Showing lines [^\\n]*Use offset=(\\d+) to continue\\.\\]$/); if (!match) { pages.push(page); break; } pages.push(page.slice(0,match.index)); const next = Number(match[1]); if (next <= offset) throw new Error('no progress'); offset = next; } const data = JSON.parse(pages.join('\\n')); return data.jobs.filter(row => row.status === 'failed').map(row => ({id:row.id,code:row.code}));";
    let filtered = script(&codemode, recorder, "json-filter", code).await;
    let json_text = filtered.result.text_content();
    assert_eq!(
        serde_json::from_str::<Value>(json_text.lines().last().unwrap()).unwrap(),
        expected
    );
    assert!(json_text.len() < first.result.text_content().len());
    let selected = script(&codemode, recorder, "listing-filter", "const names = await tools.ls({path:'staging',limit:1000}); return names.split('\\n').filter(name => name.endsWith('.failed') || name.endsWith('/')); ").await;
    let listing_text = selected.result.text_content();
    assert_eq!(
        serde_json::from_str::<Value>(listing_text.lines().last().unwrap()).unwrap(),
        json!([".cache/", "a-first.failed", "m-中.failed", "z-final.failed"])
    );
    assert!(listing_text.len() < prefix.result.text_content().len());
    let rows = recorder.0.lock().unwrap();
    for (case, raw, direct, output, tool, id_prefix) in [
        (
            "json",
            raw.as_str(),
            first.result.text_content(),
            json_text,
            "read",
            "json-filter/",
        ),
        (
            "listing",
            listing.as_str().trim_end_matches('\n'),
            prefix.result.text_content(),
            listing_text,
            "ls",
            "listing-filter/",
        ),
    ] {
        let ingress: Vec<Value> = rows
            .iter()
            .filter(|(id, name, _)| id.starts_with(id_prefix) && name == tool)
            .map(|(_, _, result)| counts(&result.result.text_content()))
            .collect();
        assert!(!ingress.is_empty());
        println!(
            "{}",
            json!({"case":case,"raw":counts(raw),"direct_model_text":counts(&direct),"script_ingress_pages":ingress,"filtered_model_text":counts(&output),"task_success":true,"exact_recovery":true})
        );
    }
}

#[test]
fn provider_free_output_baseline() {
    if let Some(root) = std::env::var_os(WORKER) {
        let root = PathBuf::from(root);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let recorder = Recorder::default();
            let read: Arc<dyn Tool> = Arc::new(ReadTool::new(root.clone()));
            logs(&root, read.clone(), &recorder).await;
            files(&root, read, &recorder).await;
        });
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "beans-output-baseline-current-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&root).unwrap();
    for dir in ["home", "runtime"] {
        std::fs::create_dir(root.join(dir)).unwrap();
    }
    // Re-exec before Tokio/login-shell initialization; no parent credentials or real HOME.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "provider_free_output_baseline", "--nocapture"])
        .env_clear()
        .env(WORKER, &root)
        .env("HOME", root.join("home"))
        .env("ZDOTDIR", root.join("home"))
        .env("TMPDIR", root.join("runtime"))
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/bash")
        .env("BEANS_SHELL", "/bin/bash")
        .current_dir(&root)
        .output()
        .unwrap();
    print!("{}", String::from_utf8_lossy(&output.stdout));
    // Remove only synthetic files and real spills inside this test's fresh directory.
    let cleanup = std::fs::remove_dir_all(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    cleanup.unwrap();
}
