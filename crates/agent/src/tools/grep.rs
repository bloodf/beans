//! `grep`: regex search over files, respecting .gitignore, with pi's output shape.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use super::truncate::{format_size, truncate_head, truncate_line, TruncationOptions, DEFAULT_MAX_BYTES, GREP_MAX_LINE_LENGTH};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

const DEFAULT_LIMIT: usize = 100;

pub struct GrepTool {
    cwd: PathBuf,
}

impl GrepTool {
    pub fn new(cwd: PathBuf) -> Self {
        GrepTool { cwd }
    }
}

struct Match {
    path: PathBuf,
    line_number: usize,
}

fn walk(root: &Path, glob: Option<&str>) -> Result<ignore::Walk, ToolError> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);
    if let Some(glob) = glob {
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        let pattern = if glob.contains('/') { glob.to_string() } else { format!("**/{glob}") };
        overrides.add(&pattern).map_err(|e| ToolError(format!("Invalid glob: {e}")))?;
        builder.overrides(overrides.build().map_err(|e| ToolError(format!("Invalid glob: {e}")))?);
    }
    builder.filter_entry(|entry| entry.file_name() != ".git");
    Ok(builder.build())
}

fn is_probably_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore. \
         Output is truncated to 100 matches or 50KB (whichever is hit first). Long lines are truncated to 500 chars."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Search pattern (regex or literal string)" },
                "path": { "type": "string", "description": "Directory or file to search (default: current directory)" },
                "glob": { "type": "string", "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'" },
                "ignoreCase": { "type": "boolean", "description": "Case-insensitive search (default: false)" },
                "literal": { "type": "boolean", "description": "Treat pattern as literal string instead of regex (default: false)" },
                "context": { "type": "number", "description": "Number of lines to show before and after each match (default: 0)" },
                "limit": { "type": "number", "description": "Maximum number of matches to return (default: 100)" }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let pattern = args["pattern"].as_str().ok_or("pattern is required")?.to_string();
        let search_path = resolve_to_cwd(args["path"].as_str().unwrap_or("."), &self.cwd);
        let glob = args["glob"].as_str().map(str::to_string);
        let ignore_case = args["ignoreCase"].as_bool().or(args["ignore_case"].as_bool()).unwrap_or(false);
        let literal = args["literal"].as_bool().unwrap_or(false);
        let context = args["context"].as_f64().map(|c| c.max(0.0) as usize).unwrap_or(0);
        let limit = args["limit"].as_f64().map(|l| (l as usize).max(1)).unwrap_or(DEFAULT_LIMIT);

        let metadata = tokio::fs::metadata(&search_path).await.map_err(|_| ToolError(format!("Path not found: {}", search_path.display())))?;
        let is_directory = metadata.is_dir();
        let regex = regex::RegexBuilder::new(&if literal { regex::escape(&pattern) } else { pattern.clone() })
            .case_insensitive(ignore_case)
            .build()
            .map_err(|e| ToolError(format!("Invalid pattern: {e}")))?;

        let cwd_root = search_path.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(Vec<Match>, bool, std::collections::HashMap<PathBuf, Vec<String>>), ToolError> {
            let mut matches = Vec::new();
            let mut limit_reached = false;
            let mut files: std::collections::HashMap<PathBuf, Vec<String>> = std::collections::HashMap::new();
            let entries: Vec<PathBuf> = if is_directory {
                walk(&cwd_root, glob.as_deref())?
                    .filter_map(Result::ok)
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .map(|e| e.into_path())
                    .collect()
            } else {
                vec![cwd_root.clone()]
            };
            'files: for file in entries {
                if cancel.is_cancelled() {
                    return Err("Operation aborted".into());
                }
                let Ok(bytes) = std::fs::read(&file) else { continue };
                if is_probably_binary(&bytes) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\r', "\n");
                let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
                for (index, line) in lines.iter().enumerate() {
                    if regex.is_match(line) {
                        matches.push(Match { path: file.clone(), line_number: index + 1 });
                        if matches.len() >= limit {
                            limit_reached = true;
                            files.insert(file.clone(), lines);
                            break 'files;
                        }
                    }
                }
                if matches.iter().any(|m| m.path == file) {
                    files.insert(file, lines);
                }
            }
            Ok((matches, limit_reached, files))
        })
        .await
        .map_err(|e| ToolError(e.to_string()))??;

        let (matches, limit_reached, files) = result;
        if matches.is_empty() {
            return Ok(ToolResult::text("No matches found").with_details(json!({ "summary": "No matches" })));
        }

        let format_path = |path: &Path| -> String {
            if is_directory {
                if let Ok(relative) = path.strip_prefix(&search_path) {
                    return relative.to_string_lossy().replace('\\', "/");
                }
            }
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        };

        let mut out_lines = Vec::new();
        let mut lines_truncated = false;
        for m in &matches {
            let lines = files.get(&m.path).cloned().unwrap_or_default();
            let relative = format_path(&m.path);
            if lines.is_empty() {
                out_lines.push(format!("{relative}:{}: (unable to read file)", m.line_number));
                continue;
            }
            let start = if context > 0 { m.line_number.saturating_sub(context).max(1) } else { m.line_number };
            let end = if context > 0 { (m.line_number + context).min(lines.len()) } else { m.line_number };
            for current in start..=end {
                let text = lines.get(current - 1).map(String::as_str).unwrap_or("");
                let (shown, was_truncated) = truncate_line(text, GREP_MAX_LINE_LENGTH);
                if was_truncated {
                    lines_truncated = true;
                }
                if current == m.line_number {
                    out_lines.push(format!("{relative}:{current}: {shown}"));
                } else {
                    out_lines.push(format!("{relative}-{current}- {shown}"));
                }
            }
        }

        let raw = out_lines.join("\n");
        let truncation = truncate_head(&raw, TruncationOptions { max_lines: Some(usize::MAX), max_bytes: None });
        let mut output = truncation.content.clone();
        let mut notices = Vec::new();
        if limit_reached {
            notices.push(format!("{limit} matches limit reached. Use limit={} for more, or refine pattern", limit * 2));
        }
        if truncation.truncated {
            notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        }
        if lines_truncated {
            notices.push(format!("Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"));
        }
        if !notices.is_empty() {
            output.push_str(&format!("\n\n[{}]", notices.join(". ")));
        }
        Ok(ToolResult::text(output).with_details(json!({
            "summary": format!("{} match{} for {pattern}", matches.len(), if matches.len() == 1 { "" } else { "es" }),
            "match_limit_reached": limit_reached,
            "lines_truncated": lines_truncated,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A scratch directory removed when the test ends.
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("beans-tool-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn grep(cwd: &Path, args: Value) -> Result<ToolResult, ToolError> {
        GrepTool::new(cwd.to_path_buf()).execute("grep", args, CancellationToken::new(), Arc::new(|_| {})).await
    }

    fn tree(files: &[(&str, &[u8])]) -> Scratch {
        let dir = Scratch::new();
        for (path, bytes) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        dir
    }

    fn lines(result: &ToolResult) -> Vec<String> {
        let mut lines: Vec<String> = result.text_content().split("\n\n[").next().unwrap().lines().map(str::to_string).collect();
        lines.sort();
        lines
    }

    #[tokio::test]
    async fn matches_carry_relative_paths_and_line_numbers() {
        let dir = tree(&[("a.rs", b"fn one() {}\nlet two = 2;\nfn three() {}\n"), ("src/b.rs", b"nothing\nfn four() {}\n")]);
        let result = grep(dir.path(), json!({ "pattern": "^fn " })).await.unwrap();
        assert_eq!(lines(&result), ["a.rs:1: fn one() {}", "a.rs:3: fn three() {}", "src/b.rs:2: fn four() {}"]);
        assert_eq!(result.details["summary"], "3 matches for ^fn ");
        assert_eq!(result.details["match_limit_reached"], false);
    }

    #[tokio::test]
    async fn literal_and_case_options() {
        let dir = tree(&[("f.txt", b"a.c\nabc\nABC\n")]);
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "a.c" })).await.unwrap()), ["f.txt:1: a.c", "f.txt:2: abc"]);
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "a.c", "literal": true })).await.unwrap()), ["f.txt:1: a.c"]);
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "abc", "ignoreCase": true })).await.unwrap()), ["f.txt:2: abc", "f.txt:3: ABC"]);
        // The snake_case spelling some models send is accepted too.
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "abc", "ignore_case": true })).await.unwrap()).len(), 2);
    }

    #[tokio::test]
    async fn glob_filters_files_by_name_anywhere_or_by_path() {
        let dir = tree(&[("a.ts", b"hit\n"), ("src/b.ts", b"hit\n"), ("src/c.rs", b"hit\n")]);
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "hit", "glob": "*.ts" })).await.unwrap()), ["a.ts:1: hit", "src/b.ts:1: hit"]);
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "hit", "glob": "src/*.rs" })).await.unwrap()), ["src/c.rs:1: hit"]);
        assert!(grep(dir.path(), json!({ "pattern": "hit", "glob": "[" })).await.unwrap_err().0.contains("Invalid glob"));
    }

    #[tokio::test]
    async fn context_lines_use_dashes_and_stay_inside_the_file() {
        let dir = tree(&[("f.txt", b"one\ntwo\nthree\nfour\n")]);
        let result = grep(dir.path(), json!({ "pattern": "^one$", "context": 2 })).await.unwrap();
        assert_eq!(result.text_content(), "f.txt:1: one\nf.txt-2- two\nf.txt-3- three");
        let result = grep(dir.path(), json!({ "pattern": "four", "context": 1 })).await.unwrap();
        assert_eq!(result.text_content(), "f.txt-3- three\nf.txt:4: four\nf.txt-5- ");
    }

    #[tokio::test]
    async fn the_match_limit_stops_the_search_and_says_so() {
        let dir = tree(&[("f.txt", b"x\nx\nx\nx\n")]);
        let result = grep(dir.path(), json!({ "pattern": "x", "limit": 2 })).await.unwrap();
        assert!(result.text_content().starts_with("f.txt:1: x\nf.txt:2: x\n\n[2 matches limit reached. Use limit=4 for more"), "{}", result.text_content());
        assert_eq!(result.details["match_limit_reached"], true);
        // A limit below one means one.
        assert_eq!(grep(dir.path(), json!({ "pattern": "x", "limit": 0 })).await.unwrap().text_content().lines().next(), Some("f.txt:1: x"));
    }

    #[tokio::test]
    async fn binary_files_are_skipped_and_long_lines_truncated() {
        let long = format!("needle {}\n", "y".repeat(GREP_MAX_LINE_LENGTH + 50));
        let dir = tree(&[("bin.dat", b"needle\0binary"), ("long.txt", long.as_bytes())]);
        let result = grep(dir.path(), json!({ "pattern": "needle" })).await.unwrap();
        let text = result.text_content();
        assert!(!text.contains("bin.dat"), "{text}");
        assert!(text.starts_with("long.txt:1: needle "), "{text}");
        assert!(text.contains(&format!("Some lines truncated to {GREP_MAX_LINE_LENGTH} chars")), "{text}");
        assert_eq!(result.details["lines_truncated"], true);
    }

    #[tokio::test]
    async fn crlf_and_cr_line_endings_split_lines() {
        let dir = tree(&[("win.txt", b"one\r\ntwo\rthree\n")]);
        assert_eq!(lines(&grep(dir.path(), json!({ "pattern": "t" })).await.unwrap()), ["win.txt:2: two", "win.txt:3: three"]);
    }

    #[tokio::test]
    async fn gitignore_applies_inside_a_repository_and_git_internals_are_never_searched() {
        let dir = tree(&[(".gitignore", b"ignored.txt\n"), ("ignored.txt", b"needle\n"), ("kept.txt", b"needle\n"), (".git/HEAD", b"needle\n"), (".hidden", b"needle\n")]);
        let found = lines(&grep(dir.path(), json!({ "pattern": "needle" })).await.unwrap());
        assert_eq!(found, [".hidden:1: needle", "kept.txt:1: needle"]);
    }

    #[tokio::test]
    async fn a_single_file_path_names_the_file() {
        let dir = tree(&[("src/one.rs", b"needle\n")]);
        let result = grep(dir.path(), json!({ "pattern": "needle", "path": "src/one.rs" })).await.unwrap();
        assert_eq!(result.text_content(), "one.rs:1: needle");
    }

    #[tokio::test]
    async fn errors_and_empty_results() {
        let dir = tree(&[("f.txt", b"text\n")]);
        assert!(grep(dir.path(), json!({})).await.unwrap_err().0.contains("pattern is required"));
        assert!(grep(dir.path(), json!({ "pattern": "(" })).await.unwrap_err().0.contains("Invalid pattern"));
        assert!(grep(dir.path(), json!({ "pattern": "x", "path": "missing" })).await.unwrap_err().0.contains("Path not found"));
        let none = grep(dir.path(), json!({ "pattern": "absent" })).await.unwrap();
        assert_eq!(none.text_content(), "No matches found");
        assert_eq!(none.details["summary"], "No matches");
    }

    #[tokio::test]
    async fn a_cancelled_search_aborts() {
        let dir = tree(&[("f.txt", b"needle\n")]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = GrepTool::new(dir.path().to_path_buf()).execute("grep", json!({ "pattern": "needle" }), cancel, Arc::new(|_| {})).await.unwrap_err();
        assert_eq!(error.0, "Operation aborted");
    }
}
