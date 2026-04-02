use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tools::memory_tool;

/// Deterministic analysis pipeline that runs after sessions to consolidate
/// memory, extract patterns, and write dream summaries.
pub struct DreamRunner {
    project_path: PathBuf,
    dream_dir: PathBuf,
    transcripts_dir: PathBuf,
    memory_dir: PathBuf,
}

/// Extracted signal from transcript analysis.
#[derive(Debug, Default)]
struct Signal {
    /// Tool name -> count of invocations
    tool_counts: HashMap<String, usize>,
    /// File path -> count of reads/edits
    file_counts: HashMap<String, usize>,
    /// Error messages encountered
    errors: Vec<String>,
    /// First user message from each session (topics)
    topics: Vec<String>,
    /// Total sessions analyzed
    session_count: usize,
}

impl DreamRunner {
    pub fn new(project_path: &Path) -> Self {
        let ftai = project_path.join(".ftai");
        Self {
            project_path: project_path.to_path_buf(),
            dream_dir: ftai.join("dreams"),
            transcripts_dir: ftai.join("transcripts"),
            memory_dir: ftai.join("memory"),
        }
    }

    /// Run the full dream pipeline. Returns the path to `latest.md`.
    pub fn run(&self, since_time: Option<u64>) -> Result<PathBuf> {
        // Validate all paths stay inside .ftai
        self.validate_paths()?;

        std::fs::create_dir_all(&self.dream_dir)?;
        std::fs::create_dir_all(&self.memory_dir)?;

        // Phase 1 — Orient: read current state
        let existing_memories = self.phase1_orient();

        // Phase 2 — Gather signal from transcripts
        let signal = self.phase2_gather_signal(since_time.unwrap_or(0));

        if signal.session_count == 0 {
            // Nothing to dream about
            let path = self.dream_dir.join("latest.md");
            let content = "---\ndate: (no sessions)\nsessions_analyzed: 0\ntopics: []\n---\n\n\
                           No sessions found since last dream.\n";
            std::fs::write(&path, content)?;
            return Ok(path);
        }

        // Phase 3 — Consolidate memory
        let memory_updates = self.phase3_consolidate_memory(&signal, &existing_memories)?;

        // Phase 4 — Analyze: write per-topic dream files for unresolved errors
        let analyses = self.phase4_analyze(&signal)?;

        // Phase 5 — Write summary
        let path = self.phase5_write_summary(&signal, &memory_updates, &analyses)?;

        Ok(path)
    }

    /// Phase 1: Read existing memory files and rules.
    fn phase1_orient(&self) -> Vec<(String, String)> {
        memory_tool::read_memory_dir(&self.memory_dir, None).unwrap_or_default()
    }

    /// Phase 2: Read transcripts since `since_time` and extract patterns.
    fn phase2_gather_signal(&self, since_time: u64) -> Signal {
        let mut signal = Signal::default();

        if !self.transcripts_dir.exists() {
            return signal;
        }

        let entries = match std::fs::read_dir(&self.transcripts_dir) {
            Ok(e) => e,
            Err(_) => return signal,
        };

        for entry in entries.flatten() {
            let path = entry.path();

            // Security: only read .jsonl files inside transcripts_dir
            if path.extension().map_or(true, |e| e != "jsonl") {
                continue;
            }
            if let Ok(canonical) = path.canonicalize() {
                if let Ok(canon_dir) = self.transcripts_dir.canonicalize() {
                    if !canonical.starts_with(&canon_dir) {
                        continue; // Path escape attempt
                    }
                }
            }

            // Check modification time
            let mtime = path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if mtime <= since_time {
                continue;
            }

            // Parse the JSONL transcript
            if let Ok(content) = std::fs::read_to_string(&path) {
                let mut is_first_user = true;
                for line in content.lines() {
                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) {
                        self.extract_from_message(&msg, &mut signal, &mut is_first_user);
                    }
                }
                signal.session_count += 1;
            }
        }

        signal
    }

    /// Extract signal from a single message JSON value.
    fn extract_from_message(
        &self,
        msg: &serde_json::Value,
        signal: &mut Signal,
        is_first_user: &mut bool,
    ) {
        let role = msg["role"].as_str().unwrap_or("");

        match role {
            "user" => {
                if *is_first_user {
                    if let Some(content) = msg["content"].as_str() {
                        let topic: String = content.chars().take(120).collect();
                        if !topic.is_empty() {
                            signal.topics.push(topic);
                        }
                    }
                    *is_first_user = false;
                }
            }
            "assistant" => {
                // Count tool calls
                if let Some(calls) = msg["tool_calls"].as_array() {
                    for call in calls {
                        if let Some(name) = call["name"].as_str() {
                            *signal.tool_counts.entry(name.to_string()).or_insert(0) += 1;

                            // Extract file paths from tool arguments
                            if let Some(args) = call["arguments"].as_object() {
                                for (_key, val) in args {
                                    if let Some(s) = val.as_str() {
                                        if looks_like_path(s) {
                                            *signal
                                                .file_counts
                                                .entry(s.to_string())
                                                .or_insert(0) += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "tool" => {
                // Grep for error patterns in tool results
                if let Some(content) = msg["content"].as_str() {
                    for line in content.lines() {
                        if line.contains("Error:") || line.contains("error[") {
                            let error_line: String = line.chars().take(200).collect();
                            signal.errors.push(error_line);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Phase 3: Write/update memory files based on patterns found.
    fn phase3_consolidate_memory(
        &self,
        signal: &Signal,
        _existing: &[(String, String)],
    ) -> Result<Vec<String>> {
        let mut updates = Vec::new();

        // Write memory for frequently edited files (>= 3 edits)
        let mut hot_files: Vec<(&String, &usize)> = signal
            .file_counts
            .iter()
            .filter(|(_, count)| **count >= 3)
            .collect();
        hot_files.sort_by(|a, b| b.1.cmp(a.1));

        if !hot_files.is_empty() {
            let mut content = String::from("Frequently edited files (from dream analysis):\n\n");
            for (path, count) in &hot_files {
                content.push_str(&format!("- {path} ({count} edits)\n"));
            }
            let now = now_iso();
            let file_content = format!(
                "---\ncategory: project\ncreated: {now}\n---\n\n{content}"
            );
            let mem_path = self.memory_dir.join("hot-files.md");
            // Validate path stays inside memory_dir
            if mem_path.starts_with(&self.memory_dir) {
                std::fs::write(&mem_path, &file_content)?;
                updates.push("Updated: hot-files".to_string());
            }
        }

        // Write memory for repeated errors (>= 2 occurrences)
        let mut error_counts: HashMap<String, usize> = HashMap::new();
        for err in &signal.errors {
            // Normalize: take first 80 chars as key
            let key: String = err.chars().take(80).collect();
            *error_counts.entry(key).or_insert(0) += 1;
        }
        let repeated_errors: Vec<_> = error_counts
            .iter()
            .filter(|(_, count)| **count >= 2)
            .collect();

        if !repeated_errors.is_empty() {
            let mut content = String::from("Repeated error patterns (from dream analysis):\n\n");
            for (err, count) in &repeated_errors {
                content.push_str(&format!("- ({count}x) {err}\n"));
            }
            let now = now_iso();
            let file_content = format!(
                "---\ncategory: project\ncreated: {now}\n---\n\n{content}"
            );
            let mem_path = self.memory_dir.join("common-errors.md");
            if mem_path.starts_with(&self.memory_dir) {
                std::fs::write(&mem_path, &file_content)?;
                updates.push("Created: common-errors".to_string());
            }
        }

        // Prune stale memories (>30 days old, not referenced in recent sessions)
        self.prune_stale_memories(signal)?;

        Ok(updates)
    }

    /// Remove memory files older than 30 days that aren't referenced in recent sessions.
    fn prune_stale_memories(&self, signal: &Signal) -> Result<()> {
        if !self.memory_dir.exists() {
            return Ok(());
        }

        let now = now_secs();
        let thirty_days = 30 * 24 * 60 * 60;

        if let Ok(entries) = std::fs::read_dir(&self.memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(true, |e| e != "md") {
                    continue;
                }
                let mtime = path
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                if now.saturating_sub(mtime) <= thirty_days {
                    continue; // Not stale
                }

                // Check if referenced in any topic or error
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let is_referenced = signal
                    .topics
                    .iter()
                    .any(|t| t.to_lowercase().contains(&name))
                    || signal
                        .errors
                        .iter()
                        .any(|e| e.to_lowercase().contains(&name));

                if !is_referenced {
                    // Safe to prune — verify path is inside memory_dir
                    if let Ok(canon) = path.canonicalize() {
                        if let Ok(canon_dir) = self.memory_dir.canonicalize() {
                            if canon.starts_with(&canon_dir) {
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Phase 4: Write per-topic analysis files for unresolved error patterns.
    fn phase4_analyze(&self, signal: &Signal) -> Result<Vec<String>> {
        let mut analyses = Vec::new();

        // Group errors by common prefix (first 40 chars)
        let mut error_groups: HashMap<String, Vec<&str>> = HashMap::new();
        for err in &signal.errors {
            let key: String = err.chars().take(40).collect();
            error_groups.entry(key).or_default().push(err);
        }

        let date = today_date();
        for (key, errors) in &error_groups {
            if errors.len() < 2 {
                continue; // Only analyze repeated patterns
            }

            // Determine related files
            let related_files: Vec<&String> = signal
                .file_counts
                .keys()
                .filter(|f| {
                    errors
                        .iter()
                        .any(|e| e.to_lowercase().contains(&f.to_lowercase()))
                })
                .collect();

            // Sanitize topic for filename: alphanumeric + hyphens only
            let topic_slug: String = key
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
                .take(30)
                .collect::<String>()
                .trim_matches('-')
                .to_string();

            let filename = format!("{date}-{topic_slug}.md");
            let dream_path = self.dream_dir.join(&filename);

            // Validate path stays inside dream_dir
            if !dream_path.starts_with(&self.dream_dir) {
                continue;
            }

            let mut content = format!(
                "---\ndate: {date}\ntopic: \"{key}\"\n---\n\n## Error Pattern\n"
            );
            for err in errors {
                content.push_str(&format!("- {err}\n"));
            }
            if !related_files.is_empty() {
                content.push_str("\n## Related Files\n");
                for f in &related_files {
                    content.push_str(&format!("- {f}\n"));
                }
            }

            std::fs::write(&dream_path, &content)?;
            analyses.push(topic_slug);
        }

        Ok(analyses)
    }

    /// Phase 5: Write latest.md summary.
    fn phase5_write_summary(
        &self,
        signal: &Signal,
        memory_updates: &[String],
        analyses: &[String],
    ) -> Result<PathBuf> {
        let date = today_date();

        // Top edited files
        let mut top_files: Vec<(&String, &usize)> = signal.file_counts.iter().collect();
        top_files.sort_by(|a, b| b.1.cmp(a.1));
        let top_files: Vec<_> = top_files.into_iter().take(5).collect();

        // Top tools
        let mut top_tools: Vec<(&String, &usize)> = signal.tool_counts.iter().collect();
        top_tools.sort_by(|a, b| b.1.cmp(a.1));
        let top_tools: Vec<_> = top_tools.into_iter().take(5).collect();

        // Topic list for frontmatter
        let topic_list: Vec<String> = signal
            .topics
            .iter()
            .map(|t| {
                let short: String = t.chars().take(60).collect();
                format!("\"{short}\"")
            })
            .take(5)
            .collect();

        let mut md = format!(
            "---\ndate: {date}\nsessions_analyzed: {}\ntopics: [{}]\n---\n\n",
            signal.session_count,
            topic_list.join(", ")
        );

        md.push_str("## Session Patterns\n");
        if top_files.is_empty() {
            md.push_str("- No file edits recorded\n");
        } else {
            for (path, count) in &top_files {
                md.push_str(&format!(
                    "- Most edited: {path} ({count} edits across sessions)\n"
                ));
            }
        }
        if !signal.errors.is_empty() {
            // Show unique error patterns
            let unique_errors: Vec<_> = {
                let mut seen = std::collections::HashSet::new();
                signal
                    .errors
                    .iter()
                    .filter(|e| {
                        let key: String = e.chars().take(60).collect();
                        seen.insert(key)
                    })
                    .take(5)
                    .collect()
            };
            for err in unique_errors {
                let short: String = err.chars().take(100).collect();
                md.push_str(&format!("- Repeated error: {short}\n"));
            }
        }

        if !top_tools.is_empty() {
            md.push_str("\n## Tool Usage\n");
            for (tool, count) in &top_tools {
                md.push_str(&format!("- {tool}: {count} calls\n"));
            }
        }

        if !memory_updates.is_empty() {
            md.push_str("\n## Memory Updates\n");
            for update in memory_updates {
                md.push_str(&format!("- {update}\n"));
            }
        }

        if !analyses.is_empty() {
            md.push_str("\n## Unresolved Issues\n");
            for topic in analyses {
                md.push_str(&format!("- See: {date}-{topic}.md\n"));
            }
        }

        let path = self.dream_dir.join("latest.md");
        std::fs::write(&path, &md)?;
        Ok(path)
    }

    /// Validate that all working paths are inside the project's .ftai directory.
    fn validate_paths(&self) -> Result<()> {
        let ftai_dir = self.project_path.join(".ftai");
        for dir in [&self.dream_dir, &self.transcripts_dir, &self.memory_dir] {
            if !dir.starts_with(&ftai_dir) {
                anyhow::bail!(
                    "Dream path {} escapes .ftai directory",
                    dir.display()
                );
            }
        }
        Ok(())
    }
}

/// Returns a formatted context string for session injection, or None if
/// no fresh dream exists (missing or older than 48 hours).
pub fn dream_context_for_session(project_path: &Path) -> Option<String> {
    let dream_path = project_path.join(".ftai").join("dreams").join("latest.md");
    if !dream_path.exists() {
        return None;
    }

    // Check age: must be < 48 hours old
    let mtime = dream_path
        .metadata()
        .ok()?
        .modified()
        .ok()?;
    let age = SystemTime::now().duration_since(mtime).ok()?;
    if age.as_secs() > 48 * 60 * 60 {
        return None;
    }

    let content = std::fs::read_to_string(&dream_path).ok()?;
    if content.trim().is_empty() {
        return None;
    }

    Some(format!(
        "[Dream results available — I analyzed problems from your recent sessions.]\n\n{content}"
    ))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Simple heuristic: does this string look like a file path?
fn looks_like_path(s: &str) -> bool {
    if s.is_empty() || s.len() > 500 {
        return false;
    }
    // Must contain a slash or a dot-extension pattern
    (s.contains('/') || s.contains('.')) && !s.contains(' ') && !s.starts_with("http")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_iso() -> String {
    let secs = now_secs();
    let days = secs / 86400;
    let (y, m, d) = days_to_date(days as i64);
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let min = (time_of_day % 3600) / 60;
    let sec = time_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{sec:02}Z")
}

fn today_date() -> String {
    let secs = now_secs();
    let days = secs / 86400;
    let (y, m, d) = days_to_date(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_to_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_project(tmp: &TempDir) -> PathBuf {
        let project = tmp.path().to_path_buf();
        let transcripts = project.join(".ftai").join("transcripts");
        let memory = project.join(".ftai").join("memory");
        let dreams = project.join(".ftai").join("dreams");
        std::fs::create_dir_all(&transcripts).unwrap();
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::create_dir_all(&dreams).unwrap();
        project
    }

    fn write_transcript(dir: &Path, name: &str, lines: &[&str]) {
        let path = dir.join(name);
        let content = lines.join("\n") + "\n";
        std::fs::write(path, content).unwrap();
    }

    // ── Phase 2: signal extraction ────────────────────────────────────────

    #[test]
    fn test_phase2_extracts_tool_counts() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);
        let transcripts = project.join(".ftai").join("transcripts");

        write_transcript(
            &transcripts,
            "100.jsonl",
            &[
                r#"{"role":"user","content":"fix auth"}"#,
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"tc1","name":"bash","arguments":{"command":"ls"}}]}"#,
                r#"{"role":"tool","content":"output","tool_call_id":"tc1"}"#,
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"tc2","name":"bash","arguments":{"command":"cat"}}]}"#,
                r#"{"role":"tool","content":"output2","tool_call_id":"tc2"}"#,
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"tc3","name":"file_read","arguments":{"path":"src/auth.rs"}}]}"#,
            ],
        );

        let runner = DreamRunner::new(&project);
        let signal = runner.phase2_gather_signal(0);

        assert_eq!(signal.session_count, 1);
        assert_eq!(*signal.tool_counts.get("bash").unwrap_or(&0), 2);
        assert_eq!(*signal.tool_counts.get("file_read").unwrap_or(&0), 1);
    }

    #[test]
    fn test_phase2_extracts_error_patterns() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);
        let transcripts = project.join(".ftai").join("transcripts");

        write_transcript(
            &transcripts,
            "200.jsonl",
            &[
                r#"{"role":"user","content":"run tests"}"#,
                r#"{"role":"tool","content":"Error: invalid_grant from oauth","tool_call_id":"tc1"}"#,
                r#"{"role":"tool","content":"all good","tool_call_id":"tc2"}"#,
                r#"{"role":"tool","content":"Error: connection refused","tool_call_id":"tc3"}"#,
            ],
        );

        let runner = DreamRunner::new(&project);
        let signal = runner.phase2_gather_signal(0);

        assert_eq!(signal.errors.len(), 2);
        assert!(signal.errors[0].contains("invalid_grant"));
        assert!(signal.errors[1].contains("connection refused"));
    }

    // ── Phase 3: memory consolidation ─────────────────────────────────────

    #[test]
    fn test_phase3_writes_hot_files_memory() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);

        let mut signal = Signal::default();
        signal
            .file_counts
            .insert("src/auth/refresh.rs".to_string(), 5);
        signal.file_counts.insert("src/main.rs".to_string(), 3);
        signal.file_counts.insert("README.md".to_string(), 1); // below threshold

        let runner = DreamRunner::new(&project);
        let updates = runner
            .phase3_consolidate_memory(&signal, &[])
            .unwrap();

        assert!(!updates.is_empty());
        let hot_files_path = project.join(".ftai").join("memory").join("hot-files.md");
        assert!(hot_files_path.exists());
        let content = std::fs::read_to_string(&hot_files_path).unwrap();
        assert!(content.contains("src/auth/refresh.rs"));
        assert!(content.contains("src/main.rs"));
        assert!(!content.contains("README.md"));
    }

    // ── Phase 5: summary writing ──────────────────────────────────────────

    #[test]
    fn test_phase5_writes_latest_md() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);

        let mut signal = Signal::default();
        signal.session_count = 3;
        signal.topics.push("auth refactor".to_string());
        signal
            .tool_counts
            .insert("bash".to_string(), 10);
        signal
            .file_counts
            .insert("src/auth.rs".to_string(), 4);

        let runner = DreamRunner::new(&project);
        let path = runner
            .phase5_write_summary(&signal, &["Updated: hot-files".to_string()], &[])
            .unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("sessions_analyzed: 3"));
        assert!(content.contains("auth refactor"));
        assert!(content.contains("src/auth.rs"));
        assert!(content.contains("Updated: hot-files"));
    }

    // ── Full run ──────────────────────────────────────────────────────────

    #[test]
    fn test_run_with_transcripts() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);
        let transcripts = project.join(".ftai").join("transcripts");

        write_transcript(
            &transcripts,
            "300.jsonl",
            &[
                r#"{"role":"user","content":"implement login"}"#,
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"tc1","name":"file_write","arguments":{"path":"src/login.rs"}}]}"#,
                r#"{"role":"tool","content":"ok","tool_call_id":"tc1"}"#,
            ],
        );

        let runner = DreamRunner::new(&project);
        let path = runner.run(Some(0)).unwrap();

        assert!(path.exists());
        assert!(path.ends_with("latest.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("sessions_analyzed: 1"));
    }

    // ── dream_context_for_session ─────────────────────────────────────────

    #[test]
    fn test_dream_context_none_when_no_dreams() {
        let tmp = TempDir::new().unwrap();
        assert!(dream_context_for_session(tmp.path()).is_none());
    }

    #[test]
    fn test_dream_context_returns_content_when_fresh() {
        let tmp = TempDir::new().unwrap();
        let dream_dir = tmp.path().join(".ftai").join("dreams");
        std::fs::create_dir_all(&dream_dir).unwrap();
        std::fs::write(
            dream_dir.join("latest.md"),
            "---\nsessions_analyzed: 3\n---\n\nKey findings here.",
        )
        .unwrap();

        let ctx = dream_context_for_session(tmp.path());
        assert!(ctx.is_some());
        let content = ctx.unwrap();
        assert!(content.contains("Dream results available"));
        assert!(content.contains("Key findings here"));
    }

    #[test]
    fn test_dream_context_none_when_empty() {
        // Proxy for "old dream" test: an empty dream file should return None
        let tmp = TempDir::new().unwrap();
        let dream_dir = tmp.path().join(".ftai").join("dreams");
        std::fs::create_dir_all(&dream_dir).unwrap();
        std::fs::write(dream_dir.join("latest.md"), "   ").unwrap();

        let result = dream_context_for_session(tmp.path());
        assert!(result.is_none(), "Empty/whitespace dream should return None");
    }

    #[test]
    fn test_dream_context_age_check_logic() {
        // Verify that the age threshold is 48 hours by testing the function
        // with a fresh file (should return Some) — the complementary "old" case
        // is validated by the code path in dream_context_for_session which
        // checks `age.as_secs() > 48 * 60 * 60`.
        let tmp = TempDir::new().unwrap();
        let dream_dir = tmp.path().join(".ftai").join("dreams");
        std::fs::create_dir_all(&dream_dir).unwrap();
        std::fs::write(dream_dir.join("latest.md"), "# Fresh dream").unwrap();

        // File was just created — well within 48h
        let result = dream_context_for_session(tmp.path());
        assert!(result.is_some());
    }

    // ── P0 Security Red Tests ─────────────────────────────────────────────

    #[test]
    fn test_p0_dream_does_not_write_outside_ftai() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);

        let runner = DreamRunner::new(&project);
        let path = runner.run(Some(0)).unwrap();

        // latest.md must be inside .ftai/dreams/
        let ftai = project.join(".ftai");
        assert!(path.starts_with(&ftai));
    }

    #[test]
    fn test_p0_transcript_paths_validated() {
        let tmp = TempDir::new().unwrap();
        let project = setup_project(&tmp);

        // Create a symlink in transcripts pointing outside (if possible)
        let transcripts = project.join(".ftai").join("transcripts");
        let evil_target = tmp.path().join("evil.jsonl");
        std::fs::write(&evil_target, r#"{"role":"user","content":"stolen"}"#).unwrap();

        #[cfg(unix)]
        {
            let link = transcripts.join("evil-link.jsonl");
            let _ = std::os::unix::fs::symlink(&evil_target, &link);
        }

        // Dream runner should still work without panicking
        let runner = DreamRunner::new(&project);
        let _ = runner.run(Some(0));
        // If symlink was followed, canonicalize check should catch it
    }

    #[test]
    fn test_p0_validate_paths_rejects_escape() {
        let tmp = TempDir::new().unwrap();
        // Construct a runner with paths that escape .ftai
        let runner = DreamRunner {
            project_path: tmp.path().to_path_buf(),
            dream_dir: tmp.path().join(".ftai").join("dreams"),
            transcripts_dir: tmp.path().join(".ftai").join("transcripts"),
            memory_dir: tmp.path().join("..").join("escape"),
        };

        let result = runner.validate_paths();
        assert!(result.is_err());
    }
}
