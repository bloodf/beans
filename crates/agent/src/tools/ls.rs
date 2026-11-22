//! `ls`: directory entries, sorted case-insensitively, directories suffixed with `/`.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use super::truncate::{format_size, truncate_head, TruncationOptions, DEFAULT_MAX_BYTES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

const DEFAULT_LIMIT: usize = 500;

pub struct LsTool {
    cwd: PathBuf,
}

impl LsTool {
    pub fn new(cwd: PathBuf) -> Self {
        LsTool { cwd }
    }
}

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }
    fn description(&self) -> &str {
        "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. \
         Output is truncated to 500 entries or 50KB (whichever is hit first)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory to list (default: current directory)" },
                "limit": { "type": "number", "description": "Maximum number of entries to return (default: 500)" }
            }
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let dir = resolve_to_cwd(args["path"].as_str().unwrap_or("."), &self.cwd);
        let limit = args["limit"].as_f64().map(|l| (l as usize).max(1)).unwrap_or(DEFAULT_LIMIT);
        let metadata = tokio::fs::metadata(&dir).await.map_err(|_| ToolError(format!("Path not found: {}", dir.display())))?;
        if !metadata.is_dir() {
            return Err(ToolError(format!("Not a directory: {}", dir.display())));
        }
        let mut read = tokio::fs::read_dir(&dir).await.map_err(|e| ToolError(format!("Cannot read directory: {e}")))?;
        let mut names: Vec<(String, bool)> = Vec::new();
        while let Some(entry) = read.next_entry().await.map_err(|e| ToolError(format!("Cannot read directory: {e}")))? {
            if cancel.is_cancelled() {
                return Err("Operation aborted".into());
            }
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false)
                || tokio::fs::metadata(entry.path()).await.map(|m| m.is_dir()).unwrap_or(false);
            names.push((entry.file_name().to_string_lossy().into_owned(), is_dir));
        }
        names.sort_by_key(|(name, _)| name.to_lowercase());
        let limit_reached = names.len() > limit;
        let results: Vec<String> = names.iter().take(limit).map(|(name, is_dir)| if *is_dir { format!("{name}/") } else { name.clone() }).collect();
        if results.is_empty() {
            return Ok(ToolResult::text("(empty directory)").with_details(json!({ "summary": "Empty directory" })));
        }
        let raw = results.join("\n");
        let truncation = truncate_head(&raw, TruncationOptions { max_lines: Some(usize::MAX), max_bytes: None });
        let mut output = truncation.content.clone();
        let mut notices = Vec::new();
        if limit_reached {
            notices.push(format!("{limit} entries limit reached. Use limit={} for more", limit * 2));
        }
        if truncation.truncated {
            notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        }
        if !notices.is_empty() {
            output.push_str(&format!("\n\n[{}]", notices.join(". ")));
        }
        Ok(ToolResult::text(output).with_details(json!({ "summary": format!("Listed {} entr{}", results.len(), if results.len() == 1 { "y" } else { "ies" }) })))
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

    async fn ls(cwd: &std::path::Path, args: Value) -> Result<ToolResult, ToolError> {
        LsTool::new(cwd.to_path_buf()).execute("ls", args, CancellationToken::new(), Arc::new(|_| {})).await
    }

    #[tokio::test]
    async fn lists_case_insensitively_with_dotfiles_and_directory_slashes() {
        let dir = Scratch::new();
        for name in ["b.txt", "A.txt", ".env"] { std::fs::write(dir.path().join(name), "x").unwrap(); }
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let result = ls(dir.path(), json!({})).await.unwrap();
        assert_eq!(result.text_content(), ".env\nA.txt\nb.txt\nsrc/");
        assert_eq!(result.details["summary"], "Listed 4 entries");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlink_to_a_directory_reads_as_a_directory() {
        let dir = Scratch::new();
        std::fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
        assert_eq!(ls(dir.path(), json!({})).await.unwrap().text_content(), "link/\nreal/");
    }

    #[tokio::test]
    async fn the_entry_limit_says_so_only_past_it() {
        let dir = Scratch::new();
        for name in ["a", "b", "c"] { std::fs::write(dir.path().join(name), "x").unwrap(); }
        let limited = ls(dir.path(), json!({ "limit": 2 })).await.unwrap();
        assert_eq!(limited.text_content(), "a\nb\n\n[2 entries limit reached. Use limit=4 for more]");
        assert_eq!(ls(dir.path(), json!({ "limit": 3 })).await.unwrap().text_content(), "a\nb\nc");
    }

    #[tokio::test]
    async fn errors_and_empty_directories() {
        let dir = Scratch::new();
        assert_eq!(ls(dir.path(), json!({})).await.unwrap().text_content(), "(empty directory)");
        std::fs::write(dir.path().join("file"), "x").unwrap();
        assert!(ls(dir.path(), json!({ "path": "file" })).await.unwrap_err().0.contains("Not a directory"));
        assert!(ls(dir.path(), json!({ "path": "missing" })).await.unwrap_err().0.contains("Path not found"));
    }
}
