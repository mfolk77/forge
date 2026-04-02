use std::path::Path;

/// A rule file with optional YAML frontmatter containing glob patterns.
#[derive(Debug, Clone)]
pub struct GlobRule {
    pub globs: Vec<String>,
    pub always_apply: bool,
    pub rule_content: String,
}

/// Parse YAML frontmatter from a rule file's content.
///
/// Supports frontmatter delimited by `---` lines:
/// ```text
/// ---
/// globs: ["**/*.rs", "**/*.toml"]
/// alwaysApply: false
/// ---
///
/// rule "example" { ... }
/// ```
///
/// If no frontmatter is present, returns a GlobRule with empty globs and
/// `always_apply: true` for backward compatibility.
pub fn parse_rule_file(content: &str) -> GlobRule {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return GlobRule {
            globs: Vec::new(),
            always_apply: true,
            rule_content: content.to_string(),
        };
    }

    // Find the closing --- delimiter
    let after_first = &trimmed[3..];
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    if let Some(end_idx) = after_first.find("\n---") {
        let frontmatter = &after_first[..end_idx];
        let rest_start = end_idx + 4; // skip "\n---"
        let rule_content = if rest_start < after_first.len() {
            after_first[rest_start..].trim_start_matches('\n').to_string()
        } else {
            String::new()
        };

        let (globs, always_apply) = parse_frontmatter(frontmatter);

        GlobRule {
            globs,
            always_apply,
            rule_content,
        }
    } else {
        // Malformed frontmatter (no closing ---), treat entire content as rule
        GlobRule {
            globs: Vec::new(),
            always_apply: true,
            rule_content: content.to_string(),
        }
    }
}

/// Parse the YAML-like frontmatter block for globs and alwaysApply.
/// This is a minimal parser — not a full YAML parser — to avoid adding a dependency.
fn parse_frontmatter(fm: &str) -> (Vec<String>, bool) {
    let mut globs = Vec::new();
    let mut always_apply = true; // default

    for line in fm.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("alwaysApply:") {
            let val = rest.trim();
            always_apply = val == "true";
        } else if let Some(rest) = line.strip_prefix("globs:") {
            globs = parse_string_array(rest.trim());
        }
    }

    (globs, always_apply)
}

/// Parse a JSON-style string array: `["**/*.rs", "**/*.toml"]`
fn parse_string_array(s: &str) -> Vec<String> {
    let s = s.trim();
    if !s.starts_with('[') || !s.ends_with(']') {
        return Vec::new();
    }
    let inner = &s[1..s.len() - 1];
    inner
        .split(',')
        .map(|item| {
            item.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Check if any glob pattern in the rule matches the given file path.
///
/// Uses `glob::Pattern::matches_path_with()` with default match options.
pub fn matches_path(rule: &GlobRule, path: &str) -> bool {
    if rule.globs.is_empty() {
        return false;
    }

    let match_opts = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    let file_path = Path::new(path);

    for pattern_str in &rule.globs {
        // Security: reject patterns containing path traversal components
        if pattern_str.contains("..") {
            continue;
        }

        if let Ok(pattern) = glob::Pattern::new(pattern_str) {
            if pattern.matches_path_with(file_path, match_opts) {
                return true;
            }
        }
    }

    false
}

/// Scan a directory for `.ftai` rule files, parse each, and return only those
/// that match the current context.
///
/// Matching logic:
/// - `always_apply: true` -> always returned
/// - `always_apply: false` + globs -> returned only if file_path matches any glob
/// - `always_apply: false` + no globs -> returned (backward compat: treated as always_apply)
pub fn load_rules_for_context(dir: &Path, file_path: Option<&str>) -> Vec<GlobRule> {
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process .ftai files
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("ftai") {
            continue;
        }

        // Security: skip symlinks that could escape the directory
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if metadata.file_type().is_symlink() {
                if let Ok(resolved) = std::fs::canonicalize(&path) {
                    let canon_dir = match std::fs::canonicalize(dir) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    if !resolved.starts_with(&canon_dir) {
                        continue; // symlink escapes the directory
                    }
                }
            }
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let rule = parse_rule_file(&content);

        if should_include(&rule, file_path) {
            results.push(rule);
        }
    }

    results
}

/// Determine whether a GlobRule should be included given the current file context.
fn should_include(rule: &GlobRule, file_path: Option<&str>) -> bool {
    // always_apply rules are always included
    if rule.always_apply {
        return true;
    }

    // No globs + not always_apply -> backward compat: treat as always_apply
    if rule.globs.is_empty() {
        return true;
    }

    // Has globs -> only include if file_path matches
    if let Some(path) = file_path {
        matches_path(rule, path)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Test 1: Parse frontmatter with globs ────────────────────────────

    #[test]
    fn test_parse_frontmatter_with_globs() {
        let content = r#"---
globs: ["**/*.rs", "**/*.toml"]
alwaysApply: false
---

rule "rust-guard" {
  on commit
  reject false
}
"#;
        let rule = parse_rule_file(content);
        assert_eq!(rule.globs, vec!["**/*.rs", "**/*.toml"]);
        assert!(!rule.always_apply);
        assert!(rule.rule_content.contains("rust-guard"));
    }

    // ── Test 2: Parse frontmatter without globs (backward compat) ───────

    #[test]
    fn test_parse_frontmatter_without_globs() {
        let content = r#"---
alwaysApply: true
---

rule "general" {
  on commit
  reject false
}
"#;
        let rule = parse_rule_file(content);
        assert!(rule.globs.is_empty());
        assert!(rule.always_apply);
        assert!(rule.rule_content.contains("general"));
    }

    // ── Test 3: Parse file with no frontmatter (backward compat) ────────

    #[test]
    fn test_parse_no_frontmatter() {
        let content = r#"rule "legacy" {
  on commit
  reject false
}
"#;
        let rule = parse_rule_file(content);
        assert!(rule.globs.is_empty());
        assert!(rule.always_apply);
        assert!(rule.rule_content.contains("legacy"));
    }

    // ── Test 4: Glob matching — **/*.rs matches src/auth.rs ─────────────

    #[test]
    fn test_glob_matches_rs_file() {
        let rule = GlobRule {
            globs: vec!["**/*.rs".to_string()],
            always_apply: false,
            rule_content: String::new(),
        };
        assert!(matches_path(&rule, "src/auth.rs"));
    }

    // ── Test 5: Glob matching — **/*.rs does NOT match src/auth.py ──────

    #[test]
    fn test_glob_does_not_match_py_file() {
        let rule = GlobRule {
            globs: vec!["**/*.rs".to_string()],
            always_apply: false,
            rule_content: String::new(),
        };
        assert!(!matches_path(&rule, "src/auth.py"));
    }

    // ── Test 6: alwaysApply rules load regardless of file context ───────

    #[test]
    fn test_always_apply_loads_without_context() {
        let rule = GlobRule {
            globs: vec!["**/*.rs".to_string()],
            always_apply: true,
            rule_content: String::new(),
        };
        // should_include returns true even with no file_path
        assert!(should_include(&rule, None));
        // and with a non-matching path
        assert!(should_include(&rule, Some("README.md")));
    }

    // ── Test 7: Multiple globs — any match triggers load ────────────────

    #[test]
    fn test_multiple_globs_any_match() {
        let rule = GlobRule {
            globs: vec!["**/*.rs".to_string(), "**/*.toml".to_string()],
            always_apply: false,
            rule_content: String::new(),
        };
        assert!(matches_path(&rule, "Cargo.toml"));
        assert!(matches_path(&rule, "src/main.rs"));
        assert!(!matches_path(&rule, "index.html"));
    }

    // ── Test 8: Empty globs array treated as always_apply ───────────────

    #[test]
    fn test_empty_globs_backward_compat() {
        let rule = GlobRule {
            globs: vec![],
            always_apply: false,
            rule_content: String::new(),
        };
        // No globs + always_apply=false -> backward compat: treated as always_apply
        assert!(should_include(&rule, None));
        assert!(should_include(&rule, Some("anything.rs")));
    }

    // ── P0 Security: Glob pattern with path traversal doesn't escape ────

    #[test]
    fn test_security_path_traversal_in_glob() {
        let rule = GlobRule {
            globs: vec!["../../etc/passwd".to_string(), "../**/*.rs".to_string()],
            always_apply: false,
            rule_content: String::new(),
        };
        // Path traversal patterns must be rejected
        assert!(!matches_path(&rule, "../../etc/passwd"));
        assert!(!matches_path(&rule, "../src/main.rs"));
        // Confirm it doesn't match anything
        assert!(!matches_path(&rule, "src/main.rs"));
    }

    // ── Test: load_rules_for_context with temp directory ─────────────────

    #[test]
    fn test_load_rules_for_context_filters_by_glob() {
        let tmp = tempfile::tempdir().unwrap();

        // Write a glob-scoped rule
        let scoped_content = r#"---
globs: ["**/*.rs"]
alwaysApply: false
---

rule "rust-only" {
  on commit
  reject false
}
"#;
        fs::write(tmp.path().join("rust.ftai"), scoped_content).unwrap();

        // Write an always-apply rule
        let always_content = r#"---
alwaysApply: true
---

rule "always" {
  on commit
  reject false
}
"#;
        fs::write(tmp.path().join("always.ftai"), always_content).unwrap();

        // With a .rs file: both should load
        let rules = load_rules_for_context(tmp.path(), Some("src/main.rs"));
        assert_eq!(rules.len(), 2);

        // With a .py file: only always-apply should load
        let rules = load_rules_for_context(tmp.path(), Some("src/main.py"));
        assert_eq!(rules.len(), 1);
        assert!(rules[0].always_apply);

        // With no file context: only always-apply should load
        let rules = load_rules_for_context(tmp.path(), None);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].always_apply);
    }

    // ── P0 Security: Symlink escape test ────────────────────────────────

    #[test]
    fn test_security_nonexistent_dir_returns_empty() {
        let rules = load_rules_for_context(Path::new("/nonexistent/path/xyz"), None);
        assert!(rules.is_empty());
    }

    // ── Integration with RulesEngine::load_glob_rules ───────────────────

    #[test]
    fn test_rules_engine_load_glob_rules() {
        use crate::rules::evaluator::RulesEngine;

        let tmp = tempfile::tempdir().unwrap();

        let content = r#"---
globs: ["**/*.rs"]
alwaysApply: false
---

rule "rust-guard" {
  on commit
  reject false
}
"#;
        fs::write(tmp.path().join("rust.ftai"), content).unwrap();

        let mut engine = RulesEngine::new();

        // Should load when matching file path
        let count = engine.load_glob_rules(tmp.path(), Some("src/main.rs")).unwrap();
        assert_eq!(count, 1);
        assert_eq!(engine.rule_count(), 1);

        // Clear and try with non-matching path
        engine.clear();
        let count = engine.load_glob_rules(tmp.path(), Some("index.html")).unwrap();
        assert_eq!(count, 0);
        assert_eq!(engine.rule_count(), 0);
    }
}
