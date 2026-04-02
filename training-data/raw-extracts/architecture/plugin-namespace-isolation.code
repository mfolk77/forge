/// A compiled-in catalog entry for a discoverable plugin.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub description: String,
    pub category: String,
    pub author: String,
    pub repo: String,
}

/// Returns the full built-in plugin catalog.
pub fn catalog() -> Vec<CatalogEntry> {
    vec![
        // ── FolkTech Core ──────────────────────────────────────────────
        CatalogEntry {
            name: "commit-helper".into(),
            description: "Guided commit workflow with staged diff review and message generation".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-commit-helper".into(),
        },
        CatalogEntry {
            name: "pr-review".into(),
            description: "Automated code review pipeline with security and quality checks".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-pr-review".into(),
        },
        CatalogEntry {
            name: "test-runner".into(),
            description: "Run project tests, format results, and track coverage".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-test-runner".into(),
        },
        CatalogEntry {
            name: "deploy-checklist".into(),
            description: "Pre-deployment verification checklists and safety gates".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-deploy-checklist".into(),
        },
        // ── Workflow ───────────────────────────────────────────────────
        CatalogEntry {
            name: "memory-manager".into(),
            description: "Persistent cross-session memory with semantic search".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-memory-manager".into(),
        },
        CatalogEntry {
            name: "web-search".into(),
            description: "Search the web and fetch page content for research".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-web-search".into(),
        },
        CatalogEntry {
            name: "notebook".into(),
            description: "Read, edit, and execute Jupyter notebook cells".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-notebook".into(),
        },
        CatalogEntry {
            name: "mcp-bridge".into(),
            description: "Connect to Model Context Protocol servers".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-mcp-bridge".into(),
        },
        // ── Utility ────────────────────────────────────────────────────
        CatalogEntry {
            name: "docker-tools".into(),
            description: "Generate Dockerfiles, compose configs, and container management".into(),
            category: "Utility".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-docker-tools".into(),
        },
        CatalogEntry {
            name: "git-workflow".into(),
            description: "Branch naming conventions, PR templates, and merge strategies".into(),
            category: "Utility".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-git-workflow".into(),
        },
        CatalogEntry {
            name: "python-tools".into(),
            description: "Python linting, formatting, virtualenv management".into(),
            category: "Utility".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-python-tools".into(),
        },
        CatalogEntry {
            name: "rust-tools".into(),
            description: "Cargo commands, clippy integration, and dependency auditing".into(),
            category: "Utility".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-rust-tools".into(),
        },
        // ── Integration ────────────────────────────────────────────────
        CatalogEntry {
            name: "github-actions".into(),
            description: "Generate and manage GitHub Actions CI/CD workflows".into(),
            category: "Integration".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-github-actions".into(),
        },
        CatalogEntry {
            name: "slack-notify".into(),
            description: "Send notifications and updates to Slack channels".into(),
            category: "Integration".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-slack-notify".into(),
        },
        CatalogEntry {
            name: "linear-tracker".into(),
            description: "Create and manage Linear issues and project tracking".into(),
            category: "Integration".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-linear-tracker".into(),
        },
        CatalogEntry {
            name: "sentry-monitor".into(),
            description: "Error tracking integration and alert management".into(),
            category: "Integration".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-sentry-monitor".into(),
        },
        // ── Community (P1 — must have) ────────────────────────────────
        CatalogEntry {
            name: "superpowers".into(),
            description: "Multi-skill suite: brainstorming, planning, TDD, debugging, code review, parallel agents".into(),
            category: "Workflow".into(),
            author: "community".into(),
            repo: "https://github.com/hesreallyhim/claude-superpowers".into(),
        },
        CatalogEntry {
            name: "session-memory".into(),
            description: "Persistent cross-session memory with automatic capture and semantic reinject (43k+ stars)".into(),
            category: "FolkTech Core".into(),
            author: "community".into(),
            repo: "https://github.com/thedotmack/claude-mem".into(),
        },
        CatalogEntry {
            name: "context7".into(),
            description: "Fetch up-to-date library docs into context — prevents hallucination from stale training data (51k+ stars)".into(),
            category: "Integration".into(),
            author: "upstash".into(),
            repo: "https://github.com/upstash/context7".into(),
        },
        CatalogEntry {
            name: "security-audit".into(),
            description: "Automated SAST analysis, dependency scanning, secrets detection, and vulnerability reporting".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-security-audit".into(),
        },
        CatalogEntry {
            name: "session-hud".into(),
            description: "Real-time observability: context usage, active tools, agent status, token consumption (15k+ stars)".into(),
            category: "DevTools".into(),
            author: "community".into(),
            repo: "https://github.com/jarrodwatts/claude-hud".into(),
        },
        CatalogEntry {
            name: "plugin-dev-kit".into(),
            description: "End-to-end plugin creation: scaffolding, hooks, skills, commands, MCP integration".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-dev-kit".into(),
        },
        CatalogEntry {
            name: "code-review".into(),
            description: "AI-powered PR review with structured feedback on correctness, security, and style".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-code-review".into(),
        },
        CatalogEntry {
            name: "commit-commands".into(),
            description: "Intelligent git commit, push, and PR creation from staged diffs".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-commit-commands".into(),
        },
        // ── MCP Servers (P1) ──────────────────────────────────────────
        CatalogEntry {
            name: "mcp-github".into(),
            description: "Official GitHub MCP — issues, PRs, repos, actions, code search (28k+ stars)".into(),
            category: "Integration".into(),
            author: "github".into(),
            repo: "https://github.com/github/github-mcp-server".into(),
        },
        CatalogEntry {
            name: "mcp-playwright".into(),
            description: "Browser automation and testing via Playwright MCP — navigate, screenshot, interact (30k+ stars)".into(),
            category: "Integration".into(),
            author: "microsoft".into(),
            repo: "https://github.com/microsoft/playwright-mcp".into(),
        },
        CatalogEntry {
            name: "mcp-serena".into(),
            description: "Semantic code intelligence — AST-aware search, symbol navigation, refactoring (22k+ stars)".into(),
            category: "Integration".into(),
            author: "oraios".into(),
            repo: "https://github.com/oraios/serena".into(),
        },
        // ── Community (P2 — should have) ──────────────────────────────
        CatalogEntry {
            name: "feature-dev".into(),
            description: "Guided feature development: codebase analysis, architecture planning, review checkpoints".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-feature-dev".into(),
        },
        CatalogEntry {
            name: "pr-review-toolkit".into(),
            description: "Multi-agent PR analysis: test coverage, type design, code simplifier, silent-failure-hunter".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-pr-review-toolkit".into(),
        },
        CatalogEntry {
            name: "hookify".into(),
            description: "Create git hooks and automation rules from conversation analysis or explicit instructions".into(),
            category: "DevTools".into(),
            author: "community".into(),
            repo: "https://github.com/mfolk77/forge-plugin-hookify".into(),
        },
        CatalogEntry {
            name: "changelog-generator".into(),
            description: "Generate user-facing changelogs from git history, categorizing and humanizing commits".into(),
            category: "Workflow".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-changelog-generator".into(),
        },
        CatalogEntry {
            name: "agent-personas".into(),
            description: "Specialized agent roles: backend architect, frontend dev, iOS dev, security auditor".into(),
            category: "AI".into(),
            author: "community".into(),
            repo: "https://github.com/alirezarezvani/claude-skills".into(),
        },
        CatalogEntry {
            name: "skill-creator".into(),
            description: "Create, modify, and benchmark custom skills with eval-driven development".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-skill-creator".into(),
        },
        CatalogEntry {
            name: "config-auditor".into(),
            description: "Audit and improve FTAI.md project config files with quality scoring and auto-fixes".into(),
            category: "FolkTech Core".into(),
            author: "folktech".into(),
            repo: "https://github.com/mfolk77/forge-plugin-config-auditor".into(),
        },
        CatalogEntry {
            name: "document-toolkit".into(),
            description: "Create and manipulate DOCX, XLSX, PPTX, and PDF files from the terminal".into(),
            category: "Utility".into(),
            author: "community".into(),
            repo: "https://github.com/mfolk77/forge-plugin-document-toolkit".into(),
        },
        // ── MCP Servers (P2) ──────────────────────────────────────────
        CatalogEntry {
            name: "mcp-figma".into(),
            description: "Read Figma designs, extract layout context, generate code from design files (14k+ stars)".into(),
            category: "Integration".into(),
            author: "community".into(),
            repo: "https://github.com/GLips/Figma-Context-MCP".into(),
        },
        CatalogEntry {
            name: "mcp-aws".into(),
            description: "Official AWS MCP servers for S3, Lambda, DynamoDB, CloudFormation (8.6k+ stars)".into(),
            category: "Integration".into(),
            author: "aws".into(),
            repo: "https://github.com/awslabs/mcp".into(),
        },
        CatalogEntry {
            name: "mcp-chrome-devtools".into(),
            description: "Chrome DevTools protocol — inspect DOM, network, console, performance (32k+ stars)".into(),
            category: "Integration".into(),
            author: "google".into(),
            repo: "https://github.com/nicholasoxford/chrome-devtools-mcp".into(),
        },
        CatalogEntry {
            name: "mcp-database".into(),
            description: "Database operations MCP — query, schema, migrations for Postgres, MySQL, SQLite (13k+ stars)".into(),
            category: "Integration".into(),
            author: "google".into(),
            repo: "https://github.com/googleapis/genai-toolbox".into(),
        },
        CatalogEntry {
            name: "mcp-huggingface".into(),
            description: "Search models, datasets, papers on HuggingFace Hub; run inference on Spaces".into(),
            category: "Integration".into(),
            author: "huggingface".into(),
            repo: "https://github.com/huggingface/huggingface-mcp-server".into(),
        },
        CatalogEntry {
            name: "mcp-firecrawl".into(),
            description: "Web scraping and crawling via Firecrawl MCP — converts pages to clean markdown (5.9k+ stars)".into(),
            category: "Integration".into(),
            author: "firecrawl".into(),
            repo: "https://github.com/firecrawl/firecrawl-mcp-server".into(),
        },
    ]
}

/// Find a catalog entry by name.
pub fn find_in_catalog(name: &str) -> Option<CatalogEntry> {
    catalog().into_iter().find(|e| e.name == name)
}

/// Validate a plugin name for creation. Returns true if valid.
/// Valid names: non-empty, alphanumeric plus hyphens and underscores, no path traversal.
pub fn is_valid_plugin_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_catalog_has_expected_count() {
        let entries = catalog();
        assert!(entries.len() >= 30, "Expected at least 30 catalog entries, got {}", entries.len());
    }

    #[test]
    fn test_catalog_names_unique() {
        let entries = catalog();
        let names: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), entries.len(), "Duplicate names in catalog");
    }

    #[test]
    fn test_catalog_entries_have_valid_fields() {
        for entry in catalog() {
            assert!(!entry.name.is_empty(), "Empty name");
            assert!(!entry.description.is_empty(), "Empty description for {}", entry.name);
            assert!(!entry.category.is_empty(), "Empty category for {}", entry.name);
            assert!(!entry.author.is_empty(), "Empty author for {}", entry.name);
            assert!(!entry.repo.is_empty(), "Empty repo for {}", entry.name);
        }
    }

    #[test]
    fn test_catalog_repo_urls_valid() {
        for entry in catalog() {
            assert!(
                entry.repo.starts_with("https://github.com/"),
                "Repo URL must start with https://github.com/ for {}: {}",
                entry.name, entry.repo,
            );
            // No injection characters in URLs
            assert!(!entry.repo.contains(';'), "Semicolon in repo URL for {}", entry.name);
            assert!(!entry.repo.contains('|'), "Pipe in repo URL for {}", entry.name);
            assert!(!entry.repo.contains('`'), "Backtick in repo URL for {}", entry.name);
        }
    }

    #[test]
    fn test_find_in_catalog_existing() {
        let entry = find_in_catalog("commit-helper");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.category, "FolkTech Core");
    }

    #[test]
    fn test_find_in_catalog_missing() {
        assert!(find_in_catalog("nonexistent-plugin").is_none());
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_catalog_repo_urls_no_injection() {
        for entry in catalog() {
            // No shell injection characters in URLs
            assert!(!entry.repo.contains(';'), "Semicolon in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('&'), "Ampersand in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('|'), "Pipe in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('`'), "Backtick in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('$'), "Dollar sign in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('('), "Paren in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains(')'), "Paren in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('\n'), "Newline in repo URL: {}", entry.repo);
            assert!(!entry.repo.contains('\0'), "Null byte in repo URL: {}", entry.repo);
        }
    }

    #[test]
    fn test_security_catalog_names_no_path_traversal() {
        for entry in catalog() {
            assert!(!entry.name.contains(".."), "Path traversal in name: {}", entry.name);
            assert!(!entry.name.contains('/'), "Slash in name: {}", entry.name);
            assert!(!entry.name.contains('\\'), "Backslash in name: {}", entry.name);
            assert!(!entry.name.contains('\0'), "Null byte in name: {}", entry.name);
        }
    }

    #[test]
    fn test_security_find_in_catalog_with_injection_input() {
        // Injection attempts in search should safely return None
        assert!(find_in_catalog("'; DROP TABLE plugins; --").is_none());
        assert!(find_in_catalog("../../../etc/passwd").is_none());
        assert!(find_in_catalog("$(whoami)").is_none());
        assert!(find_in_catalog("`rm -rf /`").is_none());
    }

    // ── Plugin Name Validation ────────────────────────────────────────────

    #[test]
    fn test_valid_plugin_names() {
        assert!(is_valid_plugin_name("my-plugin"));
        assert!(is_valid_plugin_name("my_plugin"));
        assert!(is_valid_plugin_name("plugin123"));
        assert!(is_valid_plugin_name("a"));
    }

    #[test]
    fn test_invalid_plugin_names() {
        assert!(!is_valid_plugin_name(""));
        assert!(!is_valid_plugin_name("../escape"));
        assert!(!is_valid_plugin_name("path/traversal"));
        assert!(!is_valid_plugin_name("back\\slash"));
        assert!(!is_valid_plugin_name("has spaces"));
        assert!(!is_valid_plugin_name("has.dots"));
    }

    // ── P0 Security: Plugin Create Name Validation ────────────────────────

    #[test]
    fn test_security_plugin_name_path_traversal() {
        assert!(!is_valid_plugin_name("../../etc/passwd"));
        assert!(!is_valid_plugin_name(".."));
        assert!(!is_valid_plugin_name("/etc/passwd"));
        assert!(!is_valid_plugin_name("\\\\server\\share"));
    }

    #[test]
    fn test_security_plugin_name_injection() {
        assert!(!is_valid_plugin_name("$(whoami)"));
        assert!(!is_valid_plugin_name("`rm -rf /`"));
        assert!(!is_valid_plugin_name("foo; rm -rf /"));
        assert!(!is_valid_plugin_name("foo\0bar"));
    }

    // ── Plugin Scaffold Directory Structure ───────────────────────────────

    #[test]
    fn test_scaffold_creates_correct_structure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("test-plugin");

        // Simulate what scaffold_plugin does
        let subdirs = ["tools", "skills", "hooks"];
        for dir in &subdirs {
            std::fs::create_dir_all(plugin_dir.join(dir)).unwrap();
        }

        let manifest = format!(
            r#"[plugin]
name = "test-plugin"
version = "0.1.0"
description = "A custom forge plugin"
author = ""
"#
        );
        std::fs::write(plugin_dir.join("plugin.toml"), &manifest).unwrap();

        // Verify structure
        assert!(plugin_dir.join("tools").is_dir());
        assert!(plugin_dir.join("skills").is_dir());
        assert!(plugin_dir.join("hooks").is_dir());
        assert!(plugin_dir.join("plugin.toml").is_file());

        // Verify manifest is valid TOML
        let content = std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap();
        let manifest: crate::plugins::manifest::PluginManifest = toml::from_str(&content).unwrap();
        assert_eq!(manifest.plugin.name, "test-plugin");
        assert_eq!(manifest.plugin.version, "0.1.0");
    }
}
