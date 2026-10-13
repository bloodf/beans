//! Skills: `SKILL.md` files (agentskills.io) the model is told about and reads when a task
//! matches, after pi's skill loader.

use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

/// A skill loaded from a `SKILL.md` file or given by the application. `name`, `description`,
/// and `path` go into the system prompt; `content` is what the model reads on invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    /// The file's absolute path, for the model's listing and relative references.
    pub path: PathBuf,
    /// Left out of the model's listing; the application can still invoke it.
    pub disable_model_invocation: bool,
}

/// A problem found while loading, with the file it concerns. Loading goes on regardless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadedSkills {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

/// Loads skills from directories: every `SKILL.md` below them (the first one in a directory
/// wins and its subdirectories are not searched), plus `.md` files directly in a root that
/// carry a `description` (named after the file). Ignore files are honored; a missing directory
/// is skipped.
pub fn load_skills(dirs: &[PathBuf]) -> LoadedSkills {
    let mut loaded = LoadedSkills::default();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut walker = ignore::WalkBuilder::new(dir);
        walker.hidden(true).git_ignore(true).ignore(true).filter_entry(|entry| entry.file_name() != "node_modules");
        let mut claimed: Vec<PathBuf> = Vec::new();
        let mut entries: Vec<PathBuf> = walker.build().filter_map(Result::ok).filter(|e| e.file_type().is_some_and(|t| t.is_file())).map(|e| e.into_path()).collect();
        entries.sort();
        // A SKILL.md claims its directory: nothing below it is another skill.
        let skill_files: Vec<PathBuf> = entries.iter().filter(|p| p.file_name().is_some_and(|n| n == "SKILL.md")).cloned().collect();
        for file in skill_files {
            let parent = file.parent().map(Path::to_path_buf).unwrap_or_default();
            if claimed.iter().any(|c| parent.starts_with(c) && parent != *c) {
                continue;
            }
            claimed.push(parent.clone());
            load_file(&file, true, &mut loaded);
        }
        for file in entries.iter().filter(|p| p.parent() == Some(dir.as_path()) && p.extension().is_some_and(|e| e == "md") && p.file_name().is_some_and(|n| n != "SKILL.md")) {
            load_file(file, false, &mut loaded);
        }
    }
    loaded
}

fn load_file(path: &Path, declared: bool, into: &mut LoadedSkills) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: format!("cannot read: {error}") });
            return;
        }
    };
    let parsed = parse_frontmatter(&text);
    let description = parsed.get("description").map(str::trim).unwrap_or("");
    if description.is_empty() {
        if declared {
            into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: "description is required".into() });
        }
        return;
    }
    if description.len() > MAX_DESCRIPTION_LENGTH {
        into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: format!("description exceeds {MAX_DESCRIPTION_LENGTH} characters") });
    }
    let parent_name = path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // A declared skill is named after its directory; a root `.md` file after itself.
    let fallback = if declared { parent_name.clone() } else { path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() };
    let name = parsed.get("name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or(fallback);
    for problem in validate_name(&name, &parent_name, declared) {
        into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: problem });
    }
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    into.skills.push(Skill {
        name,
        description: description.to_string(),
        content: parsed.body.clone(),
        path: absolute,
        disable_model_invocation: parsed.flag("disable-model-invocation"),
    });
}

fn validate_name(name: &str, parent: &str, declared: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if declared && name != parent {
        problems.push(format!("name \"{name}\" does not match parent directory \"{parent}\""));
    }
    if name.len() > MAX_NAME_LENGTH {
        problems.push(format!("name exceeds {MAX_NAME_LENGTH} characters"));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        problems.push("name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)".into());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        problems.push("name must not start or end with a hyphen or contain consecutive hyphens".into());
    }
    problems
}

/// The prompt that invokes a skill: its content in a `<skill>` block, plus what the user added.
pub fn format_skill_invocation(skill: &Skill, additional_instructions: Option<&str>) -> String {
    let dir = skill.path.parent().map(|p| p.display().to_string()).unwrap_or_else(|| "/".into());
    let block = format!("<skill name=\"{}\" location=\"{}\">\nReferences are relative to {dir}.\n\n{}\n</skill>", skill.name, skill.path.display(), skill.content);
    match additional_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        Some(extra) => format!("{block}\n\n{extra}"),
        None => block,
    }
}

/// The system prompt block listing the skills the model may read, in agentskills.io's shape.
/// Empty when no skill is model-visible.
pub fn format_skills_for_system_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills.iter().filter(|s| !s.disable_model_invocation).collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        "Read the full skill file when the task matches its description.".to_string(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];
    for skill in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!("    <description>{}</description>", escape_xml(&skill.description)));
        lines.push(format!("    <location>{}</location>", escape_xml(&skill.path.display().to_string())));
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

fn escape_xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_declared_skills_and_root_markdown_and_reports_problems() {
        let root = std::env::temp_dir().join(format!("beans-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("deploy/nested")).unwrap();
        std::fs::create_dir_all(root.join("Bad Name")).unwrap();
        std::fs::write(root.join("deploy/SKILL.md"), "---\nname: deploy\ndescription: Ship a release\n---\nSteps here.").unwrap();
        std::fs::write(root.join("deploy/nested/SKILL.md"), "---\nname: nested\ndescription: hidden below a skill\n---\nx").unwrap();
        std::fs::write(root.join("Bad Name/SKILL.md"), "---\ndescription: no name field\n---\nx").unwrap();
        std::fs::write(root.join("notes.md"), "---\ndescription: A root note skill\ndisable-model-invocation: true\n---\nnote body").unwrap();
        std::fs::write(root.join("plain.md"), "no frontmatter").unwrap();
        let loaded = load_skills(&[root.clone(), root.join("missing")]);
        let names: Vec<&str> = loaded.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Bad Name", "deploy", "notes"]);
        assert!(loaded.skills.iter().any(|s| s.name == "notes" && s.disable_model_invocation && s.content == "note body"));
        assert!(loaded.diagnostics.iter().any(|d| d.message.contains("invalid characters")));
        let prompt = format_skills_for_system_prompt(&loaded.skills);
        assert!(prompt.contains("<name>deploy</name>") && prompt.contains("Ship a release") && !prompt.contains("<name>notes</name>"));
        let invocation = format_skill_invocation(&loaded.skills[1], Some("focus on staging"));
        assert!(invocation.starts_with("<skill name=\"deploy\"") && invocation.ends_with("focus on staging"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Hosts in the accepted r3 matrix, each with the four committed fixture cases. Inert
    /// text only: nothing under `tests/fixtures/skill-hosts` runs.
    const HOSTS: &[&str] = &["claude", "cursor", "codex", "pi"];

    fn fixture_root(host: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skill-hosts").join(host)
    }

    /// The name each host's extended fixture declares: the others match their directory.
    fn host_name(host: &str) -> String { if host == "pi" { "pi-extended".into() } else { format!("{host}-extended") } }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() { copy_tree(&entry.path(), &target); } else { std::fs::copy(entry.path(), &target).unwrap(); }
        }
    }

    #[test]
    fn host_matrix_fixtures_load_as_expected() {
        let temp = std::env::temp_dir().join(format!("beans-host-matrix-{}", uuid::Uuid::new_v4()));
        let mut failures: Vec<String> = Vec::new();
        for host in HOSTS {
            let source = fixture_root(host);
            assert!(source.is_dir(), "missing fixture root {}", source.display());
            // (fixture directory, expected loaded names, expected diagnostic fragments)
            // Beans validates one rule for every host: a declared name that differs from its
            // directory is diagnosed. There is no host special case in the loader.
            let extended_name = host_name(host);
            let cases: Vec<(&str, Vec<&str>, Vec<&str>)> = vec![
                ("minimal", vec!["minimal"], vec![]),
                ("extended", vec![&extended_name], vec![]),
                ("duplicate-a", vec!["shared-name"], vec![]),
                ("duplicate-b", vec!["shared-name"], vec![]),
                ("malformed", vec![], vec!["description is required"]),
            ];
            for (case, want_names, want_diagnostics) in cases {
                let case_dir = if case == "extended" && *host == "pi" { source.join("extended-dir") } else { source.join(case) };
                let root = temp.join(host).join(case);
                copy_tree(&case_dir, &root);
                // The diagnostic names the copied file's parent, not the source fixture directory.
                let parent = root.file_name().unwrap().to_string_lossy().to_string();
                let mut want_diagnostics: Vec<String> = want_diagnostics.iter().map(|s| s.to_string()).collect();
                // Only the cases whose declared name differs from their directory are diagnosed.
                if matches!(case, "extended" | "duplicate-a" | "duplicate-b") {
                    want_diagnostics.push(format!("does not match parent directory \"{parent}\""));
                }
                let loaded = load_skills(&[root]);
                let names: Vec<&str> = loaded.skills.iter().map(|s| s.name.as_str()).collect();
                let messages: Vec<&str> = loaded.diagnostics.iter().map(|d| d.message.as_str()).collect();
                if names != want_names { failures.push(format!("{host}/{case}: loaded {names:?}, expected {want_names:?}")); }
                for want in &want_diagnostics {
                    if !messages.iter().any(|m| m.contains(want.as_str())) { failures.push(format!("{host}/{case}: missing diagnostic {want:?} in {messages:?}")); }
                }
                if case == "extended" {
                    if host == &"claude" && !loaded.skills.iter().any(|s| s.disable_model_invocation) {
                        failures.push("claude/extended: disable-model-invocation was not honoured".into());
                    }
                    // Host extension metadata stays inert text: one skill, the declared body, and
                    // no sidecar or extra frontmatter surfacing as a skill of its own.
                    let body = parse_frontmatter(&std::fs::read_to_string(case_dir.join("SKILL.md")).unwrap()).body;
                    let Some(skill) = loaded.skills.first() else { failures.push(format!("{host}/extended: nothing loaded")); continue };
                    if loaded.skills.len() != 1 || skill.content != body {
                        failures.push(format!("{host}/extended: expected one skill with the declared body, got {:?}", loaded.skills));
                    }
                    for marker in ["allow_implicit_invocation", "display_name"] {
                        if loaded.skills.iter().any(|s| s.content.contains(marker)) {
                            failures.push(format!("{host}/extended: sidecar content {marker:?} reached a skill body"));
                        }
                    }
                    // The sidecar directory is never walked: no skill path may enter it.
                    if loaded.skills.iter().any(|s| s.path.components().any(|c| c.as_os_str() == "agents")) {
                        failures.push(format!("{host}/extended: a sidecar path became a skill: {:?}", loaded.skills));
                    }
                    if host == &"claude" && !skill.content.contains("!`git status`") {
                        failures.push("claude/extended: the inert dynamic-context line is not in the body".into());
                    }
                }
            }
            // Both halves of the duplicate pair in one root: two distinct skills, never merged.
            let pair = temp.join(host).join("duplicate-pair");
            copy_tree(&source.join("duplicate-a"), &pair.join("duplicate-a"));
            copy_tree(&source.join("duplicate-b"), &pair.join("duplicate-b"));
            let loaded = load_skills(&[pair]);
            let names: Vec<&str> = loaded.skills.iter().map(|s| s.name.as_str()).collect();
            if names != vec!["shared-name", "shared-name"] {
                failures.push(format!("{host}: duplicate pair loaded {names:?}, expected two distinct skills"));
            }
            // Compare distinct entries only: a pair that collapsed would share one path.
            if let [first, second] = loaded.skills.as_slice() {
                if first.path == second.path {
                    failures.push(format!("{host}: duplicate pair collapsed onto {}", first.path.display()));
                }
            }
            if loaded.diagnostics.len() != 2 {
                failures.push(format!("{host}: duplicate pair produced {} diagnostics, expected one per half", loaded.diagnostics.len()));
            }
        }
        let _ = std::fs::remove_dir_all(&temp);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The bounded scanner is the loader that enforces escape and size limits; the generic
    /// listing loader has no size or symlink policy. Off Linux it fails closed with no opens,
    /// which is not proof that the escape and size logic exists.
    #[test]
    fn bounded_scanner_rejects_symlink_escape_and_oversize() {
        let temp = std::env::temp_dir().join(format!("beans-host-bounds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        // The scanner opens the root from `/` with no symlink resolution, so a symlinked
        // system temp must be resolved here, as the sibling discovery regression does.
        let temp = temp.canonicalize().unwrap();
        let root = temp.join("root");
        let outside = temp.join("outside");
        std::fs::create_dir_all(root.join("valid")).unwrap();
        std::fs::create_dir_all(root.join("oversize")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("valid/SKILL.md"), "---\nname: valid\ndescription: In bounds\n---\nbody").unwrap();
        std::fs::write(root.join("oversize/SKILL.md"), "---\nname: oversize\ndescription: Too large\n---\n").unwrap();
        let oversize = std::fs::OpenOptions::new().write(true).open(root.join("oversize/SKILL.md")).unwrap();
        oversize.set_len(crate::harness::skill_discovery::MAX_FILE_BYTES + 1).unwrap();
        // A real file outside the root, not a dangling link: the escape must have a target.
        std::fs::write(outside.join("SKILL.md"), "---\nname: outside\ndescription: forbidden\n---\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
            std::os::unix::fs::symlink(outside.join("SKILL.md"), root.join("file-escape")).unwrap();
        }
        let result = crate::harness::skill_discovery::discover_skills(&root);
        #[cfg(target_os = "linux")]
        {
            assert!(result.skills.iter().any(|s| s.path == Path::new("valid/SKILL.md")), "{result:?}");
            assert!(!result.skills.iter().any(|s| s.fields.get("name").map(String::as_str) == Some("outside")), "{result:?}");
            for fragment in ["symlink rejected", "exceeds"] {
                assert!(result.diagnostics.iter().any(|d| d.message.contains(fragment)), "missing {fragment}: {:?}", result.diagnostics);
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(result.skills.is_empty() && result.diagnostics.iter().any(|d| d.message.contains("unsupported")), "{result:?}");
        }
        let _ = std::fs::remove_dir_all(&temp);
    }
}
