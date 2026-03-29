use super::LoadedSkill;

/// Returns all built-in skills shipped with the binary.
/// Content is embedded at compile time via `include_str!`.
pub fn builtin_skills() -> Vec<LoadedSkill> {
    vec![
        LoadedSkill {
            name: "commit".to_string(),
            description: "Git commit best practices".to_string(),
            trigger: "/commit".to_string(),
            content: include_str!("../../skills/commit.md").to_string(),
            source: "builtin".to_string(),
        },
        LoadedSkill {
            name: "review".to_string(),
            description: "Code review checklist".to_string(),
            trigger: "/review".to_string(),
            content: include_str!("../../skills/review.md").to_string(),
            source: "builtin".to_string(),
        },
        LoadedSkill {
            name: "refactor".to_string(),
            description: "Refactoring guide".to_string(),
            trigger: "/refactor".to_string(),
            content: include_str!("../../skills/refactor.md").to_string(),
            source: "builtin".to_string(),
        },
        LoadedSkill {
            name: "debug".to_string(),
            description: "Systematic debugging".to_string(),
            trigger: "/debug".to_string(),
            content: include_str!("../../skills/debug.md").to_string(),
            source: "builtin".to_string(),
        },
        LoadedSkill {
            name: "test".to_string(),
            description: "Test writing guide".to_string(),
            trigger: "/test".to_string(),
            content: include_str!("../../skills/test.md").to_string(),
            source: "builtin".to_string(),
        },
        LoadedSkill {
            name: "security".to_string(),
            description: "Security audit checklist".to_string(),
            trigger: "/security".to_string(),
            content: include_str!("../../skills/security.md").to_string(),
            source: "builtin".to_string(),
        },
    ]
}

/// Validates that a skill name contains only safe characters (alphanumeric, hyphen, underscore).
pub fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_builtin_skills_load() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 6);

        for skill in &skills {
            assert!(!skill.name.is_empty(), "Skill name must not be empty");
            assert!(!skill.description.is_empty(), "Skill description must not be empty");
            assert!(skill.trigger.starts_with('/'), "Trigger must start with /");
            assert!(!skill.content.is_empty(), "Skill content must not be empty: {}", skill.name);
            assert!(skill.content.len() > 100, "Skill content too short for {}: {} bytes", skill.name, skill.content.len());
            assert_eq!(skill.source, "builtin");
        }
    }

    #[test]
    fn test_builtin_skill_triggers_are_unique() {
        let skills = builtin_skills();
        let mut triggers: Vec<&str> = skills.iter().map(|s| s.trigger.as_str()).collect();
        triggers.sort();
        triggers.dedup();
        assert_eq!(triggers.len(), skills.len(), "Duplicate triggers found");
    }

    #[test]
    fn test_builtin_skill_names_are_valid() {
        let skills = builtin_skills();
        for skill in &skills {
            assert!(
                is_valid_skill_name(&skill.name),
                "Invalid skill name: {}",
                skill.name
            );
        }
    }

    #[test]
    fn test_valid_skill_names() {
        assert!(is_valid_skill_name("commit"));
        assert!(is_valid_skill_name("code-review"));
        assert!(is_valid_skill_name("test_utils"));
        assert!(is_valid_skill_name("debug123"));
    }

    #[test]
    fn test_invalid_skill_names() {
        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("../escape"));
        assert!(!is_valid_skill_name("/absolute"));
        assert!(!is_valid_skill_name("has space"));
        assert!(!is_valid_skill_name("semi;colon"));
        assert!(!is_valid_skill_name("pipe|char"));
        assert!(!is_valid_skill_name("back`tick"));
        assert!(!is_valid_skill_name("dollar$sign"));
    }

    // --- P0 Security Red Tests ---

    #[test]
    fn test_skill_content_no_path_traversal() {
        let skills = builtin_skills();
        for skill in &skills {
            // Skill content should not contain path traversal sequences that could
            // be interpreted as file paths (outside of documentation examples)
            // We check that the skill name/trigger don't contain traversal
            assert!(
                !skill.name.contains(".."),
                "Skill name must not contain path traversal: {}",
                skill.name
            );
            assert!(
                !skill.trigger.contains(".."),
                "Skill trigger must not contain path traversal: {}",
                skill.trigger
            );
        }
    }

    #[test]
    fn test_skill_names_alphanumeric_hyphen_only() {
        let skills = builtin_skills();
        for skill in &skills {
            assert!(
                is_valid_skill_name(&skill.name),
                "Skill name must be alphanumeric/hyphen/underscore only: {}",
                skill.name
            );
        }
    }

    #[test]
    fn test_skill_triggers_no_injection() {
        let skills = builtin_skills();
        for skill in &skills {
            // Triggers must be simple slash-commands, no shell metacharacters
            let trigger_name = &skill.trigger[1..]; // strip leading /
            assert!(
                is_valid_skill_name(trigger_name),
                "Skill trigger contains unsafe characters: {}",
                skill.trigger
            );
        }
    }

    #[test]
    fn test_malicious_skill_name_rejected() {
        assert!(!is_valid_skill_name("../../etc/passwd"));
        assert!(!is_valid_skill_name("$(whoami)"));
        assert!(!is_valid_skill_name("`id`"));
        assert!(!is_valid_skill_name("name\x00null"));
        assert!(!is_valid_skill_name("name\nnewline"));
    }

    // ── P0 Security Red Tests (additional) ─────────────────────────────────

    #[test]
    fn test_security_all_skill_content_valid_utf8() {
        // P0 security red test
        // include_str! guarantees UTF-8 at compile time, but we explicitly verify
        // that all content is valid UTF-8 strings at runtime too
        let skills = builtin_skills();
        for skill in &skills {
            // .as_bytes() + from_utf8 is a runtime check
            assert!(
                std::str::from_utf8(skill.content.as_bytes()).is_ok(),
                "Skill '{}' content is not valid UTF-8",
                skill.name
            );
        }
    }

    #[test]
    fn test_security_skill_content_no_tool_call_markers() {
        // P0 security red test
        // Skill content must not contain patterns that could be confused with
        // actual tool call invocations in the conversation stream
        let skills = builtin_skills();
        let dangerous_patterns = [
            "<tool_call>",
            "<function_call>",
            "</tool_call>",
            "</function_call>",
            "\"type\": \"function\"",  // OpenAI-style function call JSON
        ];
        for skill in &skills {
            for pattern in &dangerous_patterns {
                assert!(
                    !skill.content.contains(pattern),
                    "Skill '{}' contains dangerous tool call marker: {}",
                    skill.name,
                    pattern
                );
            }
        }
    }

    #[test]
    fn test_security_skill_content_size_limit() {
        // P0 security red test
        // No skill content should exceed 100KB to prevent context window stuffing
        let skills = builtin_skills();
        for skill in &skills {
            assert!(
                skill.content.len() <= 100 * 1024,
                "Skill '{}' exceeds 100KB: {} bytes",
                skill.name,
                skill.content.len()
            );
        }
    }
}
