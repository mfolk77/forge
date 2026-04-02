/// Returns the default FTAI.md template for new projects.
pub fn default_ftai_md() -> &'static str {
    r#"# Project Configuration

## Build & Test
- [add your build command here]
- [add your test command here]

## Architecture
[Describe your project structure here]

## Git & PR Rules
- Use standard commit message formatting
- PR descriptions should explain what changed and why
- Include test coverage information in PRs
- No AI attribution in commits or PRs

## Code Quality
- Every code change must include tests
- Security tests are mandatory for code handling user input, file paths, or system commands
- P0 security tests (input injection, path traversal, auth bypass) are never skipped
- Errors from tools are results, not panics — convert Result::Err to string at boundaries

## Gotchas
[Document known issues and workarounds here]
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_ftai_md_is_not_empty() {
        let content = default_ftai_md();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_default_ftai_md_contains_build_section() {
        let content = default_ftai_md();
        assert!(content.contains("## Build & Test"));
    }

    #[test]
    fn test_default_ftai_md_contains_architecture_section() {
        let content = default_ftai_md();
        assert!(content.contains("## Architecture"));
    }

    #[test]
    fn test_default_ftai_md_contains_git_section() {
        let content = default_ftai_md();
        assert!(content.contains("## Git & PR Rules"));
    }

    #[test]
    fn test_default_ftai_md_contains_quality_section() {
        let content = default_ftai_md();
        assert!(content.contains("## Code Quality"));
    }
}
