use std::path::Path;
use crate::backend::types::ToolDefinition;
use crate::formatting::TemplateSet;

/// Build the system prompt from components
pub fn build_system_prompt(
    project_path: &Path,
    tool_defs: &[ToolDefinition],
    rules_summary: Option<&str>,
    memory_context: Option<&str>,
    templates: Option<&TemplateSet>,
    enabled_templates: &[String],
) -> String {
    let mut parts = Vec::new();

    // Core identity
    parts.push(format!(
        "You are FTAI, a FolkTech AI terminal coding assistant. \
         You help users with software engineering tasks by reading, writing, and editing code, \
         running commands, and managing git operations.\n\
         \n\
         Current project: {}\n",
        project_path.display()
    ));

    // Tool descriptions
    if !tool_defs.is_empty() {
        parts.push("# Available Tools\n".to_string());
        for tool in tool_defs {
            parts.push(format!(
                "## {}\n{}\nParameters: {}\n",
                tool.name,
                tool.description,
                serde_json::to_string_pretty(&tool.parameters).unwrap_or_default()
            ));
        }
    }

    // Active rules
    if let Some(rules) = rules_summary {
        parts.push(format!("# Active Rules\n{rules}\n"));
    }

    // Memory context
    if let Some(memory) = memory_context {
        parts.push(format!("# Memory\n{memory}\n"));
    }

    // Formatting guidelines
    if let Some(tmpl) = templates {
        let active = crate::formatting::enabled_templates(tmpl, enabled_templates);
        if !active.is_empty() {
            parts.push("# Formatting Guidelines\n".to_string());
            for (label, content) in active {
                parts.push(format!("## {label}\n{content}\n"));
            }
        }
    }

    // Project-level instructions
    let project_rules = project_path.join(".ftai").join("RULES.md");
    if project_rules.exists() {
        if let Ok(content) = std::fs::read_to_string(&project_rules) {
            parts.push(format!("# Project Rules\n{content}\n"));
        }
    }

    parts.join("\n")
}

/// Load memory context from ~/.ftai/memory/ and project memory
pub fn load_memory_context(project_path: &Path) -> Option<String> {
    let mut memory_parts = Vec::new();

    // Global memory
    if let Ok(global_dir) = crate::config::global_config_dir() {
        let global_memory = global_dir.join("memory").join("MEMORY.md");
        if global_memory.exists() {
            if let Ok(content) = std::fs::read_to_string(&global_memory) {
                memory_parts.push(content);
            }
        }
    }

    // Project memory
    if let Ok(project_dir) = crate::config::project_config_dir(project_path) {
        let project_memory = project_dir.join("memory").join("MEMORY.md");
        if project_memory.exists() {
            if let Ok(content) = std::fs::read_to_string(&project_memory) {
                memory_parts.push(content);
            }
        }
    }

    if memory_parts.is_empty() {
        None
    } else {
        Some(memory_parts.join("\n---\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_system_prompt_basic() {
        let path = PathBuf::from("/tmp/test-project");
        let prompt = build_system_prompt(&path, &[], None, None, None, &[]);
        assert!(prompt.contains("FTAI"));
        assert!(prompt.contains("/tmp/test-project"));
    }

    #[test]
    fn test_build_system_prompt_with_tools() {
        let path = PathBuf::from("/tmp/test-project");
        let tools = vec![ToolDefinition {
            name: "bash".to_string(),
            description: "Execute bash commands".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let prompt = build_system_prompt(&path, &tools, None, None, None, &[]);
        assert!(prompt.contains("bash"));
        assert!(prompt.contains("Execute bash commands"));
    }

    #[test]
    fn test_build_system_prompt_with_rules() {
        let path = PathBuf::from("/tmp/test-project");
        let prompt = build_system_prompt(&path, &[], Some("no force push allowed"), None, None, &[]);
        assert!(prompt.contains("Active Rules"));
        assert!(prompt.contains("no force push"));
    }

    #[test]
    fn test_build_system_prompt_with_memory() {
        let path = PathBuf::from("/tmp/test-project");
        let prompt = build_system_prompt(&path, &[], None, Some("User prefers Rust"), None, &[]);
        assert!(prompt.contains("Memory"));
        assert!(prompt.contains("User prefers Rust"));
    }

    #[test]
    fn test_build_system_prompt_with_templates() {
        let path = PathBuf::from("/tmp/test-project");
        let templates = TemplateSet::default();
        let prompt = build_system_prompt(&path, &[], None, None, Some(&templates), &[]);
        assert!(prompt.contains("Formatting Guidelines"));
        assert!(prompt.contains("Commit Messages"));
        assert!(prompt.contains("conventional commits"));
    }

    #[test]
    fn test_build_system_prompt_without_templates() {
        let path = PathBuf::from("/tmp/test-project");
        let prompt = build_system_prompt(&path, &[], None, None, None, &[]);
        assert!(!prompt.contains("Formatting Guidelines"));
    }

    #[test]
    fn test_build_system_prompt_partial_enabled() {
        let path = PathBuf::from("/tmp/test-project");
        let templates = TemplateSet::default();
        let enabled = vec!["commit".to_string()];
        let prompt = build_system_prompt(&path, &[], None, None, Some(&templates), &enabled);
        assert!(prompt.contains("Commit Messages"));
        assert!(!prompt.contains("Pull Requests"));
        assert!(!prompt.contains("Chat Responses"));
    }
}
