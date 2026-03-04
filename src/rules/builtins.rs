use regex::Regex;

/// Built-in functions available in rules expressions

pub fn builtin_contains(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

pub fn builtin_matches(text: &str, pattern: &str) -> bool {
    Regex::new(pattern)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

pub fn builtin_extension(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

pub fn builtin_dirname(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .to_string_lossy()
        .to_string()
}

pub fn builtin_files_exist(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

pub fn builtin_files_match(pattern: &str, files: &[String]) -> bool {
    if let Ok(glob_pat) = glob::Pattern::new(pattern) {
        files.iter().any(|f| glob_pat.matches(f))
    } else {
        false
    }
}

pub fn builtin_line_count(path: &str) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

pub fn builtin_adds_lines_matching(pattern: &str, diff: &str) -> bool {
    if let Ok(re) = Regex::new(pattern) {
        diff.lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .any(|l| re.is_match(&l[1..]))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains() {
        assert!(builtin_contains("hello world", "world"));
        assert!(!builtin_contains("hello world", "xyz"));
    }

    #[test]
    fn test_matches() {
        assert!(builtin_matches("hello123", r"\d+"));
        assert!(!builtin_matches("hello", r"\d+"));
    }

    #[test]
    fn test_extension() {
        assert_eq!(builtin_extension("/foo/bar.rs"), "rs");
        assert_eq!(builtin_extension("/foo/bar"), "");
    }

    #[test]
    fn test_dirname() {
        assert_eq!(builtin_dirname("/foo/bar.rs"), "/foo");
    }

    #[test]
    fn test_files_match() {
        let files = vec!["test_red.rs".to_string(), "main.rs".to_string()];
        assert!(builtin_files_match("*red*", &files));
        assert!(!builtin_files_match("*.py", &files));
    }

    #[test]
    fn test_adds_lines_matching() {
        let diff = "+// TODO: fix this\n+fn hello() {}\n-old line\n";
        assert!(builtin_adds_lines_matching(r"^//\s*TODO", diff));
        assert!(!builtin_adds_lines_matching(r"^fn goodbye", diff));
    }
}
